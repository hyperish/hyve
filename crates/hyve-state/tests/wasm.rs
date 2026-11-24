//! The same graph on `wasm32` (32-bit pointers change handle layout, which the
//! `sig()` call syntax depends on).

#![cfg(all(target_arch = "wasm32", target_os = "unknown"))]

use std::cell::RefCell;
use std::rc::Rc;

use hyve_state::{Owner, effect, memo, signal};
use wasm_bindgen_test::wasm_bindgen_test;

#[wasm_bindgen_test]
fn callable_handles_and_diamond() {
    let mut a = signal(1);
    let b = memo(move || a() * 2);
    let c = memo(move || a() + 10);
    let seen = Rc::new(RefCell::new(Vec::new()));
    let s = seen.clone();
    effect(move || s.borrow_mut().push(b() + c()));
    a += 1;
    assert_eq!(*seen.borrow(), vec![13, 16]);
}

#[wasm_bindgen_test]
fn owner_disposal() {
    let owner = Owner::new();
    let s = owner.with(|| signal(String::from("x")));
    assert_eq!(s(), "x");
    owner.dispose();
    assert!(!s.is_alive());
}
