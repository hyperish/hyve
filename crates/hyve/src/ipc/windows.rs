//! Same-user, non-inheritable local named pipes.
#![allow(unsafe_code)]
use hyve_core::windows_security::{current_user, process_user, with_owner_access};
use std::{
    io,
    os::windows::io::{AsHandle, AsRawHandle, FromRawHandle, OwnedHandle},
    path::{Path, PathBuf},
    ptr::from_mut,
    time::Duration,
};
use tokio::net::windows::named_pipe::{
    ClientOptions, NamedPipeClient, NamedPipeServer, ServerOptions,
};
use windows_sys::Win32::{
    Foundation::{ERROR_ALREADY_EXISTS, ERROR_PIPE_BUSY, GetLastError},
    System::{
        Pipes::GetNamedPipeServerProcessId,
        Threading::{CreateMutexW, OpenProcess, PROCESS_QUERY_LIMITED_INFORMATION},
    },
};

/// A connected local RPC client.
pub type ClientStream = NamedPipeClient;
/// The server end of a local pipe.
pub type ServerStream = NamedPipeServer;

fn validate_name(path: &Path) -> io::Result<()> {
    let name = path.to_str().and_then(|s| s.strip_prefix(r"\\.\pipe\"));
    if !name.is_some_and(|s| {
        !s.is_empty()
            && s.len() <= 200
            && s.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_.".contains(&b))
    }) {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            r"expected \\.\pipe\<local-app-name>",
        ));
    }
    Ok(())
}

fn create(path: &Path, first: bool) -> io::Result<NamedPipeServer> {
    with_owner_access(|attrs| {
        // SAFETY: the descriptor remains valid for this synchronous creation.
        unsafe {
            ServerOptions::new()
                .first_pipe_instance(first)
                .reject_remote_clients(true)
                .create_with_security_attributes_raw(path, from_mut(attrs).cast())
        }
    })
}
/// Connects to the local pipe and checks the server’s token before sending application data.
pub async fn connect(path: &Path) -> io::Result<ClientStream> {
    validate_name(path)?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(3);
    let pipe = loop {
        match ClientOptions::new().open(path) {
            Ok(pipe) => break pipe,
            Err(e)
                if e.raw_os_error() == Some(ERROR_PIPE_BUSY as i32)
                    && tokio::time::Instant::now() < deadline =>
            {
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
            Err(e) => return Err(e),
        }
    };
    // SAFETY: pipe remains connected while querying the server. A real
    // process handle pins the identity for the token comparison.
    let process = unsafe {
        let mut pid = 0;
        if GetNamedPipeServerProcessId(pipe.as_raw_handle(), &mut pid) == 0 {
            return Err(io::Error::last_os_error());
        }
        let h = OpenProcess(PROCESS_QUERY_LIMITED_INFORMATION, 0, pid);
        if h.is_null() {
            return Err(io::Error::last_os_error());
        }
        OwnedHandle::from_raw_handle(h)
    };
    if process_user(process.as_handle())? != current_user()? {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the pipe server belongs to another user",
        ));
    }
    Ok(pipe)
}
/// Reserve one listener while it hands connected pipe instances to clients.
fn reserve(path: &Path) -> io::Result<OwnedHandle> {
    let leaf = path
        .to_str()
        .and_then(|p| p.strip_prefix(r"\\.\pipe\"))
        .expect("validated pipe name");
    let name: Vec<u16> = format!(
        r"Local\hyve-pipe-{}-{}",
        current_user()?,
        leaf.to_ascii_lowercase()
    )
    .encode_utf16()
    .chain(Some(0))
    .collect();
    with_owner_access(|attrs| {
        // SAFETY: the name and descriptor stay live through CreateMutexW.
        // The object is a lifetime reservation, no thread owns its mutex.
        unsafe {
            let raw = CreateMutexW(attrs, 0, name.as_ptr());
            if raw.is_null() {
                return Err(io::Error::last_os_error());
            }
            let taken = GetLastError() == ERROR_ALREADY_EXISTS;
            let handle = OwnedHandle::from_raw_handle(raw);
            if taken {
                return Err(io::ErrorKind::AddrInUse.into());
            }
            Ok(handle)
        }
    })
}
/// A named pipe owned exclusively by this user.
pub struct Listener {
    path: PathBuf,
    next: NamedPipeServer,
    _reservation: OwnedHandle,
}
impl Listener {
    /// Reserves the first instance of the given pipe.
    pub fn bind(path: &Path) -> io::Result<Self> {
        validate_name(path)?;
        let reservation = reserve(path)?;
        Ok(Self {
            _reservation: reservation,
            path: path.into(),
            next: create(path, true)?,
        })
    }
    /// Waits for a client with access to the current user's pipe.
    pub async fn accept(&mut self) -> io::Result<ServerStream> {
        self.next.connect().await?;
        let next = create(&self.path, false)?;
        Ok(std::mem::replace(&mut self.next, next))
    }
}
