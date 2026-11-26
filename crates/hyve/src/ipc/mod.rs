//! Local, same-user byte streams for desktop adapters. Unix endpoints live in
//! an app-owned private directory. Windows endpoints are local named pipes.
#[cfg(unix)]
mod unix;
#[cfg(unix)]
pub use unix::{ClientStream, Listener, ServerStream, connect};
#[cfg(windows)]
mod windows;
#[cfg(windows)]
pub use windows::{ClientStream, Listener, ServerStream, connect};
