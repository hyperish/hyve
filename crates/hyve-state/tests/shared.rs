#![cfg(not(target_arch = "wasm32"))]
#![allow(missing_docs)]

use hyve_state::{
    Owner, Signal, drain_shared_updates, effect, memo, on_shared_wake, shared_signal,
};
use std::{cell::RefCell, rc::Rc, sync::mpsc, thread, time::Duration};

#[test]
fn concurrent_writes_wake_each_observing_thread_and_preserve_owner_lifetime() {
    let owner = Owner::new();
    let value: Signal<u32> = owner.with(|| shared_signal(0));
    let seen = Rc::new(RefCell::new(Vec::new()));
    let by_main = seen.clone();
    owner.with(|| {
        let doubled = memo(move || value() * 2);
        effect(move || by_main.borrow_mut().push(doubled()));
    });
    let (woke, wakes) = mpsc::channel();
    let _wake = on_shared_wake(move || woke.send(()).unwrap());
    let (ready, subscribed) = mpsc::channel();
    let (advance, wait) = mpsc::channel();
    let watcher = thread::spawn(move || {
        let seen = Rc::new(RefCell::new(Vec::new()));
        let log = seen.clone();
        effect(move || log.borrow_mut().push(value()));
        ready.send(()).unwrap();
        wait.recv().unwrap();
        assert!(drain_shared_updates());
        assert_eq!(*seen.borrow(), [0, 400]);
        ready.send(()).unwrap();
        wait.recv().unwrap();
        assert!(drain_shared_updates());
        assert!(!value.is_alive());
    });
    subscribed.recv().unwrap();
    thread::scope(|s| {
        for _ in 0..4 {
            s.spawn(move || {
                for _ in 0..100 {
                    value.update(|v| *v += 1);
                }
            });
        }
    });
    wakes.recv_timeout(Duration::from_secs(2)).unwrap();
    assert!(
        wakes.try_recv().is_err(),
        "writes coalesce into one event-loop wake"
    );
    assert!(drain_shared_updates());
    assert_eq!(*seen.borrow(), [0, 800]);
    advance.send(()).unwrap();
    subscribed.recv().unwrap();
    owner.dispose();
    assert!(!value.is_alive());
    advance.send(()).unwrap();
    watcher.join().unwrap();
    assert!(std::panic::catch_unwind(|| value.get()).is_err());
    assert_ne!(shared_signal(42), value);
}

#[test]
fn reads_inside_updates_and_disposal_from_a_worker() {
    let owner = Owner::new();
    let (a, b) = owner.with(|| (shared_signal(2), shared_signal(3)));
    let thread = thread::spawn(move || {
        a.update(|a| *a += b());
        assert_eq!(a(), 5);
        a.dispose();
        assert!(!a.is_alive());
    });
    thread.join().unwrap();
    drain_shared_updates();
    assert!(!a.is_alive());
    assert_eq!(b(), 3);
    owner.dispose();
    assert!(!b.is_alive());
}
