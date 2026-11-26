//! Share a user-restricted pagefile section with a separate process.
#![cfg(all(windows, feature = "shm", target_has_atomic = "64"))]
use hyve_core::MappedRing;
use std::{
    io::{Read, Write},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
type Sample = [u64; 8];
const LAYOUT: u64 = 0x1001;

#[test]
#[ignore = "launched by the parent process"]
fn windows_ring_worker() {
    let mut token = [0; 24];
    std::io::stdin().read_exact(&mut token).unwrap();
    let ring = MappedRing::<Sample>::from_token(&token, LAYOUT).unwrap();
    let mut sample = [[0; 8]];
    assert_eq!(ring.read_latest_snapshot(&mut sample), 1);
    assert_eq!(sample[0], [1; 8]);
    for i in 2..=u64::MAX {
        ring.push([i; 8]).unwrap();
    }
}
#[test]
fn windows_ring_checks_layout_and_survives_process_exit() {
    let ring = MappedRing::<Sample>::new(8, LAYOUT).unwrap();
    let token = ring.share_token();
    assert!(MappedRing::<Sample>::from_token(&token, LAYOUT + 1).is_err());
    assert!(MappedRing::<u32>::from_token(&token, LAYOUT).is_err());
    let mut short = token;
    short[16..].copy_from_slice(&56_u64.to_le_bytes());
    assert!(MappedRing::<Sample>::from_token(&short, LAYOUT).is_err());
    ring.push([1; 8]).unwrap();
    let mut child = Command::new(std::env::current_exe().unwrap())
        .args(["--exact", "windows_ring_worker", "--ignored", "--nocapture"])
        .stdin(Stdio::piped())
        .spawn()
        .unwrap();
    child.stdin.take().unwrap().write_all(&token).unwrap();
    let until = Instant::now() + Duration::from_secs(15);
    let mut out = [[0; 8]; 8];
    let mut seen = 0;
    while (seen < 10 || ring.revision() < 20)
        && Instant::now() < until
        && child.try_wait().unwrap().is_none()
    {
        let count = ring.read_latest_snapshot(&mut out);
        for sample in &out[..count] {
            assert!(sample.iter().all(|&n| n == sample[0]));
        }
        seen += count;
    }
    let _ = child.kill();
    child.wait().unwrap();
    assert!(ring.revision() >= 20 && seen >= 10);
    ring.push([99; 8]).unwrap();
    assert_eq!(ring.read_latest_snapshot(&mut out[..1]), 1);
    assert_eq!(out[0], [99; 8]);
    drop(ring);
    assert!(MappedRing::<Sample>::from_token(&token, LAYOUT).is_err());
}
