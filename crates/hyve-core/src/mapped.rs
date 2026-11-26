//! A fixed-size kernel mapping with a shared atomic header and POD payload.
#![allow(unsafe_code)]
use bytemuck::Pod;
use std::{
    io,
    marker::PhantomData,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
};
#[cfg(unix)]
mod unix;
#[cfg(unix)]
use unix::Mapping;
#[cfg(windows)]
mod windows;
#[cfg(windows)]
use windows::Mapping;

const MAGIC: u64 = u64::from_ne_bytes(*b"HYVERNG1");
const HEAD: usize = 5;
const REVISION: usize = 6;
const HEADER: usize = 7;
const LIMIT: usize = 1 << 30;

fn invalid(message: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, message)
}
fn stride<T: Pod>() -> usize {
    1 + size_of::<T>().div_ceil(4)
}
fn length<T: Pod>(capacity: usize) -> io::Result<usize> {
    capacity
        .checked_mul(stride::<T>())
        .and_then(|n| n.checked_add(HEADER))
        .and_then(|n| n.checked_mul(8))
        .filter(|&n| capacity > 0 && n <= LIMIT)
        .ok_or_else(|| invalid("ring needs nonzero capacity and at most 1 GiB"))
}

/// A nonblocking, overwrite-on-wrap shared ring. The kernel releases its
/// backing storage after the last mapping/descriptor disappears, including
/// on process termination. `layout` is the application's versioned POD ABI.
pub struct MappedRing<T: Pod> {
    mapping: Arc<Mapping>,
    capacity: usize,
    marker: PhantomData<fn() -> T>,
}
impl<T: Pod> Clone for MappedRing<T> {
    fn clone(&self) -> Self {
        Self {
            mapping: self.mapping.clone(),
            capacity: self.capacity,
            marker: PhantomData,
        }
    }
}

impl<T: Pod> crate::SharedRingBuf<T> {
    /// Creates a ring whose descriptor a local peer can attach.
    pub fn new_shared(capacity: usize, layout: u64) -> io::Result<MappedRing<T>> {
        MappedRing::new(capacity, layout)
    }
}

impl<T: Pod> MappedRing<T> {
    /// Allocates anonymous OS storage and describes its POD layout.
    pub fn new(capacity: usize, layout: u64) -> io::Result<Self> {
        let mapping = Mapping::allocate(length::<T>(capacity)?)?;
        let words = mapping.words();
        for (slot, value) in words[1..=4].iter().zip([
            layout,
            capacity as u64,
            size_of::<T>() as u64,
            align_of::<T>() as u64,
        ]) {
            slot.store(value, Ordering::Relaxed);
        }
        words[0].store(MAGIC, Ordering::Release);
        Ok(Self {
            mapping: Arc::new(mapping),
            capacity,
            marker: PhantomData,
        })
    }
    fn attach(mapping: Mapping, layout: u64) -> io::Result<Self> {
        let words = mapping.words();
        if words[0].load(Ordering::Acquire) != MAGIC
            || words[1].load(Ordering::Relaxed) != layout
            || words[3].load(Ordering::Relaxed) != size_of::<T>() as u64
            || words[4].load(Ordering::Relaxed) != align_of::<T>() as u64
        {
            return Err(invalid("shared ring version or POD layout mismatch"));
        }
        let capacity = usize::try_from(words[2].load(Ordering::Relaxed))
            .map_err(|_| invalid("invalid capacity"))?;
        if length::<T>(capacity)? > std::mem::size_of_val(words) {
            return Err(invalid("shared ring extends beyond the mapping"));
        }
        Ok(Self {
            mapping: Arc::new(mapping),
            capacity,
            marker: PhantomData,
        })
    }
    /// Number of retained samples.
    pub fn capacity(&self) -> usize {
        self.capacity
    }
    /// Number of claimed positions. Readers may skip an unpublished sample.
    pub fn total_pushed(&self) -> u64 {
        self.mapping.words()[HEAD].load(Ordering::Acquire)
    }
    /// Changes after publishing a sample, for display-frame subscribers.
    pub fn revision(&self) -> u64 {
        self.mapping.words()[REVISION].load(Ordering::Acquire)
    }
    fn slot(&self, position: u64) -> (&[AtomicU64], u64) {
        let width = stride::<T>();
        let offset = HEADER + (position % self.capacity as u64) as usize * width;
        (
            &self.mapping.words()[offset..offset + width],
            1 + position / self.capacity as u64,
        )
    }
    /// Publishes a POD without waiting on another process. Each 32-bit
    /// payload word carries its generation. A suspended writer cannot
    /// overwrite a word belonging to a newer generation.
    #[allow(deprecated)] // AtomicU64::try_update postdates the Rust 1.85 MSRV.
    pub fn push(&self, value: T) -> io::Result<()> {
        let words = self.mapping.words();
        let limit = u64::from(u32::MAX) * self.capacity as u64;
        let p = words[HEAD]
            .fetch_update(Ordering::AcqRel, Ordering::Acquire, |p| {
                (p < limit).then(|| p + 1)
            })
            .map_err(|_| io::Error::other("ring generations exhausted; allocate a new ring"))?;
        let (slot, generation) = self.slot(p);
        for (word, bytes) in slot[1..]
            .iter()
            .zip(crate::pod_bytes::bytes_of(&value).chunks(4))
        {
            let mut data = [0; 4];
            data[..bytes.len()].copy_from_slice(bytes);
            let packed = (generation << 32) | u64::from(u32::from_ne_bytes(data));
            if word
                .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |old| {
                    ((old >> 32) < generation).then_some(packed)
                })
                .is_err()
            {
                return Ok(());
            }
        }
        slot[0].fetch_max(p + 1, Ordering::Release);
        words[REVISION].fetch_add(1, Ordering::Release);
        Ok(())
    }
    fn read(&self, p: u64, dst: &mut T) -> bool {
        let (slot, generation) = self.slot(p);
        if slot[0].load(Ordering::Acquire) != p + 1 {
            return false;
        }
        for (word, target) in slot[1..]
            .iter()
            .zip(crate::pod_bytes::bytes_of_mut(dst).chunks_mut(4))
        {
            let packed = word.load(Ordering::Relaxed);
            if packed >> 32 != generation {
                return false;
            }
            target.copy_from_slice(&(packed as u32).to_ne_bytes()[..target.len()]);
        }
        true
    }
    /// Copies the latest contiguous run of intact, published samples in time order.
    pub fn read_latest_snapshot(&self, out: &mut [T]) -> usize {
        let head = self
            .total_pushed()
            .min(u64::from(u32::MAX) * self.capacity as u64);
        let want = self.capacity.min(out.len());
        let mut count = 0;
        for p in (head.saturating_sub(want as u64)..head).rev() {
            if self.read(p, &mut out[want - 1 - count]) {
                count += 1;
            } else if count > 0 {
                break;
            }
        }
        out.copy_within(want - count..want, 0);
        count
    }
}
