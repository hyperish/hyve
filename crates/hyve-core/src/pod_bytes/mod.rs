//! Byte-view branches of bytemuck 1.25.2's try_cast_slice[_mut] and bytes_of[_mut],
//! src/internal.rs at f363643e951a7ac9e4b9921de982f2b0918902e3. See LICENSE-MIT.
//! Our ring slots hold one Pod and always cast to u8, whose alignment is one.
//! Its size divides every input length, including zero.
use bytemuck::Pod;
use std::{mem::size_of, slice};

#[inline]
pub(super) fn bytes_of<T: Pod>(value: &T) -> &[u8] {
    // SAFETY: Pod excludes padding and uninitialized bytes. The shared slice
    // has the original reference's lifetime, extent and provenance.
    #[allow(unsafe_code)]
    unsafe {
        slice::from_raw_parts(std::ptr::from_ref(value).cast(), size_of::<T>())
    }
}

#[inline]
pub(super) fn bytes_of_mut<T: Pod>(value: &mut T) -> &mut [u8] {
    // SAFETY: the original exclusive borrow covers the slice. Every bit
    // pattern written through it represents a valid T.
    #[allow(unsafe_code)]
    unsafe {
        slice::from_raw_parts_mut(std::ptr::from_mut(value).cast(), size_of::<T>())
    }
}
