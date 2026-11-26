//! Same-user security descriptors for local Windows kernel objects.
#![allow(unsafe_code)]
use std::{
    ffi::c_void,
    io,
    os::windows::io::{AsRawHandle, BorrowedHandle, FromRawHandle, OwnedHandle},
    ptr::null_mut,
};
use windows_sys::Win32::{
    Foundation::{HANDLE, LocalFree},
    Security::{
        ACCESS_ALLOWED_ACE,
        Authorization::{
            ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW,
            GetSecurityInfo, SDDL_REVISION_1, SE_KERNEL_OBJECT,
        },
        DACL_SECURITY_INFORMATION, EqualSid, GetAce, GetTokenInformation,
        OWNER_SECURITY_INFORMATION, PSID, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER, TokenUser,
    },
    System::Threading::{GetCurrentProcess, OpenProcessToken},
};
/// Owns allocations returned by the Win32 security descriptor and SID APIs.
struct Local(*mut c_void);
impl Drop for Local {
    fn drop(&mut self) {
        // SAFETY: all instances wrap a successful LocalAlloc-family return.
        unsafe {
            LocalFree(self.0);
        }
    }
}
fn sid_text(sid: PSID) -> io::Result<String> {
    let mut text = null_mut();
    // SAFETY: sid is inside a live OS-supplied TOKEN_USER. The returned SID
    // string is NUL terminated and lives until its Local owner is dropped.
    unsafe {
        if ConvertSidToStringSidW(sid, &mut text) == 0 {
            return Err(io::Error::last_os_error());
        }
        let _allocation = Local(text.cast());
        let mut n = 0;
        while *text.add(n) != 0 {
            n += 1;
        }
        String::from_utf16(std::slice::from_raw_parts(text, n)).map_err(io::Error::other)
    }
}
fn process_user_raw(process: HANDLE) -> io::Result<String> {
    let mut handle = null_mut();
    // SAFETY: process is either the current-process pseudo-handle or an open
    // process handle. token takes ownership of the new successful return.
    let token = unsafe {
        if OpenProcessToken(process, TOKEN_QUERY, &mut handle) == 0 {
            return Err(io::Error::last_os_error());
        }
        OwnedHandle::from_raw_handle(handle)
    };
    let mut bytes = 0;
    // SAFETY: the first call obtains the required size. The pointer-sized
    // buffer is aligned for TOKEN_USER and stays live while its SID is read.
    unsafe {
        GetTokenInformation(token.as_raw_handle(), TokenUser, null_mut(), 0, &mut bytes);
        if (bytes as usize) < size_of::<TOKEN_USER>() {
            return Err(io::Error::last_os_error());
        }
        let mut data = vec![0usize; (bytes as usize).div_ceil(size_of::<usize>())];
        if GetTokenInformation(
            token.as_raw_handle(),
            TokenUser,
            data.as_mut_ptr().cast(),
            bytes,
            &mut bytes,
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        sid_text((*data.as_ptr().cast::<TOKEN_USER>()).User.Sid)
    }
}
/// SID of the calling user.
pub fn current_user() -> io::Result<String> {
    // SAFETY: this API returns a borrowed pseudo-handle and has no preconditions.
    process_user_raw(unsafe { GetCurrentProcess() })
}
/// SID of the user owning a live process.
pub fn process_user(process: BorrowedHandle<'_>) -> io::Result<String> {
    process_user_raw(process.as_raw_handle())
}
/// Builds a non-inheritable security descriptor granting only the current user access.
pub fn with_owner_access<R>(
    create: impl FnOnce(&mut SECURITY_ATTRIBUTES) -> io::Result<R>,
) -> io::Result<R> {
    let user = current_user()?;
    let sddl: Vec<u16> = format!("O:{user}D:P(A;;GA;;;{user})")
        .encode_utf16()
        .chain(Some(0))
        .collect();
    let mut descriptor = null_mut();
    // SAFETY: SDDL is NUL terminated, and the resulting descriptor stays
    // alive during CreateNamedPipe. The new OS handle is non-inheritable.
    unsafe {
        if ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl.as_ptr(),
            SDDL_REVISION_1,
            &mut descriptor,
            null_mut(),
        ) == 0
        {
            return Err(io::Error::last_os_error());
        }
        let descriptor = Local(descriptor);
        let mut attrs = SECURITY_ATTRIBUTES {
            nLength: size_of::<SECURITY_ATTRIBUTES>() as u32,
            lpSecurityDescriptor: descriptor.0,
            bInheritHandle: 0,
        };
        create(&mut attrs)
    }
}

/// Checks that the kernel object's owner and sole allow ACE both name the current user.
pub fn verify_owner_access(object: BorrowedHandle<'_>) -> io::Result<()> {
    let (mut owner, mut acl, mut descriptor) = (null_mut(), null_mut(), null_mut());
    // SAFETY: GetSecurityInfo returns owner and ACL pointers into one LocalFree
    // allocation. Read the allow ACE only after validating its type and length.
    unsafe {
        let rc = GetSecurityInfo(
            object.as_raw_handle(),
            SE_KERNEL_OBJECT,
            DACL_SECURITY_INFORMATION | OWNER_SECURITY_INFORMATION,
            &mut owner,
            null_mut(),
            &mut acl,
            null_mut(),
            &mut descriptor,
        );
        if rc != 0 {
            return Err(io::Error::from_raw_os_error(rc as i32));
        }
        let _descriptor = Local(descriptor);
        let denied = || {
            io::Error::new(
                io::ErrorKind::PermissionDenied,
                "shared object must be private to this user",
            )
        };
        if owner.is_null()
            || acl.is_null()
            || sid_text(owner)? != current_user()?
            || (*acl).AceCount != 1
        {
            return Err(denied());
        }
        let mut ptr = null_mut();
        if GetAce(acl, 0, &mut ptr) == 0 || ptr.is_null() {
            return Err(denied());
        }
        let ace = ptr.cast::<ACCESS_ALLOWED_ACE>();
        // ACCESS_ALLOWED_ACE_TYPE is zero. SID starts at the last variable-sized field.
        if (*ace).Header.AceType != 0
            || usize::from((*ace).Header.AceSize) < size_of::<ACCESS_ALLOWED_ACE>()
            || EqualSid(owner, std::ptr::addr_of_mut!((*ace).SidStart).cast()) == 0
        {
            return Err(denied());
        }
        Ok(())
    }
}
