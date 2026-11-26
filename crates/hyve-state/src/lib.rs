//! `hyve-state` — the platform-neutral reactive graph (`docs/02-architecture.md` §2.2).
//!
//! ```
//! use hyve_state::{effect, memo, signal};
//!
//! let mut count = signal(1);
//! let doubled = memo(move || count() * 2);
//! effect(move || println!("doubled = {}", doubled()));
//! count += 1; // prints "doubled = 4" once
//! assert_eq!(doubled(), 4);
//! ```
//!
//! - [`Signal<T>`] is a `Copy` handle into a per-thread arena. Read with
//!   `sig()` (when `T: Clone`), [`get`](Signal::get) or [`with`](Signal::with);
//!   write with [`set`](Signal::set), [`update`](Signal::update), compound
//!   assignment (`+=`, …) or [`toggle`](Signal::toggle).
//! - [`Memo<T>`] is a read-only derived value, computed lazily, that only
//!   notifies when its result changes.
//! - [`effect`] reruns a closure when what it read changes — at most once per
//!   change, even through diamond-shaped dependencies.
//! - [`Owner`] scopes own the nodes created inside them; disposing a scope
//!   frees their slots, and using a freed handle panics instead of misbehaving.
//!   Nodes created outside any scope belong to a process-lifetime root.
//! - Reading and writing *other* signals inside `update`/`with` closures is
//!   allowed; effects triggered there run after the closure returns.
//!
//! [`signal`] uses the creating thread's arena. [`shared_signal`] returns
//! the same handle type with a value accessible on worker threads. Observers
//! run on their own thread when its event loop calls [`drain_shared_updates`].
//!
//! The graph algorithm follows `reactive_graph` (push-pull, three node
//! states); the arena, owner tree and handle surface are our own.

mod callable;
mod runtime;
mod shared;

pub use shared::{SharedWake, drain_shared_updates, on_shared_wake, shared_signal};

use std::cell::RefCell;
use std::fmt;
use std::marker::PhantomData;
use std::ops::{AddAssign, Deref, DivAssign, MulAssign, RemAssign, SubAssign};
use std::rc::Rc;

use callable::{Readable, callable};
use runtime::{Kind, NodeId, OwnerId, Runtime};

macro_rules! signal_type {
    (@assign $name:ident, $tr:ident $method:ident $op:tt) => {
        impl<T: $tr<R> + 'static, R> $tr<R> for $name<T> {
            fn $method(&mut self, rhs: R) {
                self.update(move |v| *v $op rhs);
            }
        }
    };
    ($(#[$meta:meta])* $name:ident, $marker:ty, $(#[$ctor_meta:meta])* $ctor:ident, [$($bounds:tt)*]) => {
        $(#[$meta])*
        pub struct $name<T: 'static> {
            id: NodeId,
            shared: bool,
            _marker: PhantomData<$marker>,
        }

        impl<T: 'static> Clone for $name<T> {
            fn clone(&self) -> Self {
                *self
            }
        }
        impl<T: 'static> Copy for $name<T> {}

        impl<T: 'static> PartialEq for $name<T> {
            fn eq(&self, other: &Self) -> bool {
                self.id == other.id
            }
        }
        impl<T: 'static> Eq for $name<T> {}

        impl<T: 'static> fmt::Debug for $name<T> {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.debug_struct(stringify!($name))
                    .field("index", &self.id.index)
                    .field("generation", &self.id.generation)
                    .finish()
            }
        }

        $(#[$ctor_meta])*
        pub fn $ctor<T: $($bounds)*>(value: T) -> $name<T> {
            let cell: Rc<dyn std::any::Any> = Rc::new(RefCell::new(value));
            let id = Runtime::with(|rt| rt.create(Kind::Signal, Some(cell), None));
            $name { id, shared: false, _marker: PhantomData }
        }

        impl<T: 'static> $name<T> {
            fn cell(&self, track: bool) -> Rc<RefCell<T>> {
                Runtime::with(|rt| rt.read(self.id, track))
                    .downcast::<RefCell<T>>()
                    .unwrap_or_else(|_| unreachable!("signal value type mismatch"))
            }

            fn read<R>(&self, track: bool, f: impl FnOnce(&T) -> R) -> R {
                if self.shared {
                    return batch(|| shared::read(self.id, track, f));
                }
                batch(|| {
                    let cell = self.cell(track);
                    let value = cell.try_borrow().unwrap_or_else(|_| {
                        panic!("hyve-state: a signal was read inside its own `update` closure")
                    });
                    f(&value)
                })
            }

            /// Reads the value by reference, tracking the dependency.
            pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
                self.read(true, f)
            }

            /// Reads the value by reference without tracking.
            pub fn with_untracked<R>(&self, f: impl FnOnce(&T) -> R) -> R {
                self.read(false, f)
            }

            /// Mutates the value in place and notifies observers once the
            /// closure returns. The closure may read and write other signals;
            /// effects run after it finishes.
            pub fn update(&self, f: impl FnOnce(&mut T)) {
                if self.shared {
                    batch(|| shared::update(self.id, f));
                    return;
                }
                batch(|| {
                    let cell = self.cell(false);
                    {
                        let mut value = cell.try_borrow_mut().unwrap_or_else(|_| {
                            panic!("hyve-state: a signal was written inside its own `with`/`update` closure")
                        });
                        f(&mut value);
                    }
                    Runtime::with(|rt| rt.notify(self.id));
                });
            }

            /// Replaces the value and notifies observers.
            pub fn set(&self, value: T) {
                self.update(move |v| *v = value);
            }

            /// Whether the signal (and its owner scope) is still alive.
            pub fn is_alive(&self) -> bool {
                if self.shared { return shared::is_alive(self.id); }
                Runtime::with(|rt| rt.is_alive(self.id))
            }

            /// Frees the signal now instead of with its owner scope.
            pub fn dispose(self) {
                if self.shared { shared::dispose(self.id); return; }
                Runtime::with(|rt| rt.dispose_node(self.id));
            }
        }

        impl<T: Clone + 'static> $name<T> {
            /// Clones the value, tracking the dependency.
            pub fn get(&self) -> T {
                self.with(T::clone)
            }

            /// Clones the value without tracking.
            pub fn get_untracked(&self) -> T {
                self.with_untracked(T::clone)
            }
        }

        impl $name<bool> {
            /// Flips a boolean signal.
            pub fn toggle(&self) {
                self.update(|v| *v = !*v);
            }
        }

        impl<T: Clone + 'static> Readable for $name<T> {
            type Output = T;
            fn read_value(this: &Self) -> T {
                this.get()
            }
        }

        /// Enables `sig()` as a tracked clone of the value.
        impl<T: Clone + 'static> Deref for $name<T> {
            type Target = dyn Fn() -> T;
            fn deref(&self) -> &Self::Target {
                callable(self)
            }
        }

        signal_type!(@assign $name, AddAssign add_assign +=);
        signal_type!(@assign $name, SubAssign sub_assign -=);
        signal_type!(@assign $name, MulAssign mul_assign *=);
        signal_type!(@assign $name, DivAssign div_assign /=);
        signal_type!(@assign $name, RemAssign rem_assign %=);
    };
}

signal_type!(
    /// A reactive value: a `Copy` handle into the thread's reactive arena.
    Signal,
    fn() -> T,
    /// Creates a [`Signal`] owned by the current owner scope.
    signal,
    [Send + Sync + 'static]
);

signal_type!(
    /// A [`Signal`] for values that are not `Send`/`Sync` (opt-in). The handle
    /// itself is not `Send`, so it cannot leave its thread.
    LocalSignal,
    *const T,
    /// Creates a [`LocalSignal`] owned by the current owner scope.
    local_signal,
    ['static]
);

/// A read-only derived value, recomputed lazily when its inputs change and
/// notifying observers only when the result differs (`PartialEq`).
pub struct Memo<T: 'static> {
    id: NodeId,
    _marker: PhantomData<fn() -> T>,
}

impl<T: 'static> Clone for Memo<T> {
    fn clone(&self) -> Self {
        *self
    }
}
impl<T: 'static> Copy for Memo<T> {}

impl<T: 'static> PartialEq for Memo<T> {
    fn eq(&self, other: &Self) -> bool {
        self.id == other.id
    }
}
impl<T: 'static> Eq for Memo<T> {}

impl<T: 'static> fmt::Debug for Memo<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("Memo")
            .field("index", &self.id.index)
            .field("generation", &self.id.generation)
            .finish()
    }
}

/// Creates a [`Memo`] owned by the current owner scope. `f` first runs when
/// the memo is first read.
pub fn memo<T: PartialEq + Send + Sync + 'static>(f: impl Fn() -> T + 'static) -> Memo<T> {
    let cell: Rc<RefCell<Option<T>>> = Rc::new(RefCell::new(None));
    let slot = Rc::clone(&cell);
    let compute: Rc<dyn Fn() -> bool> = Rc::new(move || {
        let next = f();
        let mut current = slot.try_borrow_mut().unwrap_or_else(|_| {
            panic!("hyve-state: a memo was recomputed while it was being read")
        });
        if current.as_ref() == Some(&next) {
            false
        } else {
            *current = Some(next);
            true
        }
    });
    let id = Runtime::with(|rt| rt.create(Kind::Memo, Some(cell), Some(compute)));
    Memo {
        id,
        _marker: PhantomData,
    }
}

impl<T: 'static> Memo<T> {
    fn read<R>(&self, track: bool, f: impl FnOnce(&T) -> R) -> R {
        batch(|| {
            let cell = Runtime::with(|rt| rt.read(self.id, track))
                .downcast::<RefCell<Option<T>>>()
                .unwrap_or_else(|_| unreachable!("memo value type mismatch"));
            let value = cell.borrow();
            f(value
                .as_ref()
                .expect("hyve-state: a memo read itself while computing its first value"))
        })
    }

    /// Reads the current value by reference, tracking the dependency.
    pub fn with<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        self.read(true, f)
    }

    /// Reads the current value by reference without tracking.
    pub fn with_untracked<R>(&self, f: impl FnOnce(&T) -> R) -> R {
        self.read(false, f)
    }

    /// Whether the memo (and its owner scope) is still alive.
    pub fn is_alive(&self) -> bool {
        Runtime::with(|rt| rt.is_alive(self.id))
    }

    /// Frees the memo now instead of with its owner scope.
    pub fn dispose(self) {
        Runtime::with(|rt| rt.dispose_node(self.id));
    }
}

impl<T: Clone + 'static> Memo<T> {
    /// Clones the current value, tracking the dependency.
    pub fn get(&self) -> T {
        self.with(T::clone)
    }

    /// Clones the current value without tracking.
    pub fn get_untracked(&self) -> T {
        self.with_untracked(T::clone)
    }
}

impl<T: Clone + 'static> Readable for Memo<T> {
    type Output = T;
    fn read_value(this: &Self) -> T {
        this.get()
    }
}

/// Enables `memo()` as a tracked clone of the value.
impl<T: Clone + 'static> Deref for Memo<T> {
    type Target = dyn Fn() -> T;
    fn deref(&self) -> &Self::Target {
        callable(self)
    }
}

/// Handle to a running effect.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Effect {
    id: NodeId,
}

impl Effect {
    /// Whether the effect is still running.
    pub fn is_alive(&self) -> bool {
        Runtime::with(|rt| rt.is_alive(self.id))
    }

    /// Stops the effect and disposes everything it created. Safe to call from
    /// inside the effect itself.
    pub fn dispose(self) {
        Runtime::with(|rt| rt.dispose_node(self.id));
    }
}

/// Runs `f` now (or after the current batch/effect) and again whenever a
/// signal or memo it read changes. Nodes created inside `f` are disposed, and
/// cleanups registered inside it run, before each rerun.
pub fn effect(f: impl FnMut() + 'static) -> Effect {
    let f = RefCell::new(f);
    let compute: Rc<dyn Fn() -> bool> = Rc::new(move || {
        let mut f = f
            .try_borrow_mut()
            .unwrap_or_else(|_| panic!("hyve-state: an effect re-entered itself"));
        f();
        false
    });
    let id = Runtime::with(|rt| {
        let id = rt.create(Kind::Effect, None, Some(compute));
        rt.batch(|| rt.schedule(id));
        id
    });
    Effect { id }
}

/// Groups writes so effects run once after `f` returns.
pub fn batch<R>(f: impl FnOnce() -> R) -> R {
    Runtime::with(|rt| rt.batch(f))
}

/// Runs `f` without recording dependencies of the current effect or memo.
pub fn untrack<R>(f: impl FnOnce() -> R) -> R {
    Runtime::with(|rt| rt.untrack(f))
}

/// Registers `f` to run when the current owner scope is disposed (or, inside
/// an effect or memo, before its next run).
pub fn on_cleanup(f: impl FnOnce() + 'static) {
    Runtime::with(|rt| rt.on_cleanup(Box::new(f)));
}

/// An owner scope (component, feature, request, …). `Copy` handle.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub struct Owner {
    id: OwnerId,
}

impl Owner {
    /// A new scope, child of the current one.
    pub fn new() -> Owner {
        Owner {
            id: Runtime::with(|rt| rt.new_owner()),
        }
    }

    /// The current scope (the process-lifetime root outside any scope).
    pub fn current() -> Owner {
        Owner {
            id: Runtime::with(|rt| rt.current_owner()),
        }
    }

    /// Runs `f` with this scope as the current owner.
    ///
    /// # Panics
    ///
    /// Panics if the scope was disposed.
    pub fn with<R>(&self, f: impl FnOnce() -> R) -> R {
        Runtime::with(|rt| rt.with_owner(self.id, f))
    }

    /// Disposes the scope, its child scopes and every node they own, running
    /// cleanups child-first. Idempotent.
    ///
    /// # Panics
    ///
    /// Panics for the root scope.
    pub fn dispose(&self) {
        Runtime::with(|rt| rt.dispose_owner(self.id));
    }

    /// Whether the scope is still alive.
    pub fn is_alive(&self) -> bool {
        Runtime::with(|rt| rt.owner_alive(self.id))
    }
}

impl Default for Owner {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests;
