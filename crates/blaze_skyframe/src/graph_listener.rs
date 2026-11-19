//! Test-only port of the graph listener of `NotifyingHelper` (the test utility the Java tests
//! inject with `MemoizingEvaluator.injectGraphTransformerForTesting`): the
//! [`InMemoryGraphImpl`](crate::in_memory_graph::InMemoryGraphImpl) tells the injected
//! [`Listener`]s about calls on its node entries.
//!
//! PORT: Java wraps the graph and every `NodeEntry` it hands out (`NotifyingNodeEntry`); the
//! entries here are not reached through an interface, so the engine calls
//! `notify_graph_listener!` at each place where it makes one of the calls the Java wrapper
//! notifies about. The macro expands to nothing outside `cfg(test)`. A `BEFORE` event is emitted
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
pub(crate) enum EventType {
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
pub(crate) enum Order {
    Before,
    After,
}

/// Java: `QueryableGraph.Reason`, the reasons the evaluator gives the graph; only the one the
/// emitted `GET_BATCH` event carries.
// Java: com/google/devtools/build/skyframe/QueryableGraph.java:163 QueryableGraph.Reason
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub(crate) enum Reason {
    PreOrPostEvaluation,
}

/// A pair of [`DirtyType`] and a bit saying whether the dirtying was successful, emitted to the
/// graph listener as the context [`Order::After`] a call to [`EventType::MarkDirty`] a node.
// Java: src/test/java/com/google/devtools/build/skyframe/NotifyingHelper.java:405 NotifyingHelper.MarkDirtyAfterContext
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct MarkDirtyAfterContext {
    pub(crate) dirty_type: DirtyType,
    pub(crate) actually_dirtied: bool,
}

/// Java: the `@Nullable Object context` of `Listener.accept`.
#[derive(Clone)]
#[allow(dead_code)] // the payloads are for the listeners of tests
pub(crate) enum Context {
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
    pub(crate) fn key(&self) -> Option<&SkyKey> {
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
pub(crate) trait Listener: Send + Sync {
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
#[derive(Default)]
pub(crate) struct GraphListeners {
    listeners: parking_lot::RwLock<Vec<Arc<dyn Listener>>>,
}

impl GraphListeners {
    /// Java: `injectGraphTransformerForTesting(NotifyingHelper.makeNotifyingTransformer(listener))`.
    pub(crate) fn inject(&self, listener: Arc<dyn Listener>) {
        self.listeners.write().push(listener);
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
