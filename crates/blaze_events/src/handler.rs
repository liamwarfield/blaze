//! Ports of the handler interfaces of `lib/events`: `EventHandler`, `ExtendedEventHandler`
//! (with `Postable` and `FetchProgress`), `Reportable`, `AbstractEventHandler`,
//! `DelegatingEventHandler` and `NullEventHandler`.

use std::any::Any;
use std::fmt;
use std::sync::Arc;

use crate::event::Event;
use crate::event_kind::EventKindSet;

/// An EventHandler handles events, such as errors and warnings. It is a subset of the
/// functionality of the [`crate::Reporter`]. In most cases, you should use this interface instead
/// of the `Reporter` class. Port of `EventHandler`.
///
/// Handlers are shared between threads (Bazel reports from many threads), so `handle` takes
/// `&self` and implementations synchronize internally.
// Java: com/google/devtools/build/lib/events/EventHandler.java:22 EventHandler
pub trait EventHandler: Send + Sync {
    /// Handles an event.
    // Java: com/google/devtools/build/lib/events/EventHandler.java:26 EventHandler.handle
    fn handle(&self, event: &Event);

    /// PORT: Java's `handler instanceof ExtendedEventHandler` (used by `Reporter.post`). Every
    /// implementation of [`ExtendedEventHandler`] must override this to return `Some(self)`.
    fn as_extended_event_handler(&self) -> Option<&dyn ExtendedEventHandler> {
        None
    }
}

/// An event that can be reported to an [`ExtendedEventHandler`]. Port of `Reportable`.
// Java: com/google/devtools/build/lib/events/Reportable.java:20 Reportable
pub trait Reportable {
    // Java: com/google/devtools/build/lib/events/Reportable.java:22 Reportable.reportTo
    fn report_to(&self, handler: &dyn ExtendedEventHandler);

    /// If this event originated from `SkyFunction.Environment.getListener`, whether it should be
    /// stored in the corresponding Skyframe node to be replayed on incremental builds when the
    /// node is deemed up-to-date.
    ///
    /// Events which are crucial to the correctness of the evaluation should return `true` so that
    /// they are replayed when the SkyFunction invocation is cached. On the other hand, events that
    /// are merely informational (such as a progress update) should return `false` to avoid taking
    /// up memory.
    // Java: com/google/devtools/build/lib/events/Reportable.java:54 Reportable.storeForReplay
    fn store_for_replay(&self) -> bool {
        false
    }

    // PORT: `Reportable.withTag` is an inherent method on `Event` (`Event::with_tag`). For a
    // `Postable` it returns the object itself ("No tag-based filtering"), so it has no port.
}

/// An event that can be posted via the extended event handler. Port of
/// `ExtendedEventHandler.Postable`.
///
/// PORT: opaque for now; the concrete build events (and the Guava `EventBus` dispatch on their
/// classes) come with the runtime. Use `(&p as &dyn Any).downcast_ref` to recover the type.
// Java: com/google/devtools/build/lib/events/ExtendedEventHandler.java:35 ExtendedEventHandler.Postable
pub trait Postable: Any + Send + Sync + fmt::Debug {
    /// See [`Reportable::store_for_replay`].
    fn store_for_replay(&self) -> bool {
        false
    }

    /// The posted object as a [`FetchProgress`], when it is one.
    ///
    /// PORT: Guava's `EventBus` delivers an event to the subscribers of every interface its
    /// class implements (`UiEventHandler.downloadProgress(FetchProgress)`); a Rust subscriber
    /// recognises a concrete event by downcasting, and a `FetchProgress` implementor through
    /// this method, which each implementor overrides.
    fn as_fetch_progress(&self) -> Option<&dyn FetchProgress> {
        None
    }
}

impl Reportable for Arc<dyn Postable> {
    // Java: com/google/devtools/build/lib/events/ExtendedEventHandler.java:38 Postable.reportTo
    fn report_to(&self, handler: &dyn ExtendedEventHandler) {
        handler.post(self.clone());
    }

    fn store_for_replay(&self) -> bool {
        Postable::store_for_replay(self.as_ref())
    }
}

/// Replays a sequence of posts on `handler`.
// Java: com/google/devtools/build/lib/events/ExtendedEventHandler.java:48 Postable.replayPostsOn
pub fn replay_posts_on<'a>(
    handler: &dyn ExtendedEventHandler,
    posts: impl IntoIterator<Item = &'a Arc<dyn Postable>>,
) {
    for post in posts {
        handler.post(post.clone());
    }
}

/// Interface for reporting events during the build. It extends the [`EventHandler`] by also
/// allowing posting more structured information. Port of `ExtendedEventHandler`.
// Java: com/google/devtools/build/lib/events/ExtendedEventHandler.java:23 ExtendedEventHandler
pub trait ExtendedEventHandler: EventHandler {
    /// Posts a [`Postable`] object about an important build event.
    // Java: com/google/devtools/build/lib/events/ExtendedEventHandler.java:56 ExtendedEventHandler.post
    fn post(&self, obj: Arc<dyn Postable>);
}

/// A progress event that reports about fetching from a remote site. Port of
/// `ExtendedEventHandler.FetchProgress`.
// Java: com/google/devtools/build/lib/events/ExtendedEventHandler.java:59 ExtendedEventHandler.FetchProgress
pub trait FetchProgress: Postable {
    /// The resource that was originally requested and uniquely determines the fetch source. The
    /// actual fetching may use mirrors, proxies, or similar. The resource need not be an URL, but
    /// it has to uniquely identify the particular fetch among all fetch events.
    fn get_resource_identifier(&self) -> String;

    /// Human readable description of the progress
    fn get_progress(&self) -> String;

    /// Wether the fetch progress reported about is finished already
    fn is_finished(&self) -> bool;
}

/// An ExtendedEventHandler which does nothing. Port of `NullEventHandler` (and of
/// `ExtendedEventHandler.NOOP`, which behaves identically).
// Java: com/google/devtools/build/lib/events/NullEventHandler.java:18 NullEventHandler
#[derive(Debug)]
pub struct NullEventHandler {
    _private: (),
}

impl NullEventHandler {
    // Java: com/google/devtools/build/lib/events/NullEventHandler.java:19 NullEventHandler.INSTANCE
    pub const INSTANCE: &'static NullEventHandler = &NullEventHandler { _private: () };
}

// Java: com/google/devtools/build/lib/events/ExtendedEventHandler.java:25 ExtendedEventHandler.NOOP
pub static NOOP: &dyn ExtendedEventHandler = NullEventHandler::INSTANCE;

impl EventHandler for NullEventHandler {
    // Java: com/google/devtools/build/lib/events/NullEventHandler.java:24 NullEventHandler.handle
    fn handle(&self, _event: &Event) {}

    fn as_extended_event_handler(&self) -> Option<&dyn ExtendedEventHandler> {
        Some(self)
    }
}

impl ExtendedEventHandler for NullEventHandler {
    // Java: com/google/devtools/build/lib/events/NullEventHandler.java:28 NullEventHandler.post
    fn post(&self, _obj: Arc<dyn Postable>) {}
}

/// An abstract event handler that keeps track of the event mask. Events matching the mask will
/// be handled. Port of `AbstractEventHandler`.
///
/// PORT: Java subclasses this; Rust handlers embed it and implement [`EventHandler`] themselves.
// Java: com/google/devtools/build/lib/events/AbstractEventHandler.java:22 AbstractEventHandler
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AbstractEventHandler {
    mask: EventKindSet,
}

impl AbstractEventHandler {
    /// Events matching the mask will be handled.
    // Java: com/google/devtools/build/lib/events/AbstractEventHandler.java:28 AbstractEventHandler.AbstractEventHandler
    pub fn new(mask: EventKindSet) -> Self {
        AbstractEventHandler { mask }
    }

    // Java: com/google/devtools/build/lib/events/AbstractEventHandler.java:32 AbstractEventHandler.getEventMask
    pub fn get_event_mask(&self) -> EventKindSet {
        self.mask
    }
}

/// An EventHandler which delegates to another EventHandler. Primarily useful as a base class for
/// extending behavior. Port of `DelegatingEventHandler`.
// Java: com/google/devtools/build/lib/events/DelegatingEventHandler.java:23 DelegatingEventHandler
pub struct DelegatingEventHandler {
    delegate: Arc<dyn ExtendedEventHandler>,
}

impl DelegatingEventHandler {
    // Java: com/google/devtools/build/lib/events/DelegatingEventHandler.java:26 DelegatingEventHandler.DelegatingEventHandler
    pub fn new(delegate: Arc<dyn ExtendedEventHandler>) -> Self {
        DelegatingEventHandler { delegate }
    }

    pub fn delegate(&self) -> &Arc<dyn ExtendedEventHandler> {
        &self.delegate
    }
}

impl EventHandler for DelegatingEventHandler {
    // Java: com/google/devtools/build/lib/events/DelegatingEventHandler.java:32 DelegatingEventHandler.handle
    fn handle(&self, e: &Event) {
        self.delegate.handle(e);
    }

    fn as_extended_event_handler(&self) -> Option<&dyn ExtendedEventHandler> {
        Some(self)
    }
}

impl ExtendedEventHandler for DelegatingEventHandler {
    // Java: com/google/devtools/build/lib/events/DelegatingEventHandler.java:37 DelegatingEventHandler.post
    fn post(&self, obj: Arc<dyn Postable>) {
        self.delegate.post(obj);
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;
    use crate::event_kind::EventKind;
    use parking_lot::Mutex;

    /// Stands in for Mockito mocks of `EventHandler` / `ExtendedEventHandler` in ported tests.
    #[derive(Default)]
    pub(crate) struct RecordingEventHandler {
        events: Mutex<Vec<Event>>,
        posts: Mutex<Vec<Arc<dyn Postable>>>,
    }

    impl RecordingEventHandler {
        pub(crate) fn events(&self) -> Vec<Event> {
            self.events.lock().clone()
        }
    }

    impl EventHandler for RecordingEventHandler {
        fn handle(&self, event: &Event) {
            self.events.lock().push(event.clone());
        }

        fn as_extended_event_handler(&self) -> Option<&dyn ExtendedEventHandler> {
            Some(self)
        }
    }

    impl ExtendedEventHandler for RecordingEventHandler {
        fn post(&self, obj: Arc<dyn Postable>) {
            self.posts.lock().push(obj);
        }
    }

    // Java: src/test/java/com/google/devtools/build/lib/events/AbstractEventHandlerTest.java AbstractEventHandlerTest.retainsEventMask
    #[test]
    fn retains_event_mask() {
        assert_eq!(
            AbstractEventHandler::new(EventKind::ALL_EVENTS).get_event_mask(),
            EventKind::ALL_EVENTS
        );
        assert_eq!(
            AbstractEventHandler::new(EventKind::ERRORS_AND_WARNINGS).get_event_mask(),
            EventKind::ERRORS_AND_WARNINGS
        );
        assert_eq!(
            AbstractEventHandler::new(EventKind::ERRORS).get_event_mask(),
            EventKind::ERRORS
        );
    }
}
