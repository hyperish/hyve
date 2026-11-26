//! `sig()` call syntax on stable Rust.
//!
//! A call expression auto-dereferences its callee, so a type that derefs to
//! `dyn Fn() -> T` can be called like a function. A `Copy` handle has no
//! closure to point at, so we reinterpret the handle itself as a closure whose
//! only capture is a copy of that handle (the technique from dtolnay's
//! "callable types" case study, also used by Dioxus signals).

use std::mem::{MaybeUninit, align_of, align_of_val, size_of, size_of_val};

/// A `Copy` handle with a tracked read.
pub(crate) trait Readable: Copy + 'static {
    type Output;
    fn read_value(this: &Self) -> Self::Output;
}

#[allow(unsafe_code)]
pub(crate) fn callable<S: Readable>(this: &S) -> &(dyn Fn() -> S::Output + 'static) {
    let uninit = MaybeUninit::<S>::uninit();
    // SAFETY (closure body): the closure is only ever invoked through the
    // reference produced below, which points at a live, initialised `S`, so
    // the captured `uninit` field is that `S`.
    let closure = move || S::read_value(unsafe { &*uninit.as_ptr() });
    // The closure captures exactly one value of type `MaybeUninit<S>` (same
    // layout as `S`); check that rustc laid it out with nothing else.
    assert!(
        size_of_val(&closure) == size_of::<S>() && align_of_val(&closure) == align_of::<S>(),
        "hyve-state: unexpected closure layout for callable signal handles"
    );
    // SAFETY: the assertion above guarantees the closure type has the size and
    // alignment of `S`, so its single field (the capture) spans it from offset
    // 0 and a `&S` is a valid `&Closure` for the lifetime of `this`. The
    // closure has no drop glue (`MaybeUninit` never drops) and is only used
    // through a shared reference.
    unsafe { reinterpret(&closure, this) }
}

/// `this` viewed as the witness's (closure) type.
///
/// # Safety
///
/// `C` must have the size and alignment of `S` and be valid for any bit
/// pattern of a live `S`.
#[allow(unsafe_code)]
unsafe fn reinterpret<'a, S, C, O>(_witness: &C, this: &'a S) -> &'a (dyn Fn() -> O + 'static)
where
    C: Fn() -> O + 'static,
{
    // SAFETY: upheld by the caller.
    unsafe { &*(this as *const S).cast::<C>() }
}

#[cfg(test)]
mod tests {
    use crate::{local_signal, memo, signal};

    #[test]
    fn signals_and_memos_are_callable() {
        let a = signal(2);
        let b = memo(move || a() * 10);
        assert_eq!(a(), 2);
        assert_eq!(b(), 20);
        a.set(3);
        assert_eq!(b(), 30);
        let s = signal(String::from("x"));
        assert_eq!(s(), "x");
        let l = local_signal(std::rc::Rc::new(5));
        assert_eq!(*l(), 5);
    }
}
