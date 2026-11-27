//! Port of the graph listener of `NotifyingHelper` (the test utility the Java tests inject with
//! `MemoizingEvaluator.injectGraphTransformerForTesting`): the
//! [`InMemoryGraphImpl`](crate::in_memory_graph::InMemoryGraphImpl) tells the injected
//! [`Listener`]s about calls on its node entries.
//!
//! PORT: Java wraps the graph and every `NodeEntry` it hands out (`NotifyingNodeEntry`); the
//! entries here are not reached through an interface, so the engine calls
//! `notify_graph_listener!` at each place where it makes one of the calls the Java wrapper
//! notifies about. Without an injected listener the macro only reads one flag (and does not
//! evaluate the event's context). A `BEFORE` event is emitted
//! before the node's lock is taken and an `AFTER` event after it is released, as Java's wrapper
//! calls the listener outside the entry's `synchronized` methods. Where the engine makes several
//! calls of one kind under one lock (the signals of a batch of deps), their `BEFORE` events are
//! all emitted before the lock and their `AFTER` events after it. Not emitted: `EVALUATE`,
//! `ADD_EXTERNAL_DEP`, `GET_VALUES`, `GET_TEMPORARY_DIRECT_DEPS`, `GET_VALUE_WITH_METADATA`,
//! `ADD_TEMPORARY_DIRECT_DEPS`, `GET_ALL_DIRECT_DEPS_FOR_INCOMPLETE_NODE`,
//! `RESET_FOR_RESTART_FROM_SCRATCH`, `REMOVE`, `GET_BATCH` with `Reason.DEP_REQUESTED`, and the
//! calls of value injection and error bubbling; no ported test listens for them.

use crate::node_entry::LifecycleState;
use crate::{DirtyType, SkyKey, SkyValue};
use std::sync::Arc;

/// Graph/value entry events that the receiver can be informed of. When writing tests, feel free
/// to add additional events here if needed.
// Java: src/test/java/com/google/devtools/build/skyframe/NotifyingHelper.java:167 NotifyingHelper.EventType
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[allow(dead_code)] // the whole of Java's enum; not every event is emitted (module docs)
pub enum EventType {
    CreateIfAbsent,
    Evaluate,
    AddReverseDep,
    AddExternalDep,
    RemoveReverseDep,
    GetBatch,
    GetValues,
    GetTemporaryDirectDeps,
    Signal,
    SetValue,
    MarkDirty,
    MarkClean,
    IsChanged,
    GetLifecycleState,
    GetValueWithMetadata,
    IsDirty,
    IsReady,
    CheckIfDone,
    AddTemporaryDirectDeps,
    GetAllDirectDepsForIncompleteNode,
    ResetForRestartFromScratch,
    Remove,
}

/// Whether the given event is about to happen or has just happened. For some events, both will
/// be published, for others, only one.
// Java: src/test/java/com/google/devtools/build/skyframe/NotifyingHelper.java:197 NotifyingHelper.Order
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Order {
    Before,
    After,
}

/// Java: `QueryableGraph.Reason`, the reasons the evaluator gives the graph; only the one the
/// emitted `GET_BATCH` event carries.
// Java: com/google/devtools/build/skyframe/QueryableGraph.java:163 QueryableGraph.Reason
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Reason {
    PreOrPostEvaluation,
}

/// A pair of [`DirtyType`] and a bit saying whether the dirtying was successful, emitted to the
/// graph listener as the context [`Order::After`] a call to [`EventType::MarkDirty`] a node.
// Java: src/test/java/com/google/devtools/build/skyframe/NotifyingHelper.java:405 NotifyingHelper.MarkDirtyAfterContext
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MarkDirtyAfterContext {
    pub dirty_type: DirtyType,
    pub actually_dirtied: bool,
}

/// Java: the `@Nullable Object context` of `Listener.accept`.
#[derive(Clone)]
#[allow(dead_code)] // the payloads are for the listeners of tests
pub enum Context {
    /// Java: `null`.
    Null,
    /// A key: the reverse dep (`ADD_REVERSE_DEP`, `REMOVE_REVERSE_DEP`, `CHECK_IF_DONE`) or the
    /// signalling child (`SIGNAL`, `null` there is [`Context::Null`]).
    Key(SkyKey),
    /// The value being set (`SET_VALUE`): Java passes the `SkyValue` (a `ValueWithMetadata` for a
    /// node with an error or events); here the node's plain value, `None` for an error.
    Value(Option<SkyValue>),
    /// `MARK_DIRTY` before.
    DirtyType(DirtyType),
    /// `MARK_DIRTY` after.
    MarkDirtyAfter(MarkDirtyAfterContext),
    /// `GET_LIFECYCLE_STATE` after.
    LifecycleState(LifecycleState),
    /// `GET_BATCH`.
    Reason(Reason),
    /// Java: the `NotifyingNodeEntry` itself (`MARK_CLEAN`, `IS_CHANGED`, `IS_DIRTY`, `IS_READY`,
    /// `GET_LIFECYCLE_STATE` before).
    Entry,
}

impl Context {
    /// The key, if the context is one.
    pub fn key(&self) -> Option<&SkyKey> {
        match self {
            Context::Key(k) => Some(k),
            _ => None,
        }
    }
}

impl std::fmt::Debug for Context {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Context::Null => f.write_str("null"),
            Context::Key(k) => write!(f, "{k:?}"),
            Context::Value(v) => write!(f, "value present: {}", v.is_some()),
            Context::DirtyType(d) => write!(f, "{d:?}"),
            Context::MarkDirtyAfter(c) => write!(f, "{c:?}"),
            Context::LifecycleState(s) => write!(f, "{s:?}"),
            Context::Reason(r) => write!(f, "{r:?}"),
            Context::Entry => f.write_str("<entry>"),
        }
    }
}

/// Receiver to be informed when an event for a given key occurs.
///
/// PORT: Java's `accept` may throw `InterruptedException`; a listener here that wants to see an
/// interrupt polls the thread's interrupt status (`blaze_shell::ThreadInterrupt`) or the
/// evaluator's interrupt flag. A panic in a listener crashes the evaluation (Java's
/// `ErrorRecordingDelegatingListener` turns an exception into an `IllegalStateException`).
// Java: src/test/java/com/google/devtools/build/skyframe/NotifyingHelper.java:203 NotifyingHelper.Listener
pub trait Listener: Send + Sync {
    /// Informs this listener of an event.
    fn accept(&self, key: &SkyKey, event_type: EventType, order: Order, context: &Context);
}

impl<F> Listener for F
where
    F: Fn(&SkyKey, EventType, Order, &Context) + Send + Sync,
{
    fn accept(&self, key: &SkyKey, event_type: EventType, order: Order, context: &Context) {
        self(key, event_type, order, context)
    }
}

/// The listeners injected into one graph, innermost (first injected) first.
///
/// Java: each `injectGraphTransformerForTesting(makeNotifyingTransformer(listener))` wraps the
/// graph once more, so a call is seen by the outermost wrapper first; its `BEFORE` event goes to
/// the last injected listener first and its `AFTER` event to the first injected listener first.
pub(crate) struct GraphListeners {
    listeners: parking_lot::RwLock<Vec<Arc<dyn Listener>>>,
    /// Whether any listener is injected: the one read `notify_graph_listener!` makes otherwise.
    installed: std::sync::atomic::AtomicBool,
}

impl Default for GraphListeners {
    /// A new graph's listeners: those pending for graphs created later
    /// ([`inject_into_new_graphs_for_testing`]).
    fn default() -> Self {
        let listeners = GraphListeners {
            listeners: Default::default(),
            installed: Default::default(),
        };
        for listener in PENDING_FOR_NEW_GRAPHS.read().iter() {
            listeners.inject(listener.clone());
        }
        listeners
    }
}

impl GraphListeners {
    /// Java: `injectGraphTransformerForTesting(NotifyingHelper.makeNotifyingTransformer(listener))`.
    /// PORT: a listener already injected into this graph (the same `Arc`) is not injected again
    /// (a graph reused by a later evaluator keeps its listeners).
    pub(crate) fn inject(&self, listener: Arc<dyn Listener>) {
        let mut listeners = self.listeners.write();
        if listeners.iter().any(|l| Arc::ptr_eq(l, &listener)) {
            return;
        }
        listeners.push(listener);
        self.installed
            .store(true, std::sync::atomic::Ordering::Release);
    }

    /// Whether a listener is injected (`notify_graph_listener!` reads only this otherwise).
    pub(crate) fn is_installed(&self) -> bool {
        self.installed.load(std::sync::atomic::Ordering::Acquire)
    }

    pub(crate) fn notify(
        &self,
        key: &SkyKey,
        event_type: EventType,
        order: Order,
        context: Context,
    ) {
        let listeners: Vec<Arc<dyn Listener>> = self.listeners.read().clone();
        match order {
            Order::Before => {
                for l in listeners.iter().rev() {
                    l.accept(key, event_type, order, &context);
                }
            }
            Order::After => {
                for l in &listeners {
                    l.accept(key, event_type, order, &context);
                }
            }
        }
    }
}

/// Listeners injected into every graph created from now on (a fresh evaluator's), until
/// [`clear_pending_for_new_graphs_for_testing`].
///
/// PORT: Java's server has one evaluator, which `BuildIntegrationTestCase
/// .injectListenerAtStartOfNextBuild` wraps at the start of the next build; Blaze builds an
/// evaluator per executor (over the kept graph when there is one), so a test injects into the
/// kept graph ([`crate::Evaluator::inject_graph_listener_for_testing`]) and here for the graphs
/// the next commands create.
static PENDING_FOR_NEW_GRAPHS: parking_lot::RwLock<Vec<Arc<dyn Listener>>> =
    parking_lot::RwLock::new(Vec::new());

/// Injects `listener` into every graph created from now on.
pub fn inject_into_new_graphs_for_testing(listener: Arc<dyn Listener>) {
    PENDING_FOR_NEW_GRAPHS.write().push(listener);
}

/// Forgets the listeners of [`inject_into_new_graphs_for_testing`] (graphs that have them keep
/// them).
pub fn clear_pending_for_new_graphs_for_testing() {
    PENDING_FOR_NEW_GRAPHS.write().clear();
}

impl crate::Evaluator {
    /// Wraps this evaluator's graph so that `listener` is told about the calls on its node
    /// entries.
    // Java: com/google/devtools/build/skyframe/MemoizingEvaluator.java:171 MemoizingEvaluator.injectGraphTransformerForTesting
    // Java: src/test/java/com/google/devtools/build/skyframe/NotifyingHelper.java:38 NotifyingHelper.makeNotifyingTransformer
    pub fn inject_graph_listener_for_testing(&self, listener: Arc<dyn Listener>) {
        self.graph.listeners_for_testing.inject(listener);
    }
}
