use std::{
    fs, io,
    os::unix::fs::{MetadataExt, PermissionsExt},
    path::{Path, PathBuf},
};
use tokio::net::{UnixListener, UnixStream};

/// A connected local RPC client.
pub type ClientStream = UnixStream;
/// The authenticated server end of a local connection.
pub type ServerStream = UnixStream;
fn private_parent(path: &Path) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|p| path.is_absolute() && !p.as_os_str().is_empty())
        .ok_or_else(|| {
            io::Error::new(
                io::ErrorKind::InvalidInput,
                "expected an absolute socket path",
            )
        })?;
    let meta = fs::symlink_metadata(parent)?;
    if !meta.is_dir()
        || meta.uid() != rustix::process::geteuid().as_raw()
        || meta.mode() & 0o077 != 0
    {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "expected a private directory owned by this user",
        ));
    }
    Ok(())
}
fn same_user(stream: UnixStream) -> io::Result<UnixStream> {
    if stream.peer_cred()?.uid() != rustix::process::geteuid().as_raw() {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the local peer belongs to another user",
        ));
    }
    Ok(stream)
}
/// Connects to an application socket and checks the kernel peer identity.
pub async fn connect(path: &Path) -> io::Result<ClientStream> {
    private_parent(path)?;
    same_user(UnixStream::connect(path).await?)
}
/// Bound local endpoint. Bind errors leave existing socket paths intact.
pub struct Listener {
    socket: UnixListener,
    path: PathBuf,
    identity: (u64, u64),
}
impl Listener {
    /// Binds a new socket inside a private directory supplied by the app.
    pub fn bind(path: &Path) -> io::Result<Self> {
        private_parent(path)?;
        let socket = UnixListener::bind(path)?;
        let meta = fs::symlink_metadata(path)?;
        let listener = Self {
            socket,
            path: path.into(),
            identity: (meta.dev(), meta.ino()),
        };
        fs::set_permissions(path, fs::Permissions::from_mode(0o600))?;
        Ok(listener)
    }
    /// Waits for a peer and verifies its effective user ID.
    pub async fn accept(&mut self) -> io::Result<ServerStream> {
        same_user(self.socket.accept().await?.0)
    }
}
impl Drop for Listener {
    fn drop(&mut self) {
        if fs::symlink_metadata(&self.path).is_ok_and(|m| (m.dev(), m.ino()) == self.identity) {
            let _ = fs::remove_file(&self.path);
        }
    }
}
