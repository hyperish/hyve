//! Hierarchical cooperative cancellation.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::task::{Context, Poll, Waker};

/// A cancellation signal forming a tree: cancelling a token cancels all of its
/// descendants, never its parent. Observable by polling
/// ([`is_cancelled`](Self::is_cancelled)) and by awaiting
/// ([`cancelled`](Self::cancelled)). Cheap to clone; clones share state.
#[derive(Clone)]
pub struct CancellationToken {
    inner: Arc<Inner>,
}

struct Inner {
    cancelled: AtomicBool,
    state: Mutex<State>,
}

#[derive(Default)]
struct State {
    children: Vec<Weak<Inner>>,
    wakers: Vec<Waker>,
}

impl CancellationToken {
    /// A new root token.
    pub fn new() -> Self {
        CancellationToken {
            inner: Arc::new(Inner {
                cancelled: AtomicBool::new(false),
                state: Mutex::new(State::default()),
            }),
        }
    }

    /// A child token, cancelled when `self` is (immediately if it already is).
    pub fn child(&self) -> CancellationToken {
        let child = CancellationToken::new();
        {
            let mut st = lock(&self.inner.state);
            if !self.inner.cancelled.load(Ordering::Acquire) {
                st.children.retain(|w| w.strong_count() > 0);
                st.children.push(Arc::downgrade(&child.inner));
                return child;
            }
        }
        child.cancel();
        child
    }

    /// Cancels this token and all of its descendants. Idempotent.
    pub fn cancel(&self) {
        cancel_inner(&self.inner);
    }

    /// Whether this token has been cancelled.
    pub fn is_cancelled(&self) -> bool {
        self.inner.cancelled.load(Ordering::Acquire)
    }

    /// A future that completes once this token is cancelled.
    pub fn cancelled(&self) -> Cancelled<'_> {
        Cancelled { token: self }
    }
}

fn cancel_inner(inner: &Inner) {
    if inner.cancelled.swap(true, Ordering::AcqRel) {
        return;
    }
    let (children, wakers) = {
        let mut st = lock(&inner.state);
        (
            std::mem::take(&mut st.children),
            std::mem::take(&mut st.wakers),
        )
    };
    for w in wakers {
        w.wake();
    }
    for child in children.iter().filter_map(Weak::upgrade) {
        cancel_inner(&child);
    }
}

fn lock(m: &Mutex<State>) -> std::sync::MutexGuard<'_, State> {
    m.lock().unwrap_or_else(|p| p.into_inner())
}

impl Default for CancellationToken {
    fn default() -> Self {
        Self::new()
    }
}

impl fmt::Debug for CancellationToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CancellationToken")
            .field("cancelled", &self.is_cancelled())
            .finish()
    }
}

/// Future returned by [`CancellationToken::cancelled`].
#[must_use = "futures do nothing unless awaited"]
pub struct Cancelled<'a> {
    token: &'a CancellationToken,
}

impl Future for Cancelled<'_> {
    type Output = ();

    fn poll(self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<()> {
        let inner = &self.token.inner;
        if inner.cancelled.load(Ordering::Acquire) {
            return Poll::Ready(());
        }
        let mut st = lock(&inner.state);
        // Re-check under the lock: `cancel` sets the flag before taking wakers.
        if inner.cancelled.load(Ordering::Acquire) {
            return Poll::Ready(());
        }
        if !st.wakers.iter().any(|w| w.will_wake(cx.waker())) {
            st.wakers.push(cx.waker().clone());
        }
        Poll::Pending
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::AtomicUsize;
    use std::task::Wake;

    struct CountWake(AtomicUsize);
    impl Wake for CountWake {
        fn wake(self: Arc<Self>) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    #[test]
    fn cancel_propagates_down_not_up() {
        let root = CancellationToken::new();
        let child = root.child();
        let grandchild = child.child();
        let sibling = root.child();
        child.cancel();
        assert!(child.is_cancelled());
        assert!(grandchild.is_cancelled());
        assert!(!root.is_cancelled());
        assert!(!sibling.is_cancelled());
        root.cancel();
        assert!(sibling.is_cancelled());
    }

    #[test]
    fn child_of_cancelled_is_cancelled() {
        let root = CancellationToken::new();
        root.cancel();
        assert!(root.child().is_cancelled());
    }

    #[test]
    fn awaiting_wakes_on_cancel() {
        let token = CancellationToken::new();
        let counter = Arc::new(CountWake(AtomicUsize::new(0)));
        let waker = Waker::from(counter.clone());
        let mut cx = Context::from_waker(&waker);
        let child = token.child();
        let mut fut = std::pin::pin!(child.cancelled());
        assert!(fut.as_mut().poll(&mut cx).is_pending());
        assert!(fut.as_mut().poll(&mut cx).is_pending()); // same waker registered once
        token.cancel();
        assert_eq!(counter.0.load(Ordering::SeqCst), 1);
        assert!(fut.as_mut().poll(&mut cx).is_ready());
    }

    #[test]
    fn dropped_children_are_pruned() {
        let root = CancellationToken::new();
        for _ in 0..1000 {
            drop(root.child());
        }
        let _keep = root.child();
        assert!(lock(&root.inner.state).children.len() <= 2);
    }
}
