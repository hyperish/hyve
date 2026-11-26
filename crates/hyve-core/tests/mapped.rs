//! Descriptor transfer, process death and concurrent ring snapshots.
#![cfg(all(
    feature = "shm",
    target_has_atomic = "64",
    any(
        target_os = "macos",
        target_os = "ios",
        target_os = "linux",
        target_os = "android"
    )
))]
use hyve_core::{MappedRing, SharedRingBuf, Ulid};
use std::{
    fs,
    io::{Read, Write},
    os::unix::{
        fs::DirBuilderExt,
        net::{UnixListener, UnixStream},
    },
    path::PathBuf,
    process::{Child, Command},
    time::{Duration, Instant},
};

const LAYOUT: u64 = 0x1001;
const WAIT: Duration = Duration::from_secs(15);
type Sample = [u64; 8];

struct Fixture {
    child: Child,
    dir: PathBuf,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
        let _ = fs::remove_dir_all(&self.dir);
    }
}

#[test]
#[ignore = "started by independent_process_shares_and_survives_a_writer"]
fn shared_worker() {
    let mut socket =
        UnixStream::connect(std::env::var_os("HYVE_TEST_RING_SOCKET").unwrap()).unwrap();
    let ring = MappedRing::<Sample>::receive(&socket, LAYOUT).unwrap();
    assert_eq!(ring.capacity(), 32);
    let mut latest = [[0; 8]];
    assert_eq!(ring.read_latest_snapshot(&mut latest), 1);
    assert_eq!(latest, [[7; 8]]);
    ring.push([8; 8]).unwrap();
    socket.write_all(b"!").unwrap();
    let mut start = [0];
    socket.read_exact(&mut start).unwrap();
    assert_eq!(start, *b"+");
    for n in 9..=u64::MAX {
        ring.push([n; 8]).unwrap();
    }
}

#[test]
fn independent_process_shares_and_survives_a_writer() {
    let dir = std::env::temp_dir().join(format!("hv-{}", Ulid::new()));
    fs::DirBuilder::new().mode(0o700).create(&dir).unwrap();
    let path = dir.join("p");
    let listener = UnixListener::bind(&path).unwrap();
    listener.set_nonblocking(true).unwrap();
    let child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "shared_worker", "--ignored", "--nocapture"])
        .env("HYVE_TEST_RING_SOCKET", &path)
        .spawn()
        .unwrap();
    let mut fixture = Fixture { child, dir };
    let until = Instant::now() + WAIT;
    let mut peer = loop {
        match listener.accept() {
            Ok((stream, _)) => break stream,
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                assert!(Instant::now() < until && fixture.child.try_wait().unwrap().is_none());
                std::thread::sleep(Duration::from_millis(2));
            }
            Err(e) => panic!("{e}"),
        }
    };
    peer.set_nonblocking(false).unwrap();
    peer.set_read_timeout(Some(WAIT)).unwrap();
    let ring = SharedRingBuf::<Sample>::new_shared(32, LAYOUT).unwrap();
    ring.push([7; 8]).unwrap();
    ring.send(&peer).unwrap();
    let mut ready = [0];
    peer.read_exact(&mut ready).unwrap();
    assert_eq!(ready, *b"!");
    let mut out = [[0; 8]; 32];
    assert_eq!(ring.read_latest_snapshot(&mut out), 2);
    assert_eq!(&out[..2], &[[7; 8], [8; 8]]);
    peer.write_all(b"+").unwrap();
    let mut observed = 0;
    while observed < 64 || ring.revision() < 100 {
        assert!(Instant::now() < until, "peer did not publish");
        let n = ring.read_latest_snapshot(&mut out);
        for sample in &out[..n] {
            assert!(sample.iter().all(|word| *word == sample[0]));
        }
        for pair in out[..n].windows(2) {
            assert_eq!(pair[0][0] + 1, pair[1][0]);
        }
        observed += n;
    }
    fixture.child.kill().unwrap();
    fixture.child.wait().unwrap();
    ring.push([u64::MAX; 8]).unwrap();
    assert_eq!(ring.read_latest_snapshot(&mut out[..1]), 1);
    assert_eq!(out[0], [u64::MAX; 8]);
}

#[test]
fn validates_layout_capacity_and_descriptor_kind() {
    use rustix::net::{SendAncillaryBuffer, SendAncillaryMessage, SendFlags, sendmsg};
    use std::{io::IoSlice, mem::MaybeUninit, os::fd::AsFd};
    assert!(MappedRing::<Sample>::new(0, LAYOUT).is_err());
    assert!(MappedRing::<Sample>::new(usize::MAX, LAYOUT).is_err());
    let ring = MappedRing::<Sample>::new(3, LAYOUT).unwrap();
    let (a, b) = UnixStream::pair().unwrap();
    ring.send(&a).unwrap();
    assert!(MappedRing::<Sample>::receive(&b, LAYOUT + 1).is_err());
    ring.send(&a).unwrap();
    assert!(MappedRing::<u32>::receive(&b, LAYOUT).is_err());
    // A normal, resizable file must never become the backing object.
    use std::os::unix::fs::OpenOptionsExt;
    let path = std::env::temp_dir().join(format!("hv-file-{}", Ulid::new()));
    let file = fs::OpenOptions::new()
        .create_new(true)
        .read(true)
        .write(true)
        .mode(0o600)
        .open(&path)
        .unwrap();
    fs::remove_file(path).unwrap();
    file.set_len(4096).unwrap();
    let files = [file.as_fd()];
    let mut bytes = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(1))];
    let mut control = SendAncillaryBuffer::new(&mut bytes);
    assert!(control.push(SendAncillaryMessage::ScmRights(&files)));
    sendmsg(&a, &[IoSlice::new(b"R")], &mut control, SendFlags::empty()).unwrap();
    assert!(MappedRing::<Sample>::receive(&b, LAYOUT).is_err());
}

#[test]
fn independent_writers_wrap_without_tearing_samples() {
    let ring = MappedRing::<Sample>::new(8, LAYOUT).unwrap();
    let mut out = [[0; 8]; 8];
    std::thread::scope(|scope| {
        let mut workers = Vec::new();
        for writer in 0..2u64 {
            let (a, b) = UnixStream::pair().unwrap();
            ring.send(&a).unwrap();
            let peer = MappedRing::<Sample>::receive(&b, LAYOUT).unwrap();
            workers.push(scope.spawn(move || {
                for n in 1..20_001 {
                    peer.push([(writer << 32) + n; 8]).unwrap();
                }
            }));
        }
        while workers.iter().any(|w| !w.is_finished()) {
            let n = ring.read_latest_snapshot(&mut out);
            assert!(out[..n].iter().all(|v| v.iter().all(|word| *word == v[0])));
        }
    });
    assert_eq!(ring.total_pushed(), 40_000);
    assert_eq!(ring.read_latest_snapshot(&mut out), 8);
}
