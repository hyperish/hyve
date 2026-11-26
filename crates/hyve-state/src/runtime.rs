//! The per-thread reactive runtime: an index arena of nodes plus an owner tree.
//!
//! Algorithm: push-pull with three node states (the scheme `reactive_graph`
//! also uses). Writing a signal marks its direct observers `Dirty` and
//! everything downstream `Check`; effects are queued once. Running an effect
//! first brings each `Check` source up to date (recomputing memos whose inputs
//! changed) and only reruns the effect if something it read actually changed.
//! Each effect therefore runs at most once per change, even through
//! diamond-shaped dependencies.
//!
//! A node is marked clean *before* its function runs, so a write that happens
//! during the run (to something it already read) schedules it again instead of
//! being lost.
//!
//! User code (memo/effect functions, cleanups, `Drop` of released values)
//! never runs while the runtime is borrowed, so it may freely read and write
//! other signals.

use std::any::Any;
use std::cell::{RefCell, RefMut};
use std::collections::VecDeque;
use std::rc::Rc;
use std::sync::atomic::{AtomicU32, Ordering};

/// Upper bound on effect runs in one flush before we report a feedback loop.
const MAX_EFFECT_RUNS_PER_FLUSH: usize = 100_000;

static NEXT_RUNTIME_ID: AtomicU32 = AtomicU32::new(1);

thread_local! {
    static RUNTIME: Runtime = Runtime::new();
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub(crate) enum NodeState {
    Clean,
    Check,
    Dirty,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub(crate) enum Kind {
    Signal,
    Memo,
    Effect,
}

/// Address of a node: arena index + generation + owning runtime.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct NodeId {
    pub(crate) index: u32,
    pub(crate) generation: u32,
    pub(crate) runtime: u32,
}

/// Address of an owner scope.
#[derive(Clone, Copy, PartialEq, Eq, Hash, Debug)]
pub(crate) struct OwnerId {
    index: u32,
    generation: u32,
    runtime: u32,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug, Default)]
struct OwnerRef {
    index: u32,
    generation: u32,
}

type Compute = Rc<dyn Fn() -> bool>;
type Cleanup = Box<dyn FnOnce()>;

/// What a disposal released; handled after the runtime borrow ends because
/// cleanups and `Drop` impls may touch signals.
#[derive(Default)]
struct Released {
    garbage: Vec<Box<dyn Any>>,
    cleanups: Vec<Cleanup>,
}

struct Node {
    generation: u32,
    alive: bool,
    kind: Kind,
    state: NodeState,
    /// `Rc<RefCell<T>>` for signals, `Rc<RefCell<Option<T>>>` for memos.
    value: Option<Rc<dyn Any>>,
    /// Recomputes a memo (returns whether the value changed) or runs an effect.
    compute: Option<Compute>,
    sources: Vec<u32>,
    /// While the node runs: sources of its previous run it has not read
    /// again yet. Their edges stay in place (so a source read again keeps
    /// its edge, and a write's cost does not grow with the number of other
    /// observers) but do not count until read again (`live`); the run
    /// drops the rest when it ends.
    stale: Vec<u32>,
    observers: Vec<u32>,
    owner: u32,
    /// Owner of nodes created while this memo/effect last ran.
    run_owner: Option<OwnerRef>,
}

struct OwnerSlot {
    generation: u32,
    alive: bool,
    parent: Option<u32>,
    children: Vec<u32>,
    nodes: Vec<u32>,
    cleanups: Vec<Cleanup>,
}

impl OwnerSlot {
    fn new(generation: u32, parent: Option<u32>) -> Self {
        OwnerSlot {
            generation,
            alive: true,
            parent,
            children: Vec::new(),
            nodes: Vec::new(),
            cleanups: Vec::new(),
        }
    }
}

struct State {
    nodes: Vec<Node>,
    free_nodes: Vec<u32>,
    owners: Vec<OwnerSlot>,
    free_owners: Vec<u32>,
    observer: Option<u32>,
    owner: OwnerRef,
    pending: VecDeque<u32>,
    batch_depth: u32,
    flushing: bool,
}

impl State {
    fn node_alive(&self, index: u32, generation: u32) -> bool {
        self.nodes
            .get(index as usize)
            .is_some_and(|n| n.alive && n.generation == generation)
    }

    fn owner_alive(&self, o: OwnerRef) -> bool {
        self.owners
            .get(o.index as usize)
            .is_some_and(|s| s.alive && s.generation == o.generation)
    }

    /// The current owner's index; panics if it was disposed.
    fn live_owner(&self) -> u32 {
        if self.owner_alive(self.owner) {
            self.owner.index
        } else {
            panic!(
                "hyve-state: a signal, memo, effect, scope or cleanup was created inside an owner scope that was already disposed"
            )
        }
    }
}

pub(crate) struct Runtime {
    pub(crate) id: u32,
    state: RefCell<State>,
}

impl Runtime {
    fn new() -> Self {
        let state = State {
            nodes: Vec::new(),
            free_nodes: Vec::new(),
            owners: vec![OwnerSlot::new(0, None)],
            free_owners: Vec::new(),
            observer: None,
            owner: OwnerRef::default(),
            pending: VecDeque::new(),
            batch_depth: 0,
            flushing: false,
        };
        Runtime {
            id: NEXT_RUNTIME_ID.fetch_add(1, Ordering::Relaxed),
            state: RefCell::new(state),
        }
    }

    pub(crate) fn with<R>(f: impl FnOnce(&Runtime) -> R) -> R {
        RUNTIME.with(f)
    }

    fn st(&self) -> RefMut<'_, State> {
        self.state
            .try_borrow_mut()
            .expect("hyve-state: reactive runtime re-entered while busy (a hyve-state bug)")
    }

    fn check_thread(&self, runtime: u32) {
        if runtime != self.id {
            panic!(
                "hyve-state: a reactive handle was used on a thread other than the one that created it \
                 (Phase 1 reactive storage is thread-local; keep signals, memos, effects and scopes on their thread)"
            );
        }
    }

    fn check_node(&self, id: NodeId) -> u32 {
        self.check_thread(id.runtime);
        if self.state.borrow().node_alive(id.index, id.generation) {
            id.index
        } else {
            panic!(
                "hyve-state: a signal, memo or effect was used after it (or its owner scope) was disposed"
            )
        }
    }

    pub(crate) fn is_alive(&self, id: NodeId) -> bool {
        id.runtime == self.id && self.state.borrow().node_alive(id.index, id.generation)
    }

    pub(crate) fn observing(&self) -> bool {
        self.state.borrow().observer.is_some()
    }

    /// A foreign shared signal has one dependency node per observing thread.
    pub(crate) fn mirror(&self) -> NodeId {
        let root = OwnerId {
            runtime: self.id,
            index: 0,
            generation: 0,
        };
        self.with_owner(root, || self.create(Kind::Signal, Some(Rc::new(())), None))
    }

    // ----- creation -----

    pub(crate) fn create(
        &self,
        kind: Kind,
        value: Option<Rc<dyn Any>>,
        compute: Option<Compute>,
    ) -> NodeId {
        let mut st = self.st();
        let owner = st.live_owner();
        let state = if kind == Kind::Signal {
            NodeState::Clean
        } else {
            NodeState::Dirty
        };
        let node = |generation| Node {
            generation,
            alive: true,
            kind,
            state,
            value,
            compute,
            sources: Vec::new(),
            stale: Vec::new(),
            observers: Vec::new(),
            owner,
            run_owner: None,
        };
        let index = match st.free_nodes.pop() {
            Some(i) => {
                let generation = st.nodes[i as usize].generation;
                st.nodes[i as usize] = node(generation);
                i
            }
            None => {
                st.nodes.push(node(0));
                (st.nodes.len() - 1) as u32
            }
        };
        st.owners[owner as usize].nodes.push(index);
        NodeId {
            index,
            generation: st.nodes[index as usize].generation,
            runtime: self.id,
        }
    }

    /// Queues a freshly created effect for its first run.
    pub(crate) fn schedule(&self, id: NodeId) {
        let index = self.check_node(id);
        self.st().pending.push_back(index);
    }

    // ----- reads -----

    /// Brings a memo up to date, records a dependency of the current observer
    /// (when `track`), and returns the node's value cell.
    pub(crate) fn read(&self, id: NodeId, track: bool) -> Rc<dyn Any> {
        let index = self.check_node(id);
        if self.state.borrow().nodes[index as usize].kind == Kind::Memo {
            self.update_if_necessary(index, id.generation);
        }
        let mut st = self.st();
        if !st.node_alive(index, id.generation) {
            panic!("hyve-state: a memo was disposed while it was being read");
        }
        if track {
            if let Some(obs) = st.observer {
                if obs != index && !st.nodes[obs as usize].sources.contains(&index) {
                    st.nodes[obs as usize].sources.push(index);
                    // An edge kept from the previous run is already there.
                    if !st.nodes[obs as usize].stale.contains(&index) {
                        st.nodes[index as usize].observers.push(obs);
                    }
                }
            }
        }
        st.nodes[index as usize]
            .value
            .clone()
            .expect("hyve-state: node without a value")
    }

    /// The observers of a node, in order (tests of edge handling).
    #[cfg(test)]
    pub(crate) fn observers_of(&self, id: NodeId) -> Vec<u32> {
        let index = self.check_node(id);
        self.state.borrow().nodes[index as usize].observers.clone()
    }

    // ----- writes -----

    /// Marks observers of a written signal and runs effects (unless batching).
    pub(crate) fn notify(&self, id: NodeId) {
        let index = self.check_node(id);
        {
            let mut st = self.st();
            let observers = st.nodes[index as usize].observers.clone();
            for o in observers {
                if live(&st, index, o) {
                    mark(&mut st, o, NodeState::Dirty);
                }
            }
        }
        self.flush();
    }

    pub(crate) fn batch<R>(&self, f: impl FnOnce() -> R) -> R {
        self.st().batch_depth += 1;
        struct Exit<'a>(&'a Runtime);
        impl Drop for Exit<'_> {
            fn drop(&mut self) {
                self.0.st().batch_depth -= 1;
            }
        }
        let r = {
            let _exit = Exit(self);
            f()
        };
        self.flush();
        r
    }

    fn flush(&self) {
        {
            let mut st = self.st();
            if st.batch_depth > 0 || st.flushing || st.pending.is_empty() {
                return;
            }
            st.flushing = true;
        }
        struct Done<'a>(&'a Runtime);
        impl Drop for Done<'_> {
            fn drop(&mut self) {
                self.0.st().flushing = false;
            }
        }
        let _done = Done(self);
        let mut runs = 0usize;
        loop {
            let next = {
                let mut st = self.st();
                loop {
                    match st.pending.pop_front() {
                        Some(i) if st.nodes[i as usize].alive => {
                            break Some((i, st.nodes[i as usize].generation));
                        }
                        Some(_) => continue,
                        None => break None,
                    }
                }
            };
            let Some((index, generation)) = next else {
                break;
            };
            runs += 1;
            if runs > MAX_EFFECT_RUNS_PER_FLUSH {
                self.st().pending.clear();
                panic!(
                    "hyve-state: effects kept re-triggering each other ({MAX_EFFECT_RUNS_PER_FLUSH} runs in one update); \
                     check for an effect that writes a signal it also reads"
                );
            }
            self.update_if_necessary(index, generation);
        }
    }

    // ----- pull -----

    fn state_of(&self, index: u32, generation: u32) -> Option<NodeState> {
        let st = self.state.borrow();
        st.node_alive(index, generation)
            .then(|| st.nodes[index as usize].state)
    }

    fn update_if_necessary(&self, index: u32, generation: u32) {
        if self.state_of(index, generation) == Some(NodeState::Check) {
            let sources = self.state.borrow().nodes[index as usize].sources.clone();
            for src in sources {
                let memo = {
                    let st = self.state.borrow();
                    st.nodes
                        .get(src as usize)
                        .filter(|n| n.alive && n.kind == Kind::Memo)
                        .map(|n| n.generation)
                };
                if let Some(src_generation) = memo {
                    self.update_if_necessary(src, src_generation);
                }
                if self.state_of(index, generation) != Some(NodeState::Check) {
                    break;
                }
            }
        }
        match self.state_of(index, generation) {
            Some(NodeState::Dirty) => self.recompute(index, generation),
            Some(NodeState::Check) => self.st().nodes[index as usize].state = NodeState::Clean,
            _ => {}
        }
    }

    fn recompute(&self, index: u32, generation: u32) {
        // Dispose what the previous run created, before switching context.
        // The run's scope is emptied in place (not replaced), so it keeps its
        // place among its parent's children.
        let released = {
            let mut st = self.st();
            let mut released = Released::default();
            if let Some(old) = st.nodes[index as usize].run_owner.take() {
                st.nodes[index as usize].run_owner = reset_owner(&mut st, old, &mut released);
            }
            released
        };
        self.release(released);

        let (compute, saved) = {
            let mut st = self.st();
            if !st.node_alive(index, generation) {
                return;
            }
            let parent = st.nodes[index as usize].owner;
            let run_owner = match st.nodes[index as usize].run_owner {
                Some(o) if st.owner_alive(o) => o,
                _ => new_owner_slot(&mut st, parent),
            };
            let node = &mut st.nodes[index as usize];
            node.run_owner = Some(run_owner);
            node.state = NodeState::Clean;
            let compute = node
                .compute
                .clone()
                .expect("hyve-state: computed node without a function");
            // The previous run's edges become stale: the run re-registers
            // what it reads (keeping those edges), the rest go afterwards.
            // The two lists swap, so a re-run allocates nothing for its
            // edges. (A run that panicked left its stale edges; they merge in.)
            if node.stale.is_empty() {
                std::mem::swap(&mut node.sources, &mut node.stale);
            } else {
                let mut previous = std::mem::take(&mut node.sources);
                node.stale.append(&mut previous);
            }
            let saved = Saved {
                observer: st.observer.replace(index),
                owner: std::mem::replace(&mut st.owner, run_owner),
            };
            (compute, saved)
        };

        let changed = {
            let _restore = Restore { rt: self, saved };
            compute()
        };

        let mut st = self.st();
        if !st.node_alive(index, generation) {
            return;
        }
        // Drop the edges to sources this run did not read (the list keeps
        // its capacity for the next run).
        let mut stale = std::mem::take(&mut st.nodes[index as usize].stale);
        for &s in &stale {
            if !st.nodes[index as usize].sources.contains(&s) {
                st.nodes[s as usize].observers.retain(|&o| o != index);
            }
        }
        stale.clear();
        st.nodes[index as usize].stale = stale;
        if changed && st.nodes[index as usize].kind == Kind::Memo {
            let observers = st.nodes[index as usize].observers.clone();
            for o in observers {
                if live(&st, index, o) {
                    mark(&mut st, o, NodeState::Dirty);
                }
            }
        }
    }

    // ----- context -----

    pub(crate) fn untrack<R>(&self, f: impl FnOnce() -> R) -> R {
        let saved = {
            let mut st = self.st();
            Saved {
                observer: st.observer.take(),
                owner: st.owner,
            }
        };
        let _restore = Restore { rt: self, saved };
        f()
    }

    // ----- owners -----

    pub(crate) fn current_owner(&self) -> OwnerId {
        let o = self.state.borrow().owner;
        OwnerId {
            index: o.index,
            generation: o.generation,
            runtime: self.id,
        }
    }

    pub(crate) fn new_owner(&self) -> OwnerId {
        let mut st = self.st();
        let parent = st.live_owner();
        let o = new_owner_slot(&mut st, parent);
        OwnerId {
            index: o.index,
            generation: o.generation,
            runtime: self.id,
        }
    }

    fn owner_ref(&self, id: OwnerId) -> OwnerRef {
        self.check_thread(id.runtime);
        OwnerRef {
            index: id.index,
            generation: id.generation,
        }
    }

    pub(crate) fn owner_alive(&self, id: OwnerId) -> bool {
        id.runtime == self.id
            && self.state.borrow().owner_alive(OwnerRef {
                index: id.index,
                generation: id.generation,
            })
    }

    pub(crate) fn with_owner<R>(&self, id: OwnerId, f: impl FnOnce() -> R) -> R {
        let target = self.owner_ref(id);
        let saved = {
            let mut st = self.st();
            if !st.owner_alive(target) {
                panic!("hyve-state: an owner scope was used after it was disposed");
            }
            Saved {
                observer: st.observer,
                owner: std::mem::replace(&mut st.owner, target),
            }
        };
        let _restore = Restore { rt: self, saved };
        f()
    }

    pub(crate) fn on_cleanup(&self, f: Cleanup) {
        let mut st = self.st();
        let owner = st.live_owner();
        st.owners[owner as usize].cleanups.push(f);
    }

    pub(crate) fn dispose_owner(&self, id: OwnerId) {
        let target = self.owner_ref(id);
        if target.index == 0 {
            panic!("hyve-state: the root owner scope cannot be disposed");
        }
        let released = {
            let mut st = self.st();
            let mut released = Released::default();
            dispose_owner_ref(&mut st, target, &mut released);
            released
        };
        self.release(released);
    }

    pub(crate) fn dispose_node(&self, id: NodeId) {
        if !self.is_alive(id) {
            return;
        }
        let released = {
            let mut st = self.st();
            let owner = st.nodes[id.index as usize].owner;
            st.owners[owner as usize].nodes.retain(|&n| n != id.index);
            let mut released = Released::default();
            dispose_node(&mut st, id.index, &mut released);
            released
        };
        self.release(released);
    }

    /// Runs cleanups (child scopes first, registration order within a scope)
    /// and drops released values, without tracking.
    fn release(&self, released: Released) {
        if released.cleanups.is_empty() && released.garbage.is_empty() {
            return;
        }
        self.untrack(move || {
            for c in released.cleanups {
                c();
            }
            drop(released.garbage);
        });
    }

    #[cfg(test)]
    pub(crate) fn live_nodes(&self) -> usize {
        self.state.borrow().nodes.iter().filter(|n| n.alive).count()
    }

    #[cfg(test)]
    pub(crate) fn live_owners(&self) -> usize {
        self.state
            .borrow()
            .owners
            .iter()
            .filter(|o| o.alive)
            .count()
    }
}

struct Saved {
    observer: Option<u32>,
    owner: OwnerRef,
}

struct Restore<'a> {
    rt: &'a Runtime,
    saved: Saved,
}

impl Drop for Restore<'_> {
    fn drop(&mut self) {
        let mut st = self.rt.st();
        st.observer = self.saved.observer;
        st.owner = self.saved.owner;
    }
}

fn mark(st: &mut State, index: u32, to: NodeState) {
    let node = &mut st.nodes[index as usize];
    if !node.alive || node.state >= to {
        return;
    }
    let was_clean = node.state == NodeState::Clean;
    node.state = to;
    if was_clean {
        if node.kind == Kind::Effect {
            st.pending.push_back(index);
        }
        // A node that was already `Check` has already marked its observers.
        let observers = st.nodes[index as usize].observers.clone();
        for o in observers {
            if live(st, index, o) {
                mark(st, o, NodeState::Check);
            }
        }
    }
}

/// Whether `o` observes `s` now. During `o`'s run, an edge kept from its
/// previous run counts only once the run has read `s` again, as if the
/// run had started without it.
fn live(st: &State, s: u32, o: u32) -> bool {
    let n = &st.nodes[o as usize];
    n.stale.is_empty() || !n.stale.contains(&s) || n.sources.contains(&s)
}

fn new_owner_slot(st: &mut State, parent: u32) -> OwnerRef {
    let index = match st.free_owners.pop() {
        Some(i) => {
            let generation = st.owners[i as usize].generation;
            st.owners[i as usize] = OwnerSlot::new(generation, Some(parent));
            i
        }
        None => {
            st.owners.push(OwnerSlot::new(0, Some(parent)));
            (st.owners.len() - 1) as u32
        }
    };
    st.owners[parent as usize].children.push(index);
    OwnerRef {
        index,
        generation: st.owners[index as usize].generation,
    }
}

/// Empties an owner for a re-run of its node: disposes what it holds and
/// bumps its generation (handles to the previous run's scope are dead), but
/// keeps the slot and its place among its parent's children, which a
/// dispose and a new slot would cost a scan of the siblings for.
fn reset_owner(st: &mut State, o: OwnerRef, released: &mut Released) -> Option<OwnerRef> {
    if !st.owner_alive(o) {
        return None;
    }
    let (children, nodes, cleanups) = {
        let slot = &mut st.owners[o.index as usize];
        slot.generation = slot.generation.wrapping_add(1);
        (
            std::mem::take(&mut slot.children),
            std::mem::take(&mut slot.nodes),
            std::mem::take(&mut slot.cleanups),
        )
    };
    for c in children {
        dispose_owner(st, c, released);
    }
    for n in nodes {
        dispose_node(st, n, released);
    }
    released.cleanups.extend(cleanups);
    Some(OwnerRef {
        index: o.index,
        generation: st.owners[o.index as usize].generation,
    })
}

fn dispose_owner_ref(st: &mut State, o: OwnerRef, released: &mut Released) {
    if st.owner_alive(o) {
        dispose_owner(st, o.index, released);
    }
}

fn dispose_owner(st: &mut State, index: u32, released: &mut Released) {
    let (children, nodes, cleanups, parent) = {
        let o = &mut st.owners[index as usize];
        if !o.alive {
            return;
        }
        o.alive = false;
        o.generation = o.generation.wrapping_add(1);
        (
            std::mem::take(&mut o.children),
            std::mem::take(&mut o.nodes),
            std::mem::take(&mut o.cleanups),
            o.parent,
        )
    };
    for c in children {
        dispose_owner(st, c, released);
    }
    for n in nodes {
        dispose_node(st, n, released);
    }
    released.cleanups.extend(cleanups);
    if let Some(p) = parent {
        st.owners[p as usize].children.retain(|&c| c != index);
    }
    st.free_owners.push(index);
}

fn dispose_node(st: &mut State, index: u32, released: &mut Released) {
    if !st.nodes[index as usize].alive {
        return;
    }
    if let Some(run_owner) = st.nodes[index as usize].run_owner.take() {
        dispose_owner_ref(st, run_owner, released);
    }
    let node = &mut st.nodes[index as usize];
    node.alive = false;
    node.generation = node.generation.wrapping_add(1);
    node.state = NodeState::Clean;
    if let Some(v) = node.value.take() {
        released.garbage.push(Box::new(v));
    }
    if let Some(c) = node.compute.take() {
        released.garbage.push(Box::new(c));
    }
    let mut sources = std::mem::take(&mut node.sources);
    sources.append(&mut node.stale);
    let observers = std::mem::take(&mut node.observers);
    for s in sources {
        st.nodes[s as usize].observers.retain(|&o| o != index);
    }
    for o in observers {
        let n = &mut st.nodes[o as usize];
        n.sources.retain(|&s| s != index);
        n.stale.retain(|&s| s != index);
    }
    st.pending.retain(|&p| p != index);
    st.free_nodes.push(index);
}
