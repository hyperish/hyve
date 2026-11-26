//! Kernel storage and descriptor transport for Unix hosts.
use super::{HEADER, LIMIT, MappedRing, Pod, invalid};
use memmap2::MmapRaw;
use rustix::{
    fs::{Mode, fstat, ftruncate},
    net::{
        RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, SendAncillaryBuffer,
        SendAncillaryMessage, SendFlags, recvmsg, sendmsg,
    },
};
use std::{
    fs::File,
    io::{self, IoSlice, IoSliceMut},
    mem::MaybeUninit,
    os::{
        fd::{AsFd, OwnedFd},
        unix::net::UnixStream,
    },
    sync::atomic::AtomicU64,
};

#[cfg(any(target_os = "linux", target_os = "android"))]
fn allocate(length: usize) -> io::Result<File> {
    use rustix::fs::{MemfdFlags, SealFlags, fchmod, fcntl_add_seals, memfd_create};
    let fd = memfd_create("hyve-ring", MemfdFlags::CLOEXEC | MemfdFlags::ALLOW_SEALING)?;
    fchmod(&fd, Mode::RUSR | Mode::WUSR)?;
    ftruncate(&fd, length as u64)?;
    fcntl_add_seals(&fd, SealFlags::GROW | SealFlags::SHRINK | SealFlags::SEAL)?;
    Ok(fd.into())
}

#[cfg(any(target_os = "macos", target_os = "ios"))]
fn allocate(length: usize) -> io::Result<File> {
    use rustix::shm::{OFlags, open, unlink};
    // The name exists only between allocation and unlink. All subsequent
    // sharing uses a descriptor, including across an App Group socket.
    let name = format!("/hv{}", crate::Ulid::new());
    let fd = open(
        name.as_str(),
        OFlags::RDWR | OFlags::CREATE | OFlags::EXCL,
        Mode::RUSR | Mode::WUSR,
    )?;
    unlink(name.as_str())?;
    ftruncate(&fd, length as u64)?;
    Ok(fd.into())
}

fn fixed_size(file: &File) -> io::Result<usize> {
    let stat = fstat(file)?;
    if stat.st_uid != rustix::process::geteuid().as_raw() || stat.st_mode & 0o077 != 0 {
        return Err(invalid("shared ring must be private to the current user"));
    }
    #[cfg(any(target_os = "linux", target_os = "android"))]
    {
        use rustix::fs::{SealFlags, fcntl_get_seals};
        if !fcntl_get_seals(file)?.contains(SealFlags::GROW | SealFlags::SHRINK) {
            return Err(invalid("shared memory size is not sealed"));
        }
    }
    #[cfg(any(target_os = "macos", target_os = "ios"))]
    {
        use std::os::fd::AsRawFd;
        // XNU permits exactly one ftruncate on POSIX shared memory. Reject
        // other FD types before mapping: a regular file could be truncated.
        // PROC_PIDFDPSHMINFO (sys/proc_info.h) only succeeds on a pshm FD.
        const PROC_PIDFDPSHMINFO: i32 = 5;
        let mut info = [0u64; 512];
        // SAFETY: info is aligned and writable for the supplied byte count.
        // The kernel queries our own live descriptor and never retains the pointer.
        let n = unsafe {
            libc::proc_pidfdinfo(
                std::process::id() as i32,
                file.as_raw_fd(),
                PROC_PIDFDPSHMINFO,
                info.as_mut_ptr().cast(),
                size_of_val(&info) as i32,
            )
        };
        if n <= 0 {
            return Err(invalid(
                "descriptor is not a Darwin POSIX shared memory object",
            ));
        }
    }
    usize::try_from(stat.st_size)
        .ok()
        .filter(|n| (HEADER * 8..=LIMIT).contains(n))
        .ok_or_else(|| invalid("invalid kernel shared memory size"))
}

pub(super) struct Mapping {
    file: File,
    map: MmapRaw,
}
impl Mapping {
    pub(super) fn allocate(size: usize) -> io::Result<Self> {
        Self::new(allocate(size)?)
    }
    fn new(file: File) -> io::Result<Self> {
        let size = fixed_size(&file)?;
        let map = memmap2::MmapOptions::new().len(size).map_raw(&file)?;
        Ok(Self { file, map })
    }
    pub(super) fn words(&self) -> &[AtomicU64] {
        // SAFETY: mmap returns a page-aligned address; the FD has immutable
        // size for this mapping's lifetime. We only expose naturally aligned
        // AtomicU64 words, and all accesses by cooperating peers are atomic.
        // Local bounds use the verified map length, never mutable header data.
        unsafe { std::slice::from_raw_parts(self.map.as_ptr().cast(), self.map.len() / 8) }
    }
}

impl<T: Pod> MappedRing<T> {
    /// Passes a duplicate descriptor to an authorized local peer. Use a
    /// socketpair or a socket in the application's private/group container.
    pub fn send(&self, peer: &UnixStream) -> io::Result<()> {
        let fds = [self.mapping.file.as_fd()];
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
        let mut control = SendAncillaryBuffer::new(&mut space);
        if !control.push(SendAncillaryMessage::ScmRights(&fds)) {
            return Err(invalid("descriptor does not fit"));
        }
        if sendmsg(
            peer,
            &[IoSlice::new(b"R")],
            &mut control,
            SendFlags::empty(),
        )? != 1
        {
            return Err(io::ErrorKind::WriteZero.into());
        }
        Ok(())
    }
    /// Receives a ring from a local peer, checking its kernel object, bounds,
    /// format and the producer's layout ID before any sample is read.
    pub fn receive(peer: &UnixStream, layout: u64) -> io::Result<Self> {
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
        let mut control = RecvAncillaryBuffer::new(&mut space);
        let mut tag = [0];
        #[cfg(any(target_os = "linux", target_os = "android"))]
        let flags = RecvFlags::CMSG_CLOEXEC;
        #[cfg(any(target_os = "macos", target_os = "ios"))]
        let flags = RecvFlags::empty();
        let msg = recvmsg(peer, &mut [IoSliceMut::new(&mut tag)], &mut control, flags)?;
        if msg.bytes != 1 || tag != *b"R" || msg.flags.contains(rustix::net::ReturnFlags::CTRUNC) {
            return Err(invalid("invalid ring transfer"));
        }
        let mut fd: Option<OwnedFd> = None;
        for entry in control.drain() {
            if let RecvAncillaryMessage::ScmRights(fds) = entry {
                for next in fds {
                    if fd.replace(next).is_some() {
                        return Err(invalid("expected exactly one descriptor"));
                    }
                }
            }
        }
        let fd = fd.ok_or_else(|| invalid("missing ring descriptor"))?;
        rustix::io::fcntl_setfd(&fd, rustix::io::FdFlags::CLOEXEC)?;
        Self::attach(Mapping::new(fd.into())?, layout)
    }
}
