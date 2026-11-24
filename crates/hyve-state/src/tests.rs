use std::cell::{Cell, RefCell};
use std::rc::Rc;

use crate::runtime::Runtime;
use crate::*;

fn live() -> (usize, usize) {
    Runtime::with(|rt| (rt.live_nodes(), rt.live_owners()))
}

fn log<T>() -> Rc<RefCell<Vec<T>>> {
    Rc::new(RefCell::new(Vec::new()))
}

#[test]
fn diamond_runs_effect_once_per_change() {
    let a = signal(1);
    let b = memo(move || a() * 2);
    let c = memo(move || a() + 10);
    let d = memo(move || b() + c());
    let seen = log();
    let s = seen.clone();
    effect(move || s.borrow_mut().push((a(), b(), c(), d())));
    a.set(2);
    a.set(3);
    assert_eq!(
        *seen.borrow(),
        vec![(1, 2, 11, 13), (2, 4, 12, 16), (3, 6, 13, 19)]
    );
}

#[test]
fn memo_equality_stops_propagation_and_memos_are_lazy() {
    let a = signal(1);
    let computed = Rc::new(Cell::new(0));
    let c = computed.clone();
    let parity = memo(move || {
        c.set(c.get() + 1);
        a() % 2
    });
    assert_eq!(computed.get(), 0, "not computed before the first read");
    let runs = Rc::new(Cell::new(0));
    let r = runs.clone();
    effect(move || {
        parity();
        r.set(r.get() + 1);
    });
    assert_eq!((computed.get(), runs.get()), (1, 1));
    a.set(3); // still odd
    assert_eq!((computed.get(), runs.get()), (2, 1));
    a.set(4);
    assert_eq!((computed.get(), runs.get()), (3, 2));
}

#[test]
fn batch_coalesces_writes() {
    let x = signal(1);
    let y = signal(2);
    let seen = log();
    let s = seen.clone();
    effect(move || s.borrow_mut().push(x() + y()));
    batch(|| {
        x.set(10);
        y.set(20);
        assert_eq!(seen.borrow().len(), 1, "effects wait for the batch");
    });
    assert_eq!(*seen.borrow(), vec![3, 30]);
}

#[test]
fn update_and_with_may_read_and_write_other_signals() {
    let a = signal(1);
    let b = signal(10);
    let seen = log();
    let s = seen.clone();
    effect(move || s.borrow_mut().push((a(), b())));
    a.update(|v| {
        *v += b.get();
        b.set(20);
        // The effect has not run yet: it waits for the closure.
        assert_eq!(seen.borrow().len(), 1);
    });
    assert_eq!(*seen.borrow(), vec![(1, 10), (11, 20)]);
    a.with(|v| b.set(*v * 2));
    assert_eq!(seen.borrow().last(), Some(&(11, 22)));
}

#[test]
fn untracked_reads_do_not_subscribe() {
    let a = signal(1);
    let b = signal(1);
    let runs = Rc::new(Cell::new(0));
    let r = runs.clone();
    effect(move || {
        a();
        untrack(|| b.get());
        b.get_untracked();
        r.set(r.get() + 1);
    });
    b.set(2);
    assert_eq!(runs.get(), 1);
    a.set(2);
    assert_eq!(runs.get(), 2);
}

#[test]
fn disposing_an_owner_returns_slots() {
    let before = live();
    let owner = Owner::new();
    let s = owner.with(|| {
        let s = signal(0);
        let m = memo(move || s() + 1);
        effect(move || {
            m();
            // A node created inside the effect run, disposed on rerun.
            let _scratch = signal(());
        });
        s
    });
    s.set(1);
    let during = live();
    assert_eq!(
        during.0,
        before.0 + 4,
        "signal, memo, effect, scratch signal"
    );
    owner.dispose();
    assert_eq!(live(), before);
    assert!(!s.is_alive());
    assert!(!owner.is_alive());
    owner.dispose(); // idempotent

    // The freed slot is reused, and the stale handle stays dead.
    let t = signal(5);
    assert!(t.is_alive());
    assert!(!s.is_alive());
    t.dispose();
    assert_eq!(live(), before);
}

#[test]
#[should_panic(expected = "disposed")]
fn using_a_disposed_signal_panics() {
    let owner = Owner::new();
    let s = owner.with(|| signal(1));
    owner.dispose();
    s.get();
}

#[test]
fn effect_children_and_cleanups_reset_on_rerun() {
    let trigger = signal(0);
    let events = log();
    let inner = log();
    let (e, i) = (events.clone(), inner.clone());
    effect(move || {
        let n = trigger();
        i.borrow_mut().push(signal(n));
        let e = e.clone();
        on_cleanup(move || e.borrow_mut().push(format!("cleanup {n}")));
    });
    trigger.set(1);
    assert_eq!(*events.borrow(), vec!["cleanup 0"]);
    let inner = inner.borrow();
    assert!(!inner[0].is_alive());
    assert!(inner[1].is_alive());
}

#[test]
fn cleanups_run_child_first_and_untracked() {
    let events = log();
    let parent = Owner::new();
    let (e1, e2) = (events.clone(), events.clone());
    let watched = signal(0);
    let runs = Rc::new(Cell::new(0));
    let r = runs.clone();
    // An effect outside the scope; cleanups must not subscribe it to anything.
    effect(move || {
        r.set(r.get() + 1);
    });
    parent.with(|| {
        on_cleanup(move || e1.borrow_mut().push("parent"));
        Owner::new().with(|| {
            on_cleanup(move || {
                watched.get();
                e2.borrow_mut().push("child")
            })
        });
    });
    parent.dispose();
    assert_eq!(*events.borrow(), vec!["child", "parent"]);
    watched.set(1);
    assert_eq!(runs.get(), 1);
}

#[test]
fn effect_can_dispose_itself() {
    let s = signal(0);
    let runs = Rc::new(Cell::new(0));
    let me: Rc<Cell<Option<Effect>>> = Rc::new(Cell::new(None));
    let (r, m) = (runs.clone(), me.clone());
    let e = effect(move || {
        r.set(r.get() + 1);
        if s() == 2 {
            m.get().expect("handle set after creation").dispose();
        }
    });
    me.set(Some(e));
    s.set(1);
    s.set(2);
    s.set(3);
    assert_eq!(runs.get(), 3);
    assert!(!e.is_alive());
}

#[test]
fn effects_chain_through_writes() {
    let source = signal(1);
    let derived = signal(0);
    let seen = log();
    let s = seen.clone();
    effect(move || derived.set(source() * 100));
    effect(move || s.borrow_mut().push(derived()));
    source.set(2);
    assert_eq!(*seen.borrow(), vec![100, 200]);
}

#[test]
fn surface_operators() {
    let mut n = signal(1);
    n += 4;
    n *= 2;
    n -= 1;
    n /= 3;
    n %= 2;
    assert_eq!(n(), 1);
    let flag = signal(false);
    flag.toggle();
    assert!(flag());
    let mut text = signal(String::from("a"));
    text += "b";
    assert_eq!(text.with(|t| t.len()), 2);
}

#[test]
#[should_panic(expected = "re-triggering")]
fn feedback_loops_are_reported() {
    let s = signal(0u64);
    effect(move || s.set(s() + 1));
}

#[test]
fn cross_thread_use_panics() {
    let s = signal(1);
    let result = std::thread::spawn(move || s.get()).join();
    assert!(result.is_err());
    assert_eq!(s.get(), 1);
}

#[test]
#[should_panic(expected = "already disposed")]
fn creating_inside_a_disposed_scope_panics() {
    let owner = Owner::new();
    owner.with(|| {
        Owner::current().dispose();
        signal(1);
    });
}
