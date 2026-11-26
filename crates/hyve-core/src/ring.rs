//! `SharedRingBuf<T>`: an overwriting ring of POD samples with tear-free reads.
//!
//! Writers never block and never wait for readers: each `push` claims the next
//! position and overwrites the oldest slot. Readers copy the most recent samples
//! with [`SharedRingBuf::read_latest_snapshot`]; a sample being overwritten while
//! it is copied is detected (per-slot sequence lock) and never returned torn.
//!
//! The payload is stored as atomic 64-bit words, so concurrent reads and writes
//! are ordinary atomic operations and this module needs no `unsafe`. `T` must be
//! [`bytemuck::Pod`] (`#[repr(C)]`, no padding, no pointers), which is also the
//! contract the Phase 2 OS shared-memory mode will require
//! (`docs/02-architecture.md` §2.1).
//!
//! Design references: `rtrb`, `crossbeam-queue` (seqlock-per-slot idea, not code).

use std::fmt;
use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering, fence};

use bytemuck::Pod;

/// Shared handle to an overwriting ring of `T` samples. Clones share the ring.
pub struct SharedRingBuf<T: Pod> {
    ring: Arc<Ring>,
    _marker: PhantomData<fn() -> T>,
}

struct Ring {
    /// Number of positions claimed so far (the next position to write).
    head: AtomicU64,
    capacity: usize,
    stride: usize,
    slots: Box<[AtomicU64]>,
}

struct Slot<'a> {
    /// Even: stable; odd: being written. Incremented twice per write.
    seq: &'a AtomicU64,
    /// Ring position (+1) currently stored; 0 = never written.
    pos: &'a AtomicU64,
    words: &'a [AtomicU64],
}

impl Ring {
    fn slot(&self, position: u64) -> Slot<'_> {
        let start = (position % self.capacity as u64) as usize * self.stride;
        let fields = &self.slots[start..start + self.stride];
        Slot {
            seq: &fields[0],
            pos: &fields[1],
            words: &fields[2..],
        }
    }
}

const MAX_READ_ATTEMPTS: usize = 4;

impl<T: Pod> SharedRingBuf<T> {
    /// A ring holding the latest `capacity` samples.
    ///
    /// # Panics
    ///
    /// Panics if `capacity` is zero or the ring size overflows.
    pub fn new(capacity: usize) -> Self {
        assert!(capacity > 0, "SharedRingBuf capacity must be non-zero");
        let stride = 2 + std::mem::size_of::<T>().div_ceil(8);
        let length = capacity.checked_mul(stride).expect("ring size overflow");
        let slots = (0..length).map(|_| AtomicU64::new(0)).collect();
        SharedRingBuf {
            ring: Arc::new(Ring {
                head: AtomicU64::new(0),
                capacity,
                stride,
                slots,
            }),
            _marker: PhantomData,
        }
    }

    /// Number of samples the ring retains.
    pub fn capacity(&self) -> usize {
        self.ring.capacity
    }

    /// Total number of samples pushed so far.
    pub fn total_pushed(&self) -> u64 {
        self.ring.head.load(Ordering::Acquire)
    }

    /// Appends a sample, overwriting the oldest one when full. Never blocks on
    /// readers; concurrent writers to the same slot wait for each other only
    /// when the ring has wrapped completely during one write.
    pub fn push(&self, value: T) {
        let ring = &*self.ring;
        let pos = ring.head.fetch_add(1, Ordering::AcqRel);
        let slot = ring.slot(pos);

        // Acquire the slot's write lock (odd sequence).
        let mut seq = slot.seq.load(Ordering::Relaxed);
        loop {
            if seq & 1 == 1 {
                std::hint::spin_loop();
                seq = slot.seq.load(Ordering::Relaxed);
                continue;
            }
            match slot
                .seq
                .compare_exchange_weak(seq, seq + 1, Ordering::Acquire, Ordering::Relaxed)
            {
                Ok(_) => break,
                Err(actual) => seq = actual,
            }
        }
        // A newer position may already have claimed and written this slot
        // (writers overtook each other); never replace newer data with older.
        if slot.pos.load(Ordering::Relaxed) <= pos + 1 {
            fence(Ordering::Release);
            let mut chunks = crate::pod_bytes::bytes_of(&value).chunks_exact(8);
            for (chunk, word) in chunks.by_ref().zip(slot.words) {
                word.store(
                    u64::from_ne_bytes(chunk.try_into().unwrap()),
                    Ordering::Relaxed,
                );
            }
            let tail = chunks.remainder();
            if !tail.is_empty() {
                let mut word = [0; 8];
                word[..tail.len()].copy_from_slice(tail);
                slot.words
                    .last()
                    .unwrap()
                    .store(u64::from_ne_bytes(word), Ordering::Relaxed);
            }
            slot.pos.store(pos + 1, Ordering::Relaxed);
        }
        slot.seq.store(seq + 2, Ordering::Release);
    }

    /// Copies the most recent samples into `out`, oldest first, and returns how
    /// many were written (at most `min(out.len(), capacity)`). Samples that are
    /// being overwritten during the copy are skipped from the old end, so every
    /// returned sample is intact and the returned run is contiguous in time.
    pub fn read_latest_snapshot(&self, out: &mut [T]) -> usize {
        let ring = &*self.ring;
        let cap = ring.capacity as u64;
        let want = (out.len() as u64).min(cap);
        if want == 0 {
            return 0;
        }
        for _ in 0..MAX_READ_ATTEMPTS {
            let head = ring.head.load(Ordering::Acquire);
            let start = head.saturating_sub(want);
            // Fill from the newest backwards into the tail of `out`.
            let mut filled = 0usize;
            let mut pos = head;
            while pos > start {
                pos -= 1;
                let dst = (want as usize) - 1 - filled;
                match self.read_slot(pos, &mut out[dst]) {
                    SlotRead::Ok => filled += 1,
                    SlotRead::NotYetWritten if filled == 0 => continue, // newest still in flight
                    _ => break,
                }
            }
            if filled > 0 || head == 0 {
                out.copy_within((want as usize - filled)..(want as usize), 0);
                return filled;
            }
        }
        0
    }

    fn read_slot(&self, pos: u64, dst: &mut T) -> SlotRead {
        let ring = &*self.ring;
        let slot = ring.slot(pos);
        let s1 = slot.seq.load(Ordering::Acquire);
        if s1 & 1 == 1 {
            return SlotRead::NotYetWritten;
        }
        let stored = slot.pos.load(Ordering::Relaxed);
        if stored != pos + 1 {
            return if stored < pos + 1 {
                SlotRead::NotYetWritten
            } else {
                SlotRead::Overwritten
            };
        }
        let mut chunks = crate::pod_bytes::bytes_of_mut(dst).chunks_exact_mut(8);
        for (chunk, word) in chunks.by_ref().zip(slot.words) {
            chunk.copy_from_slice(&word.load(Ordering::Relaxed).to_ne_bytes());
        }
        let tail = chunks.into_remainder();
        if !tail.is_empty() {
            let word = slot
                .words
                .last()
                .unwrap()
                .load(Ordering::Relaxed)
                .to_ne_bytes();
            tail.copy_from_slice(&word[..tail.len()]);
        }
        fence(Ordering::Acquire);
        let s2 = slot.seq.load(Ordering::Relaxed);
        if s1 == s2 {
            SlotRead::Ok
        } else {
            SlotRead::Overwritten
        }
    }
}

enum SlotRead {
    Ok,
    NotYetWritten,
    Overwritten,
}

impl<T: Pod> Clone for SharedRingBuf<T> {
    fn clone(&self) -> Self {
        SharedRingBuf {
            ring: Arc::clone(&self.ring),
            _marker: PhantomData,
        }
    }
}

impl<T: Pod> fmt::Debug for SharedRingBuf<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SharedRingBuf")
            .field("capacity", &self.capacity())
            .field("total_pushed", &self.total_pushed())
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use bytemuck::{Pod, Zeroable};

    #[test]
    fn partial_words_and_empty_samples_survive_wrap() {
        fn check<T: Pod + PartialEq + std::fmt::Debug>(first: T, second: T) {
            let ring = SharedRingBuf::new(2);
            ring.push(first);
            ring.push(second);
            ring.push(first);
            let mut out = [T::zeroed(); 2];
            assert_eq!(ring.read_latest_snapshot(&mut out), 2);
            assert_eq!(out, [second, first]);
        }
        check((), ());
        check([1; 1], [2u8; 1]);
        check([1; 7], [2u8; 7]);
        check([1; 8], [2u8; 8]);
        check([1; 9], [2u8; 9]);
        check([1; 17], [2u8; 17]);
    }

    #[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
    #[repr(C)]
    struct Sample {
        seq: u64,
        check: u64,
        x: f32,
        y: f32,
        z: u32,
        _pad: u32,
    }

    impl Sample {
        fn new(seq: u64) -> Self {
            Sample {
                seq,
                check: !seq,
                x: seq as f32,
                y: -(seq as f32),
                z: seq as u32,
                _pad: 0,
            }
        }
        fn intact(&self) -> bool {
            self.check == !self.seq && self.z == self.seq as u32
        }
    }

    #[test]
    fn empty_and_partial() {
        let ring = SharedRingBuf::<Sample>::new(8);
        let mut out = [Sample::zeroed(); 4];
        assert_eq!(ring.read_latest_snapshot(&mut out), 0);
        ring.push(Sample::new(1));
        ring.push(Sample::new(2));
        assert_eq!(ring.read_latest_snapshot(&mut out), 2);
        assert_eq!(out[0].seq, 1);
        assert_eq!(out[1].seq, 2);
    }

    #[test]
    fn keeps_latest_in_order_after_wrap() {
        let ring = SharedRingBuf::<Sample>::new(5);
        for i in 1..=12 {
            ring.push(Sample::new(i));
        }
        let mut out = [Sample::zeroed(); 16];
        let n = ring.read_latest_snapshot(&mut out);
        assert_eq!(n, 5);
        assert_eq!(
            out[..n].iter().map(|s| s.seq).collect::<Vec<_>>(),
            vec![8, 9, 10, 11, 12]
        );
        let mut small = [Sample::zeroed(); 2];
        assert_eq!(ring.read_latest_snapshot(&mut small), 2);
        assert_eq!((small[0].seq, small[1].seq), (11, 12));
    }

    #[test]
    fn odd_sized_payload() {
        #[derive(Clone, Copy, Debug, PartialEq, Pod, Zeroable)]
        #[repr(C)]
        struct Tiny([u8; 3]);
        let ring = SharedRingBuf::<Tiny>::new(3);
        ring.push(Tiny([1, 2, 3]));
        let mut out = [Tiny([0; 3]); 1];
        assert_eq!(ring.read_latest_snapshot(&mut out), 1);
        assert_eq!(out[0], Tiny([1, 2, 3]));
    }

    #[cfg(not(target_arch = "wasm32"))]
    #[test]
    fn concurrent_reads_are_never_torn() {
        use std::sync::atomic::AtomicBool;
        let ring = SharedRingBuf::<Sample>::new(4);
        let stop = Arc::new(AtomicBool::new(false));
        let writers: Vec<_> = (0..2)
            .map(|w| {
                let ring = ring.clone();
                let stop = stop.clone();
                std::thread::spawn(move || {
                    let mut i = w as u64 * 1_000_000_000;
                    while !stop.load(Ordering::Relaxed) {
                        ring.push(Sample::new(i));
                        i += 1;
                    }
                })
            })
            .collect();
        let mut out = [Sample::zeroed(); 4];
        let mut reads = 0;
        for _ in 0..200_000 {
            let n = ring.read_latest_snapshot(&mut out);
            for s in &out[..n] {
                assert!(s.intact(), "torn sample {s:?}");
            }
            reads += n;
        }
        stop.store(true, Ordering::Relaxed);
        for w in writers {
            w.join().unwrap();
        }
        assert!(reads > 0);
    }
}
