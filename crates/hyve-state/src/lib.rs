//! `hyve-state` — Platform-neutral reactive state graph with SoA storage.
//!
//! Exposes a single `Copy` generational-arena handle (`Signal<T>`) backed by
//! a lock-free thread-local SoA store for zero-overhead UI reactivity, plus
//! `SyncSignal<T>` for shared multi-thread paths.

use std::any::Any;
use std::cell::RefCell;
use std::fmt;
use std::ops::{AddAssign, Deref};
use std::rc::Rc;
use std::sync::{Arc, Mutex, OnceLock};

thread_local! {
    static LOCAL_ARENA: RefCell<LocalArena> = RefCell::new(LocalArena::new());
    static OBSERVER_STACK: RefCell<Vec<ObserverEntry>> = const { RefCell::new(Vec::new()) };
}

#[derive(Clone)]
enum ObserverEntry {
    Local(usize),
    #[allow(dead_code)]
    Sync(usize),
}

struct LocalArena {
    values: Vec<Option<Box<dyn Any>>>,
    generations: Vec<u64>,
    subscribers: Vec<Vec<usize>>,
    observers: Vec<Option<Rc<dyn Fn()>>>,
}

impl LocalArena {
    fn new() -> Self {
        Self {
            values: Vec::new(),
            generations: Vec::new(),
            subscribers: Vec::new(),
            observers: Vec::new(),
        }
    }

    fn alloc_signal<T: Clone + 'static>(&mut self, initial: T) -> (usize, u64) {
        let slot = self.values.len();
        let generation = 1;
        self.values.push(Some(Box::new(initial)));
        self.generations.push(generation);
        self.subscribers.push(Vec::new());
        (slot, generation)
    }

    fn alloc_observer(&mut self, callback: Rc<dyn Fn()>) -> usize {
        let id = self.observers.len();
        self.observers.push(Some(callback));
        id
    }

    fn track(&mut self, slot: usize, generation: u64, obs_id: usize) {
        if self.generations.get(slot).copied() != Some(generation) {
            return;
        }
        let subs = &mut self.subscribers[slot];
        if !subs.contains(&obs_id) {
            subs.push(obs_id);
        }
    }

    fn collect_subscribers(&self, slot: usize, generation: u64) -> Vec<Rc<dyn Fn()>> {
        if self.generations.get(slot).copied() != Some(generation) {
            return Vec::new();
        }
        self.subscribers[slot]
            .iter()
            .filter_map(|&obs_id| self.observers.get(obs_id).and_then(|o| o.as_ref().cloned()))
            .collect()
    }
}

struct ObserverGuard {
    pushed: bool,
}

impl ObserverGuard {
    fn push(entry: ObserverEntry) -> Self {
        OBSERVER_STACK.with(|stack| stack.borrow_mut().push(entry));
        Self { pushed: true }
    }
}

impl Drop for ObserverGuard {
    fn drop(&mut self) {
        if self.pushed {
            OBSERVER_STACK.with(|stack| {
                stack.borrow_mut().pop();
            });
        }
    }
}

fn current_local_observer() -> Option<usize> {
    OBSERVER_STACK.with(|stack| {
        stack.borrow().last().and_then(|entry| match entry {
            ObserverEntry::Local(id) => Some(*id),
            ObserverEntry::Sync(_) => None,
        })
    })
}

/// Thread-local `Copy` handle into a SoA generational reactive arena.
///
/// Supports arbitrary `T: Clone + 'static` (including `!Send` UI values),
/// function-call reading `sig()`, `.get()`, `.set(...)`, `.update(...)`,
/// `+=`, and `bool::toggle()`.
pub struct Signal<T: 'static> {
    slot: usize,
    generation: u64,
    getter: &'static dyn Fn() -> T,
}

/// Alias emphasizing `!Send` thread-local reactive fast-path usage.
pub type LocalSignal<T> = Signal<T>;

impl<T: 'static> Copy for Signal<T> {}

impl<T: 'static> Clone for Signal<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Clone + 'static> Signal<T> {
    fn get_by_slot(slot: usize, generation: u64) -> T {
        if let Some(obs_id) = current_local_observer() {
            LOCAL_ARENA.with(|arena| arena.borrow_mut().track(slot, generation, obs_id));
        }
        LOCAL_ARENA.with(|arena| {
            let arena = arena.borrow();
            assert_eq!(
                arena.generations.get(slot).copied(),
                Some(generation),
                "attempted to read stale or freed Signal"
            );
            arena.values[slot]
                .as_ref()
                .and_then(|b| b.downcast_ref::<T>())
                .cloned()
                .expect("signal type mismatch")
        })
    }

    /// Reads and clones the current signal value while tracking active observers.
    #[must_use]
    pub fn get(&self) -> T {
        Self::get_by_slot(self.slot, self.generation)
    }

    /// Borrows the current signal value in place without cloning `T`, while tracking active observers.
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        if let Some(obs_id) = current_local_observer() {
            let slot = self.slot;
            let generation = self.generation;
            LOCAL_ARENA.with(|arena| arena.borrow_mut().track(slot, generation, obs_id));
        }
        LOCAL_ARENA.with(|arena| {
            let arena = arena.borrow();
            assert_eq!(
                arena.generations.get(self.slot).copied(),
                Some(self.generation),
                "attempted to read stale or freed Signal"
            );
            let value = arena.values[self.slot]
                .as_ref()
                .and_then(|b| b.downcast_ref::<T>())
                .expect("signal type mismatch");
            f(value)
        })
    }

    /// Replaces the signal value and notifies subscribed memos and effects.
    pub fn set(&self, val: T) {
        let callbacks = LOCAL_ARENA.with(|arena| {
            let mut arena = arena.borrow_mut();
            if arena.generations.get(self.slot).copied() != Some(self.generation) {
                return Vec::new();
            }
            arena.values[self.slot] = Some(Box::new(val));
            arena.collect_subscribers(self.slot, self.generation)
        });
        for cb in callbacks {
            cb();
        }
    }

    /// Mutates the signal value in place and notifies subscribed memos and effects.
    pub fn update(&self, f: impl FnOnce(&mut T)) {
        let callbacks = LOCAL_ARENA.with(|arena| {
            let mut arena = arena.borrow_mut();
            if arena.generations.get(self.slot).copied() != Some(self.generation) {
                return Vec::new();
            }
            if let Some(val) = arena.values[self.slot]
                .as_mut()
                .and_then(|b| b.downcast_mut::<T>())
            {
                f(val);
            }
            arena.collect_subscribers(self.slot, self.generation)
        });
        for cb in callbacks {
            cb();
        }
    }
}

impl Signal<bool> {
    /// Toggles a boolean signal (`true` <-> `false`) and notifies subscribers.
    pub fn toggle(&self) {
        self.update(|b| *b = !*b);
    }
}

impl<T> AddAssign<T> for Signal<T>
where
    T: AddAssign + Clone + 'static,
{
    fn add_assign(&mut self, rhs: T) {
        self.update(|v| *v += rhs);
    }
}

impl<T: 'static> Deref for Signal<T> {
    type Target = dyn Fn() -> T;

    fn deref(&self) -> &Self::Target {
        self.getter
    }
}

impl<T: Clone + fmt::Debug + 'static> fmt::Debug for Signal<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.with(|v| f.debug_tuple("Signal").field(v).finish())
    }
}

/// Creates a reactive [`Signal<T>`] initialized with `initial`.
#[must_use]
pub fn signal<T: Clone + 'static>(initial: T) -> Signal<T> {
    let (slot, generation) = LOCAL_ARENA.with(|arena| arena.borrow_mut().alloc_signal(initial));
    let getter: &'static dyn Fn() -> T =
        Box::leak(Box::new(move || Signal::<T>::get_by_slot(slot, generation)));
    Signal {
        slot,
        generation,
        getter,
    }
}

/// Creates a derived read-only/cached reactive [`Signal<T>`] that automatically
/// re-computes whenever any signal read inside `compute` changes.
#[must_use]
pub fn memo<T: Clone + 'static>(compute: impl Fn() -> T + 'static) -> Signal<T> {
    let compute = Rc::new(compute);
    let slot_holder: Rc<RefCell<Option<Signal<T>>>> = Rc::new(RefCell::new(None));

    let obs_slot_holder = Rc::clone(&slot_holder);
    let obs_compute = Rc::clone(&compute);
    let obs_id_holder: Rc<RefCell<Option<usize>>> = Rc::new(RefCell::new(None));
    let obs_id_ref = Rc::clone(&obs_id_holder);

    let obs_cb: Rc<dyn Fn()> = Rc::new(move || {
        let Some(obs_id) = *obs_id_ref.borrow() else {
            return;
        };
        let next = {
            let _guard = ObserverGuard::push(ObserverEntry::Local(obs_id));
            obs_compute()
        };
        if let Some(sig) = *obs_slot_holder.borrow() {
            sig.set(next);
        }
    });

    let obs_id = LOCAL_ARENA.with(|arena| arena.borrow_mut().alloc_observer(obs_cb));
    *obs_id_holder.borrow_mut() = Some(obs_id);

    let initial = {
        let _guard = ObserverGuard::push(ObserverEntry::Local(obs_id));
        compute()
    };
    let derived = signal(initial);
    *slot_holder.borrow_mut() = Some(derived);
    derived
}

/// Registers a reactive side effect that runs immediately and automatically re-runs
/// whenever any signal read inside `run` changes.
pub fn effect(run: impl Fn() + 'static) {
    let run = Rc::new(run);
    let obs_id_holder: Rc<RefCell<Option<usize>>> = Rc::new(RefCell::new(None));
    let obs_id_ref = Rc::clone(&obs_id_holder);

    let obs_cb: Rc<dyn Fn()> = Rc::new(move || {
        let Some(obs_id) = *obs_id_ref.borrow() else {
            return;
        };
        let _guard = ObserverGuard::push(ObserverEntry::Local(obs_id));
        run();
    });

    let obs_id = LOCAL_ARENA.with(|arena| arena.borrow_mut().alloc_observer(Rc::clone(&obs_cb)));
    *obs_id_holder.borrow_mut() = Some(obs_id);
    obs_cb();
}

// ---------------------------------------------------------------------------
// Thread-safe SyncSignal<T> for multi-thread / Tier 1 backend-UI shared state
// ---------------------------------------------------------------------------

struct SyncArena {
    values: Vec<Option<Box<dyn Any + Send + Sync>>>,
    generations: Vec<u64>,
    subscribers: Vec<Vec<Arc<dyn Fn() + Send + Sync>>>,
}

impl SyncArena {
    fn new() -> Self {
        Self {
            values: Vec::new(),
            generations: Vec::new(),
            subscribers: Vec::new(),
        }
    }
}

fn global_sync_arena() -> &'static Mutex<SyncArena> {
    static ARENA: OnceLock<Mutex<SyncArena>> = OnceLock::new();
    ARENA.get_or_init(|| Mutex::new(SyncArena::new()))
}

/// Thread-safe `Send + Sync + Copy` reactive handle for shared backend/engine state.
pub struct SyncSignal<T: Send + Sync + 'static> {
    slot: usize,
    generation: u64,
    getter: &'static (dyn Fn() -> T + Send + Sync),
}

impl<T: Send + Sync + 'static> Copy for SyncSignal<T> {}

impl<T: Send + Sync + 'static> Clone for SyncSignal<T> {
    fn clone(&self) -> Self {
        *self
    }
}

impl<T: Clone + Send + Sync + 'static> SyncSignal<T> {
    fn get_by_slot(slot: usize, generation: u64) -> T {
        let arena = global_sync_arena()
            .lock()
            .expect("sync signal arena poisoned");
        assert_eq!(
            arena.generations.get(slot).copied(),
            Some(generation),
            "attempted to read stale or freed SyncSignal"
        );
        arena.values[slot]
            .as_ref()
            .and_then(|b| b.downcast_ref::<T>())
            .cloned()
            .expect("sync signal type mismatch")
    }

    /// Reads and clones the current value cleanly under the sync arena lock.
    #[must_use]
    pub fn get(&self) -> T {
        Self::get_by_slot(self.slot, self.generation)
    }

    /// Borrows the current value cleanly while holding the sync arena lock briefly.
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        let arena = global_sync_arena()
            .lock()
            .expect("sync signal arena poisoned");
        assert_eq!(
            arena.generations.get(self.slot).copied(),
            Some(self.generation),
            "attempted to read stale or freed SyncSignal"
        );
        let value = arena.values[self.slot]
            .as_ref()
            .and_then(|b| b.downcast_ref::<T>())
            .expect("sync signal type mismatch");
        f(value)
    }

    /// Replaces the value and invokes registered `Send + Sync` listeners.
    pub fn set(&self, val: T) {
        let subs = {
            let mut arena = global_sync_arena()
                .lock()
                .expect("sync signal arena poisoned");
            if arena.generations.get(self.slot).copied() != Some(self.generation) {
                return;
            }
            arena.values[self.slot] = Some(Box::new(val));
            arena.subscribers[self.slot].clone()
        };
        for cb in subs {
            cb();
        }
    }

    /// Mutates the value in place and notifies registered listeners.
    pub fn update(&self, f: impl FnOnce(&mut T)) {
        let subs = {
            let mut arena = global_sync_arena()
                .lock()
                .expect("sync signal arena poisoned");
            if arena.generations.get(self.slot).copied() != Some(self.generation) {
                return;
            }
            if let Some(val) = arena.values[self.slot]
                .as_mut()
                .and_then(|b| b.downcast_mut::<T>())
            {
                f(val);
            }
            arena.subscribers[self.slot].clone()
        };
        for cb in subs {
            cb();
        }
    }

    /// Attaches a cross-thread subscriber callback to this `SyncSignal`.
    pub fn subscribe(&self, listener: impl Fn() + Send + Sync + 'static) {
        if let Ok(mut arena) = global_sync_arena().lock()
            && arena.generations.get(self.slot).copied() == Some(self.generation)
        {
            arena.subscribers[self.slot].push(Arc::new(listener));
        }
    }
}

impl SyncSignal<bool> {
    /// Toggles a boolean `SyncSignal` and notifies listeners.
    pub fn toggle(&self) {
        self.update(|b| *b = !*b);
    }
}

impl<T> AddAssign<T> for SyncSignal<T>
where
    T: AddAssign + Clone + Send + Sync + 'static,
{
    fn add_assign(&mut self, rhs: T) {
        self.update(|v| *v += rhs);
    }
}

impl<T: Send + Sync + 'static> Deref for SyncSignal<T> {
    type Target = dyn Fn() -> T + Send + Sync;

    fn deref(&self) -> &Self::Target {
        self.getter
    }
}

impl<T: Clone + Send + Sync + fmt::Debug + 'static> fmt::Debug for SyncSignal<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.with(|v| f.debug_tuple("SyncSignal").field(v).finish())
    }
}

/// Creates a thread-safe `Send + Sync + Copy` [`SyncSignal<T>`].
#[must_use]
pub fn sync_signal<T: Clone + Send + Sync + 'static>(initial: T) -> SyncSignal<T> {
    let (slot, generation) = {
        let mut arena = global_sync_arena()
            .lock()
            .expect("sync signal arena poisoned");
        let slot = arena.values.len();
        let generation = 1;
        arena.values.push(Some(Box::new(initial)));
        arena.generations.push(generation);
        arena.subscribers.push(Vec::new());
        (slot, generation)
    };
    let getter: &'static (dyn Fn() -> T + Send + Sync) = Box::leak(Box::new(move || {
        SyncSignal::<T>::get_by_slot(slot, generation)
    }));
    SyncSignal {
        slot,
        generation,
        getter,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::rc::Rc;

    #[test]
    fn signal_is_copy_callable_and_mutates_ergonomically() {
        let mut count = signal(0);
        let copy = count;
        assert_eq!(count(), 0);
        count += 1;
        assert_eq!(copy.get(), 1);

        let liked = signal(false);
        assert!(!liked());
        liked.toggle();
        assert!(liked());
    }

    #[test]
    fn memo_and_effect_react_to_dependency_changes() {
        let base = signal(2);
        let doubled = memo(move || base.get() * 2);
        let seen = Rc::new(RefCell::new(Vec::new()));
        let seen_clone = Rc::clone(&seen);

        effect(move || {
            seen_clone.borrow_mut().push(doubled.get());
        });

        assert_eq!(*seen.borrow(), vec![4]);
        base.set(5);
        assert_eq!(doubled(), 10);
        assert_eq!(*seen.borrow(), vec![4, 10]);
    }

    #[test]
    fn sync_signal_shares_across_threads() {
        let shared = sync_signal(10u32);
        let handle = std::thread::spawn(move || shared.get() + 5);
        assert_eq!(handle.join().unwrap(), 15);
        shared.set(42);
        assert_eq!(shared(), 42);
    }
}
