//! Scratch-buffer ownership adapted from rkyv 0.8.18's util/alloc/arena.rs at
//! 8db673416b5489f39793e654145ceb938d652c84. See LICENSE-MIT.
//! A nested decoder borrows its own buffer. One bounded buffer can survive per
//! thread; archives that arrive aligned take the original zero-copy path.
use rkyv::util::AlignedVec;
use std::cell::Cell;

pub(super) const ALIGN: usize = 16;
const RETAIN_LIMIT: usize = 64 * 1024;
thread_local! {
    static SCRATCH: Cell<Option<AlignedVec<ALIGN>>> = const { Cell::new(None) };
}

pub(super) fn with_aligned<R>(bytes: &[u8], f: impl FnOnce(&[u8]) -> R) -> R {
    if (bytes.as_ptr() as usize) % ALIGN == 0 {
        return f(bytes);
    }
    if bytes.len() > RETAIN_LIMIT {
        let mut copy = AlignedVec::<ALIGN>::with_capacity(bytes.len());
        copy.extend_from_slice(bytes);
        return f(&copy);
    }
    let mut copy = SCRATCH
        .try_with(Cell::take)
        .ok()
        .flatten()
        .unwrap_or_default();
    copy.extend_from_slice(bytes);
    let result = f(&copy);
    copy.fill(0);
    copy.clear();
    if copy.capacity() <= RETAIN_LIMIT {
        let _ = SCRATCH.try_with(|slot| {
            if let Some(nested) = slot.take() {
                if nested.capacity() > copy.capacity() {
                    copy = nested;
                }
            }
            slot.set(Some(copy));
        });
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn nested_unaligned_archives_use_disjoint_live_storage() {
        let mut data = AlignedVec::<ALIGN>::new();
        data.extend_from_slice(&[7; 128]);
        with_aligned(&data[1..], |outer| {
            with_aligned(&data[3..], |inner| {
                assert_ne!(outer.as_ptr(), inner.as_ptr());
                assert_eq!(inner, &data[3..]);
            });
            assert_eq!(outer, &data[1..]);
        });
        let stored = SCRATCH.take().unwrap();
        assert!(stored.is_empty() && stored.capacity() <= RETAIN_LIMIT);
        with_aligned(&data, |aligned| assert_eq!(aligned.as_ptr(), data.as_ptr()));
    }
}
