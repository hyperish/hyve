//! Scoped dependency provision (`hyperish/docs` `02-core-contracts.md` §1).
//!
//! A [`Scope`] is a node in a tree (app → component/feature → request/view).
//! Values provided to a scope are visible to it and to its descendants; the
//! nearest provider wins; siblings never see each other's values. `orbic` and
//! `serge` decide *when* scopes are created and resolved; this module only
//! offers the mechanism.
//!
//! The *current* scope is tracked per thread so that code without an explicit
//! scope parameter (for example an `#[server]` stub looking up its transport)
//! can find it. Futures carry their scope across `.await` points with
//! [`Scope::scoped`].

use std::any::{Any, TypeId, type_name};
use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;
use std::future::Future;
use std::ops::Deref;
use std::pin::Pin;
use std::sync::{Arc, RwLock};
use std::task::{Context, Poll};

use crate::{CancellationToken, HyveError};

type ValueMap = HashMap<TypeId, Arc<dyn Any + Send + Sync>>;

/// A node in the scope tree. Cheap to clone; clones refer to the same node.
#[derive(Clone)]
pub struct Scope {
    inner: Arc<ScopeInner>,
}

struct ScopeInner {
    parent: Option<Scope>,
    values: RwLock<ValueMap>,
    cancel: CancellationToken,
}

impl Scope {
    /// A new root scope with its own cancellation token.
    pub fn root() -> Scope {
        Scope {
            inner: Arc::new(ScopeInner {
                parent: None,
                values: RwLock::default(),
                cancel: CancellationToken::new(),
            }),
        }
    }

    /// A child scope. It sees this scope's values and is cancelled with it.
    pub fn child(&self) -> Scope {
        Scope {
            inner: Arc::new(ScopeInner {
                parent: Some(self.clone()),
                values: RwLock::default(),
                cancel: self.inner.cancel.child(),
            }),
        }
    }

    /// The parent scope, if any.
    pub fn parent(&self) -> Option<&Scope> {
        self.inner.parent.as_ref()
    }

    /// Provides `value` in this scope, replacing a previous value of the same type
    /// in this scope (values in ancestors are shadowed, not replaced).
    pub fn provide<T: Send + Sync + 'static>(&self, value: T) -> &Self {
        self.provide_arc(Arc::new(value))
    }

    /// Like [`provide`](Self::provide) for a value that is already shared.
    pub fn provide_arc<T: Send + Sync + 'static>(&self, value: Arc<T>) -> &Self {
        write(&self.inner.values).insert(TypeId::of::<T>(), value);
        self
    }

    /// Whether this scope itself (not an ancestor) provides `T`.
    pub fn provides_locally<T: 'static>(&self) -> bool {
        read(&self.inner.values).contains_key(&TypeId::of::<T>())
    }

    /// The nearest provided `T`, searching this scope then its ancestors.
    pub fn get<T: Send + Sync + 'static>(&self) -> Option<Ctx<T>> {
        let id = TypeId::of::<T>();
        let mut scope = Some(self);
        while let Some(s) = scope {
            if let Some(v) = read(&s.inner.values).get(&id) {
                let arc = Arc::clone(v).downcast::<T>().ok()?;
                return Some(Ctx(arc));
            }
            scope = s.inner.parent.as_ref();
        }
        None
    }

    /// Like [`get`](Self::get) but returns an `Internal` error naming the
    /// missing type. Assembly code should resolve requirements up front so this
    /// error only signals a framework defect.
    pub fn require<T: Send + Sync + 'static>(&self) -> Result<Ctx<T>, HyveError> {
        self.get::<T>().ok_or_else(|| {
            HyveError::internal(format!(
                "no `{}` was provided in this scope or its ancestors",
                type_name::<T>()
            ))
        })
    }

    /// This scope's cancellation token (a child of the parent's).
    pub fn cancellation(&self) -> &CancellationToken {
        &self.inner.cancel
    }

    /// Ends the scope: cancels its token (and so every descendant's) and drops
    /// the values it provided. Values stay alive while a [`Ctx`] still holds them.
    pub fn dispose(&self) {
        self.inner.cancel.cancel();
        write(&self.inner.values).clear();
    }

    /// Whether [`dispose`](Self::dispose) was called on this scope or an ancestor.
    pub fn is_disposed(&self) -> bool {
        self.inner.cancel.is_cancelled()
    }

    /// Runs `f` with this scope as the thread's current scope.
    pub fn enter<R>(&self, f: impl FnOnce() -> R) -> R {
        let _guard = EnterGuard::push(self.clone());
        f()
    }

    /// The thread's current scope, if any.
    pub fn current() -> Option<Scope> {
        CURRENT.with(|c| c.borrow().last().cloned())
    }

    /// Wraps `future` so that this scope is current while it is polled.
    pub fn scoped<F: Future>(&self, future: F) -> Scoped<F> {
        Scoped {
            scope: self.clone(),
            future,
        }
    }

    /// Whether two handles refer to the same scope node.
    pub fn ptr_eq(&self, other: &Scope) -> bool {
        Arc::ptr_eq(&self.inner, &other.inner)
    }
}

impl fmt::Debug for Scope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let mut depth = 0;
        let mut p = self.parent();
        while let Some(s) = p {
            depth += 1;
            p = s.parent();
        }
        f.debug_struct("Scope")
            .field("depth", &depth)
            .field("values", &read(&self.inner.values).len())
            .field("disposed", &self.is_disposed())
            .finish()
    }
}

fn read(lock: &RwLock<ValueMap>) -> std::sync::RwLockReadGuard<'_, ValueMap> {
    lock.read().unwrap_or_else(|p| p.into_inner())
}

fn write(lock: &RwLock<ValueMap>) -> std::sync::RwLockWriteGuard<'_, ValueMap> {
    lock.write().unwrap_or_else(|p| p.into_inner())
}

thread_local! {
    static CURRENT: RefCell<Vec<Scope>> = const { RefCell::new(Vec::new()) };
}

struct EnterGuard;

impl EnterGuard {
    fn push(scope: Scope) -> EnterGuard {
        CURRENT.with(|c| c.borrow_mut().push(scope));
        EnterGuard
    }
}

impl Drop for EnterGuard {
    fn drop(&mut self) {
        CURRENT.with(|c| {
            c.borrow_mut().pop();
        });
    }
}

/// Future adapter returned by [`Scope::scoped`].
#[must_use = "futures do nothing unless awaited"]
pub struct Scoped<F> {
    scope: Scope,
    future: F,
}

impl<F: Future> Future for Scoped<F> {
    type Output = F::Output;

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<F::Output> {
        // Pin projection instead of boxing keeps scoped futures allocation-free.
        let this = self.project();
        let _guard = EnterGuard::push(this.scope.clone());
        this.future.poll(cx)
    }
}

struct ScopedProj<'a, F> {
    scope: &'a Scope,
    future: Pin<&'a mut F>,
}

impl<F> Scoped<F> {
    fn project(self: Pin<&mut Self>) -> ScopedProj<'_, F> {
        pin_project_scoped(self)
    }
}

#[allow(unsafe_code)]
fn pin_project_scoped<F>(this: Pin<&mut Scoped<F>>) -> ScopedProj<'_, F> {
    // SAFETY: `Scoped` never moves `future` out of a pinned `Scoped`, has no
    // `Drop` impl that touches it, and does not implement `Unpin` manually, so
    // projecting a pinned reference to the field is sound (the standard
    // structural-pinning argument). `scope` is not structurally pinned.
    unsafe {
        let this = this.get_unchecked_mut();
        ScopedProj {
            scope: &this.scope,
            future: Pin::new_unchecked(&mut this.future),
        }
    }
}

/// A handle to a value provided in a [`Scope`]. Dereferences to the value.
/// Cheap to clone.
pub struct Ctx<T: ?Sized>(Arc<T>);

impl<T: ?Sized> Ctx<T> {
    /// Wraps an already shared value (useful in tests and adapters).
    pub fn from_arc(value: Arc<T>) -> Self {
        Ctx(value)
    }

    /// The shared value.
    pub fn arc(this: &Self) -> &Arc<T> {
        &this.0
    }
}

impl<T> Ctx<T> {
    /// Wraps a value.
    pub fn new(value: T) -> Self {
        Ctx(Arc::new(value))
    }
}

impl<T: ?Sized> Clone for Ctx<T> {
    fn clone(&self) -> Self {
        Ctx(Arc::clone(&self.0))
    }
}

impl<T: ?Sized> Deref for Ctx<T> {
    type Target = T;
    fn deref(&self) -> &T {
        &self.0
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for Ctx<T> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_tuple("Ctx").field(&&*self.0).finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[derive(Debug, PartialEq)]
    struct Db(&'static str);
    #[derive(Debug, PartialEq)]
    struct Cache(u32);

    #[test]
    fn nearest_provider_wins_and_siblings_are_isolated() {
        let app = Scope::root();
        app.provide(Db("app"));
        let a = app.child();
        let b = app.child();
        a.provide(Db("a")).provide(Cache(1));
        assert_eq!(*a.get::<Db>().unwrap(), Db("a"));
        assert_eq!(*a.child().get::<Db>().unwrap(), Db("a"));
        assert_eq!(*b.get::<Db>().unwrap(), Db("app"));
        assert!(b.get::<Cache>().is_none());
        assert!(a.provides_locally::<Cache>());
        assert!(!a.child().provides_locally::<Cache>());
    }

    #[test]
    fn require_names_missing_type() {
        let e = Scope::root().require::<Cache>().unwrap_err();
        assert_eq!(e.code(), "internal_error");
        assert!(e.message().contains("Cache"));
    }

    #[test]
    fn dispose_cancels_descendants_and_drops_values() {
        let app = Scope::root();
        let req = app.child();
        req.provide(Cache(7));
        let held = req.get::<Cache>().unwrap();
        let grandchild = req.child();
        req.dispose();
        assert!(req.is_disposed());
        assert!(grandchild.is_disposed());
        assert!(!app.is_disposed());
        assert!(req.get::<Cache>().is_none());
        assert_eq!(*held, Cache(7)); // a held Ctx keeps the value alive
    }

    #[test]
    fn current_scope_nests_and_restores() {
        assert!(Scope::current().is_none());
        let a = Scope::root();
        let b = a.child();
        a.enter(|| {
            assert!(Scope::current().unwrap().ptr_eq(&a));
            b.enter(|| assert!(Scope::current().unwrap().ptr_eq(&b)));
            assert!(Scope::current().unwrap().ptr_eq(&a));
        });
        assert!(Scope::current().is_none());
    }

    #[test]
    fn scoped_future_sees_scope_while_polled() {
        let s = Scope::root();
        s.provide(Cache(3));
        let fut = s.scoped(async {
            Scope::current()
                .and_then(|c| c.get::<Cache>())
                .map(|c| (*c).0)
        });
        let waker = std::task::Waker::noop();
        let mut cx = Context::from_waker(waker);
        let mut fut = std::pin::pin!(fut);
        assert_eq!(fut.as_mut().poll(&mut cx), Poll::Ready(Some(3)));
        assert!(Scope::current().is_none());
    }
}
