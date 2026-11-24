//! Runs on `wasm32-unknown-unknown` under Node (`cargo test --target wasm32-unknown-unknown`).
#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use hyve_core::{CancellationToken, Id, Scope, SharedRingBuf, Timestamp, TraceId, Ulid};
use wasm_bindgen_test::wasm_bindgen_test;

#[wasm_bindgen_test]
fn clock_is_real() {
    let now = Timestamp::now();
    assert!(now > "2025-01-01T00:00:00Z".parse().unwrap());
}

#[wasm_bindgen_test]
fn ulids_use_csprng_and_stay_monotonic() {
    let mut prev = Ulid::new();
    for _ in 0..1000 {
        let next = Ulid::new();
        assert!(next > prev);
        prev = next;
    }
    struct User;
    let id: Id<User> = Id::new();
    assert_eq!(id.to_string().parse::<Id<User>>().unwrap(), id);
    assert_ne!(TraceId::new(), TraceId::new());
}

#[wasm_bindgen_test]
fn scopes_and_cancellation() {
    let app = Scope::root();
    app.provide(5u32);
    let child = app.child();
    assert_eq!(*child.get::<u32>().unwrap(), 5);
    let token: &CancellationToken = child.cancellation();
    app.dispose();
    assert!(token.is_cancelled());
}

#[wasm_bindgen_test]
fn ring_buffer() {
    let ring = SharedRingBuf::<[u32; 2]>::new(3);
    for i in 0..5u32 {
        ring.push([i, i * 2]);
    }
    let mut out = [[0u32; 2]; 3];
    assert_eq!(ring.read_latest_snapshot(&mut out), 3);
    assert_eq!(out, [[2, 4], [3, 6], [4, 8]]);
}
