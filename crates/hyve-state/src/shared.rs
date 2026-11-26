//! Shared values keep the reactive graph thread-affine. A graph that reads a
//! shared value tracks a local node, and writers wake its event loop.
use super::{Kind, NodeId, Runtime, Signal};
use std::any::Any;
use std::cell::Cell;
use std::collections::HashMap;
use std::marker::PhantomData;
use std::rc::Rc;
use std::sync::{Arc, LazyLock, Mutex, RwLock, TryLockError, Weak};

type Wake = Arc<dyn Fn() + Send + Sync>;
type Subscribers = HashMap<u32, (NodeId, Weak<Inbox>)>;

#[derive(Default)]
struct Pending {
    // Disposal takes precedence over any unprocessed change.
    nodes: HashMap<NodeId, bool>,
    next_wake: u64,
    wakes: HashMap<u64, Wake>,
}

#[derive(Default)]
struct Inbox(Mutex<Pending>);

impl Inbox {
    fn post(&self, id: NodeId, dispose: bool) {
        let wakes: Vec<_> = {
            let mut pending = self.0.lock().expect("shared signal inbox");
            let was_empty = pending.nodes.is_empty();
            *pending.nodes.entry(id).or_default() |= dispose;
            if !was_empty {
                return;
            }
            pending.wakes.values().cloned().collect()
        };
        for wake in wakes {
            wake();
        }
    }
}

thread_local! {
    static INBOX: Arc<Inbox> = Arc::default();
    static HOLDING_VALUE: Cell<bool> = const { Cell::new(false) };
}

/// Keeps a host's cross-thread wake callback registered for its lifetime.
pub struct SharedWake {
    inbox: Arc<Inbox>,
    key: u64,
}

impl Drop for SharedWake {
    fn drop(&mut self) {
        let removed = self
            .inbox
            .0
            .lock()
            .expect("shared signal inbox")
            .wakes
            .remove(&self.key);
        drop(removed);
    }
}

/// Registers an event-loop wake for this thread. The callback may run on a
/// writer thread and should schedule [`drain_shared_updates`] on this thread.
pub fn on_shared_wake(wake: impl Fn() + Send + Sync + 'static) -> SharedWake {
    let wake: Wake = Arc::new(wake);
    INBOX.with(|inbox| {
        let (key, ready) = {
            let mut p = inbox.0.lock().expect("shared signal inbox");
            let key = p.next_wake;
            p.next_wake = key
                .checked_add(1)
                .expect("shared signal wake IDs exhausted");
            p.wakes.insert(key, wake.clone());
            (key, !p.nodes.is_empty())
        };
        if ready {
            wake();
        }
        SharedWake {
            inbox: inbox.clone(),
            key,
        }
    })
}

/// Runs pending cross-thread invalidations and their effects on this thread.
/// Multiple writes to a signal before this call produce one invalidation.
pub fn drain_shared_updates() -> bool {
    let nodes = INBOX.with(|i| std::mem::take(&mut i.0.lock().expect("shared signal inbox").nodes));
    if nodes.is_empty() {
        return false;
    }
    Runtime::with(|rt| {
        rt.batch(|| {
            for (id, dispose) in nodes {
                if rt.is_alive(id) {
                    if dispose {
                        rt.dispose_node(id);
                    } else {
                        rt.notify(id);
                    }
                }
            }
        })
    });
    true
}

struct Record {
    value: Box<dyn Any + Send + Sync>,
    subscribers: Mutex<Option<Subscribers>>,
}

type Registry = HashMap<NodeId, Arc<Record>>;
static REGISTRY: LazyLock<Mutex<Registry>> = LazyLock::new(Mutex::default);

fn expired() -> ! {
    panic!("hyve-state: a shared signal was used after its owner was disposed")
}

fn get(id: NodeId) -> Arc<Record> {
    let found = REGISTRY
        .lock()
        .expect("shared signal registry")
        .get(&id)
        .cloned();
    found.unwrap_or_else(|| expired())
}

struct Registration(Cell<Option<NodeId>>);
impl Drop for Registration {
    fn drop(&mut self) {
        if let Some(id) = self.0.get() {
            retire(id, false);
        }
    }
}

/// Creates an owner-scoped [`Signal`] usable on multiple threads. The
/// constructor [`super::signal`] keeps ordinary UI state thread-local.
pub fn shared_signal<T: Send + Sync + 'static>(value: T) -> Signal<T> {
    let registration = Rc::new(Registration(Cell::new(None)));
    let id = Runtime::with(|rt| rt.create(Kind::Signal, Some(registration.clone()), None));
    registration.0.set(Some(id));
    let subscriber = INBOX.with(Arc::downgrade);
    let record = Record {
        value: Box::new(RwLock::new(value)),
        subscribers: Mutex::new(Some(HashMap::from([(id.runtime, (id, subscriber))]))),
    };
    REGISTRY
        .lock()
        .expect("shared signal registry")
        .insert(id, Arc::new(record));
    Signal {
        id,
        shared: true,
        _marker: PhantomData,
    }
}

pub(super) fn is_alive(id: NodeId) -> bool {
    REGISTRY
        .lock()
        .expect("shared signal registry")
        .contains_key(&id)
}

fn retire(id: NodeId, include_origin: bool) {
    let record = REGISTRY.lock().expect("shared signal registry").remove(&id);
    if let Some(record) = record {
        let subscribers = record
            .subscribers
            .lock()
            .expect("shared signal observers")
            .take()
            .unwrap_or_default();
        for (_, (node, inbox)) in subscribers {
            if include_origin || node != id {
                if let Some(inbox) = inbox.upgrade() {
                    inbox.post(node, true);
                }
            }
        }
    }
}

pub(super) fn dispose(id: NodeId) {
    Runtime::with(|rt| {
        if rt.is_alive(id) {
            rt.dispose_node(id);
        } else {
            retire(id, true);
        }
    });
}

struct Holding<'a>(&'a Cell<bool>, bool);
impl Drop for Holding<'_> {
    fn drop(&mut self) {
        self.0.set(self.1);
    }
}

// Top-level reads/writes wait for concurrent writers. A nested borrow never
// waits on another held lock, which also prevents opposite-order deadlocks.
fn borrow_value<R>(f: impl FnOnce(bool) -> R) -> R {
    HOLDING_VALUE.with(|holding| {
        let nested = holding.replace(true);
        let _restore = Holding(holding, nested);
        f(nested)
    })
}

fn nested_lock<T>(result: Result<T, TryLockError<T>>) -> T {
    match result {
        Ok(value) => value,
        Err(TryLockError::Poisoned(error)) => error.into_inner(),
        Err(TryLockError::WouldBlock) => {
            panic!("hyve-state: conflicting nested shared signal borrow")
        }
    }
}

impl Record {
    fn track(&self, rt: &Runtime) {
        if !rt.observing() {
            return;
        }
        let node = {
            let mut state = self.subscribers.lock().expect("shared signal observers");
            let Some(subscribers) = state.as_mut() else {
                drop(state);
                expired()
            };
            subscribers
                .entry(rt.id)
                .or_insert_with(|| (rt.mirror(), INBOX.with(Arc::downgrade)))
                .0
        };
        rt.read(node, true);
    }

    fn lock<T: 'static>(&self) -> &RwLock<T> {
        self.value
            .downcast_ref()
            .expect("shared signal type mismatch")
    }
}

pub(super) fn read<T: 'static, R>(id: NodeId, track: bool, f: impl FnOnce(&T) -> R) -> R {
    let record = get(id);
    if track {
        Runtime::with(|rt| record.track(rt));
    }
    let lock = record.lock::<T>();
    borrow_value(|nested| {
        let value = if nested {
            nested_lock(lock.try_read())
        } else {
            lock.read().unwrap_or_else(|e| e.into_inner())
        };
        f(&value)
    })
}

pub(super) fn update<T: 'static>(id: NodeId, f: impl FnOnce(&mut T)) {
    let record = get(id);
    let lock = record.lock::<T>();
    borrow_value(|nested| {
        let mut value = if nested {
            nested_lock(lock.try_write())
        } else {
            lock.write().unwrap_or_else(|e| e.into_inner())
        };
        f(&mut value);
    });
    let targets: Vec<_> = {
        let mut state = record.subscribers.lock().expect("shared signal observers");
        let Some(subscribers) = state.as_mut() else {
            return;
        };
        subscribers.retain(|_, (_, inbox)| inbox.strong_count() != 0);
        subscribers.values().cloned().collect()
    };
    Runtime::with(|rt| {
        for (node, inbox) in targets {
            if node.runtime == rt.id {
                if rt.is_alive(node) {
                    rt.notify(node);
                }
            } else if let Some(inbox) = inbox.upgrade() {
                inbox.post(node, false);
            }
        }
    });
}
