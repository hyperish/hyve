//! Pagefile sections in the current logon session with a current-user-only DACL.
use super::{HEADER, LIMIT, MappedRing, Pod, invalid};
use crate::windows_security::{verify_owner_access, with_owner_access};
use std::{
    io,
    os::windows::io::{AsHandle, AsRawHandle, FromRawHandle, OwnedHandle},
    ptr::NonNull,
    sync::atomic::AtomicU64,
};
use windows_sys::Win32::{
    Foundation::{ERROR_ALREADY_EXISTS, GetLastError, INVALID_HANDLE_VALUE},
    System::Memory::{
        CreateFileMappingW, FILE_MAP_ALL_ACCESS, FILE_MAP_WRITE, MEMORY_MAPPED_VIEW_ADDRESS,
        MapViewOfFile, OpenFileMappingW, PAGE_READWRITE, UnmapViewOfFile,
    },
};

fn name(key: &[u8; 16]) -> Vec<u16> {
    format!(r"Local\hyve-ring-{:032x}", u128::from_be_bytes(*key))
        .encode_utf16()
        .chain(Some(0))
        .collect()
}

pub(super) struct Mapping {
    _handle: OwnedHandle,
    view: NonNull<AtomicU64>,
    length: usize,
    key: [u8; 16],
}
// SAFETY: the view is immutable in size, lives as long as Mapping and its
// contents are accessed exclusively through aligned atomics across all threads.
unsafe impl Send for Mapping {}
unsafe impl Sync for Mapping {}
impl Drop for Mapping {
    fn drop(&mut self) {
        // SAFETY: this is the base of our one live view, released exactly once.
        unsafe {
            UnmapViewOfFile(MEMORY_MAPPED_VIEW_ADDRESS {
                Value: self.view.as_ptr().cast(),
            });
        }
    }
}
impl Mapping {
    pub(super) fn allocate(length: usize) -> io::Result<Self> {
        let mut key = [0; 16];
        getrandom::fill(&mut key).map_err(io::Error::other)?;
        let name = name(&key);
        let handle = with_owner_access(|attrs| {
            // SAFETY: a pagefile section has stable storage. The name and
            // security descriptor remain valid while the kernel copies them.
            unsafe {
                let raw = CreateFileMappingW(
                    INVALID_HANDLE_VALUE,
                    attrs,
                    PAGE_READWRITE,
                    0,
                    length as u32,
                    name.as_ptr(),
                );
                if raw.is_null() {
                    return Err(io::Error::last_os_error());
                }
                let already_exists = GetLastError() == ERROR_ALREADY_EXISTS;
                let handle = OwnedHandle::from_raw_handle(raw);
                if already_exists {
                    return Err(io::ErrorKind::AlreadyExists.into());
                }
                Ok(handle)
            }
        })?;
        Self::map(handle, key, length)
    }
    fn map(handle: OwnedHandle, key: [u8; 16], length: usize) -> io::Result<Self> {
        // SAFETY: MapViewOfFile checks the exact requested length against the
        // kernel section. A successful view pins those pages until unmapping.
        let raw = unsafe { MapViewOfFile(handle.as_raw_handle(), FILE_MAP_WRITE, 0, 0, length) };
        let view = NonNull::new(raw.Value.cast()).ok_or_else(io::Error::last_os_error)?;
        Ok(Self {
            _handle: handle,
            view,
            length,
            key,
        })
    }
    pub(super) fn words(&self) -> &[AtomicU64] {
        // SAFETY: all mapping lengths are multiples of 8 and the OS maps
        // page-aligned storage. The borrow cannot outlive the mapped view.
        unsafe { std::slice::from_raw_parts(self.view.as_ptr(), self.length / 8) }
    }
}

impl<T: Pod> MappedRing<T> {
    /// A fixed-size token to send to the authorized peer over local IPC.
    /// The kernel independently checks the identity of each process attaching.
    pub fn share_token(&self) -> [u8; 24] {
        let mut token = [0; 24];
        token[..16].copy_from_slice(&self.mapping.key);
        token[16..].copy_from_slice(&(self.mapping.length as u64).to_le_bytes());
        token
    }
    /// Attaches the same user's section after checking its ownership, access
    /// list, mapped bounds and versioned sample layout.
    pub fn from_token(token: &[u8; 24], layout: u64) -> io::Result<Self> {
        let key = token[..16].try_into().expect("fixed key length");
        let length = u64::from_le_bytes(token[16..].try_into().expect("fixed size field"));
        let len = usize::try_from(length)
            .ok()
            .filter(|n| (HEADER * 8..=LIMIT).contains(n) && n % 8 == 0)
            .ok_or_else(|| invalid("invalid shared section length"))?;
        let name = name(&key);
        // SAFETY: the generated name is NUL terminated. A successful call
        // returns an owned non-inheritable handle on a kernel section.
        let handle = unsafe {
            let raw = OpenFileMappingW(FILE_MAP_ALL_ACCESS, 0, name.as_ptr());
            if raw.is_null() {
                return Err(io::Error::last_os_error());
            }
            OwnedHandle::from_raw_handle(raw)
        };
        verify_owner_access(handle.as_handle())?;
        let ring = Self::attach(Mapping::map(handle, key, len)?, layout)?;
        if super::length::<T>(ring.capacity)? != len {
            return Err(invalid("section token does not match the ring layout"));
        }
        Ok(ring)
    }
}
