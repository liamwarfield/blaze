//! Ports of `EventCollector` and `StoredEventHandler`.

use std::fmt;
use std::sync::Arc;

use parking_lot::Mutex;

use crate::event::Event;
use crate::event_kind::{EventKind, EventKindSet};
use crate::handler::{
    AbstractEventHandler, EventHandler, ExtendedEventHandler, Postable, replay_posts_on,
};

/// An [`EventHandler`] that collects all events it encounters, and makes them available. The
/// collected events contain not just the original event information but also the location
/// context. Port of `EventCollector`.
// Java: com/google/devtools/build/lib/events/EventCollector.java:28 EventCollector
pub struct EventCollector {
    base: AbstractEventHandler,
    /// PORT: Java accepts any caller-supplied `Collection<Event>`; Rust shares a vector.
    collected: Arc<Mutex<Vec<Event>>>,
}

impl Default for EventCollector {
    fn default() -> Self {
        Self::new()
    }
}

impl EventCollector {
    /// This collector will collect all events.
    // Java: com/google/devtools/build/lib/events/EventCollector.java:41 EventCollector.EventCollector
    pub fn new() -> Self {
        Self::with_collection(EventKind::ALL_EVENTS, Arc::default())
    }

    /// This collector will collect all events that match the event mask.
    // Java: com/google/devtools/build/lib/events/EventCollector.java:34 EventCollector.EventCollector
    pub fn with_mask(mask: EventKindSet) -> Self {
        Self::with_collection(mask, Arc::default())
    }

    /// This collector will collect all events that match the event mask.
    // Java: com/google/devtools/build/lib/events/EventCollector.java:48 EventCollector.EventCollector
    pub fn with_kinds(mask: &[EventKind]) -> Self {
        Self::with_collection(mask.iter().copied().collect(), Arc::default())
    }

    /// This collector will save the Event instances in the provided collection.
    // Java: com/google/devtools/build/lib/events/EventCollector.java:56 EventCollector.EventCollector
    pub fn with_collection(mask: EventKindSet, collected: Arc<Mutex<Vec<Event>>>) -> Self {
        EventCollector {
            base: AbstractEventHandler::new(mask),
            collected,
        }
    }

    pub fn get_event_mask(&self) -> EventKindSet {
        self.base.get_event_mask()
    }

    /// Returns the collected events. PORT: Java's `iterator()`; returns a snapshot.
    // Java: com/google/devtools/build/lib/events/EventCollector.java:82 EventCollector.iterator
    pub fn iter(&self) -> std::vec::IntoIter<Event> {
        self.collected.lock().clone().into_iter()
    }

    /// Returns the collected events of the given kind. PORT: a snapshot.
    // Java: com/google/devtools/build/lib/events/EventCollector.java:90 EventCollector.filtered
    pub fn filtered(&self, event_kind: EventKind) -> Vec<Event> {
        self.collected
            .lock()
            .iter()
            .filter(|event| event.get_kind() == event_kind)
            .cloned()
            .collect()
    }

    /// Returns the number of events collected.
    // Java: com/google/devtools/build/lib/events/EventCollector.java:97 EventCollector.count
    pub fn count(&self) -> i32 {
        self.collected.lock().len() as i32
    }

    /// Clears the collected events
    // Java: com/google/devtools/build/lib/events/EventCollector.java:104 EventCollector.clear
    pub fn clear(&self) {
        self.collected.lock().clear();
    }
}

impl EventHandler for EventCollector {
    // Java: com/google/devtools/build/lib/events/EventCollector.java:65 EventCollector.handle
    fn handle(&self, event: &Event) {
        if self.get_event_mask().contains(event.get_kind()) {
            self.collected.lock().push(event.clone());
        }
        if let Some(stderr) = event.get_std_err() {
            self.handle(&Event::of_bytes_at(EventKind::Stderr, None, stderr));
        }
        if let Some(stdout) = event.get_std_out() {
            self.handle(&Event::of_bytes_at(EventKind::Stdout, None, stdout));
        }
    }
}

impl IntoIterator for &EventCollector {
    type Item = Event;
    type IntoIter = std::vec::IntoIter<Event>;

    fn into_iter(self) -> Self::IntoIter {
        self.iter()
    }
}

// Java: com/google/devtools/build/lib/events/EventCollector.java:109 EventCollector.toString
impl fmt::Display for EventCollector {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "EventCollector: ")?;
        write_list(f, self.collected.lock().iter())
    }
}

/// Java `AbstractCollection.toString` / `Iterables.toString`: `[a, b]`.
fn write_list<T: fmt::Debug>(
    f: &mut fmt::Formatter<'_>,
    items: impl Iterator<Item = T>,
) -> fmt::Result {
    f.write_str("[")?;
    for (i, item) in items.enumerate() {
        if i > 0 {
            f.write_str(", ")?;
        }
        write!(f, "{item:?}")?;
    }
    f.write_str("]")
}

#[derive(Default)]
struct Stored {
    events: Vec<Event>,
    posts: Vec<Arc<dyn Postable>>,
    has_errors: bool,
}

/// Stores error and warning events, and later replays them. Thread-safe. Port of
/// `StoredEventHandler`.
// Java: com/google/devtools/build/lib/events/StoredEventHandler.java:23 StoredEventHandler
#[derive(Default)]
pub struct StoredEventHandler {
    inner: Mutex<Stored>,
}

impl StoredEventHandler {
    pub fn new() -> Self {
        Self::default()
    }

    // Java: com/google/devtools/build/lib/events/StoredEventHandler.java:29 StoredEventHandler.getEvents
    pub fn get_events(&self) -> Vec<Event> {
        self.inner.lock().events.clone()
    }

    // Java: com/google/devtools/build/lib/events/StoredEventHandler.java:33 StoredEventHandler.getPosts
    pub fn get_posts(&self) -> Vec<Arc<dyn Postable>> {
        self.inner.lock().posts.clone()
    }

    /// Returns true if there are no stored events.
    // Java: com/google/devtools/build/lib/events/StoredEventHandler.java:38 StoredEventHandler.isEmpty
    pub fn is_empty(&self) -> bool {
        let inner = self.inner.lock();
        inner.events.is_empty() && inner.posts.is_empty()
    }

    /// Replay all events stored in this object on the given eventHandler, in the same order.
    // Java: com/google/devtools/build/lib/events/StoredEventHandler.java:55 StoredEventHandler.replayOn
    pub fn replay_on(&self, event_handler: &dyn ExtendedEventHandler) {
        // PORT: Java holds this object's monitor while replaying; replaying onto itself would
        // re-enter. We snapshot under the lock instead, so replaying onto `self` cannot deadlock.
        let (events, posts) = {
            let inner = self.inner.lock();
            (inner.events.clone(), inner.posts.clone())
        };
        Event::replay_events_on(event_handler, &events);
        replay_posts_on(event_handler, &posts);
    }

    /// Returns whether any of the events on this objects were errors.
    // Java: com/google/devtools/build/lib/events/StoredEventHandler.java:63 StoredEventHandler.hasErrors
    pub fn has_errors(&self) -> bool {
        self.inner.lock().has_errors
    }

    // Java: com/google/devtools/build/lib/events/StoredEventHandler.java:67 StoredEventHandler.clear
    pub fn clear(&self) {
        let mut inner = self.inner.lock();
        inner.events.clear();
        inner.posts.clear();
        inner.has_errors = false;
    }
}

impl EventHandler for StoredEventHandler {
    // Java: com/google/devtools/build/lib/events/StoredEventHandler.java:44 StoredEventHandler.handle
    fn handle(&self, e: &Event) {
        let mut inner = self.inner.lock();
        inner.has_errors |= e.get_kind() == EventKind::Error;
        inner.events.push(e.clone());
    }

    fn as_extended_event_handler(&self) -> Option<&dyn ExtendedEventHandler> {
        Some(self)
    }
}

impl ExtendedEventHandler for StoredEventHandler {
    // Java: com/google/devtools/build/lib/events/StoredEventHandler.java:50 StoredEventHandler.post
    fn post(&self, e: Arc<dyn Postable>) {
        self.inner.lock().posts.push(e);
    }
}

// Java: com/google/devtools/build/lib/events/StoredEventHandler.java:74 StoredEventHandler.toString
impl fmt::Display for StoredEventHandler {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let inner = self.inner.lock();
        f.write_str("StoredEventHandler{events=")?;
        write_list(f, inner.events.iter())?;
        f.write_str(", posts=")?;
        write_list(f, inner.posts.iter())?;
        write!(f, ", hasErrors={}}}", inner.has_errors)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::location::Location;

    // Java: src/test/java/com/google/devtools/build/lib/events/EventTestTemplate.java EventTestTemplate.createLocations
    fn template() -> (Event, Location) {
        let message = "This is not an error message.";
        let file = "/path/to/workspace/my/sample/path.txt";
        let location = Location::from_file_line_column(file, 3, 4);
        let event = Event::of_at(EventKind::Warning, Some(location.clone()), message);
        (event, location)
    }

    // Java: src/test/java/com/google/devtools/build/lib/events/EventCollectorTest.java EventCollectorTest.usesPassedInCollection
    #[test]
    fn uses_passed_in_collection() {
        let (event, location) = template();
        let events: Arc<Mutex<Vec<Event>>> = Arc::default();
        let collector = EventCollector::with_collection(EventKind::ALL_EVENTS, events.clone());
        collector.handle(&event);
        let only_event = events.lock()[0].clone();
        assert_eq!(only_event.get_message(), event.get_message());
        assert_eq!(only_event.get_location(), Some(&location));
        assert_eq!(only_event.get_kind(), event.get_kind());
        assert_eq!(only_event.get_location(), event.get_location());
        assert_eq!(collector.count(), 1);
        assert_eq!(events.lock().len(), 1);
    }

    // Java: src/test/java/com/google/devtools/build/lib/events/EventCollectorTest.java EventCollectorTest.collectsEvents
    #[test]
    fn collects_events() {
        let (event, location) = template();
        let collector = EventCollector::new();
        collector.handle(&event);
        let mut collected_event_it = collector.iter();
        let only_event = collected_event_it.next().unwrap();
        assert_eq!(only_event.get_message(), event.get_message());
        assert_eq!(only_event.get_location(), Some(&location));
        assert_eq!(only_event.get_kind(), event.get_kind());
        assert_eq!(only_event.get_location(), event.get_location());
        assert!(collected_event_it.next().is_none());
    }

    // Java: src/test/java/com/google/devtools/build/lib/events/StoredErrorEventHandlerTest.java StoredErrorEventHandlerTest.hasErrors
    #[test]
    fn has_errors() {
        let event_handler = StoredEventHandler::new();
        assert!(!event_handler.has_errors());
        event_handler.handle(&Event::warn("warning"));
        assert!(!event_handler.has_errors());
        event_handler.handle(&Event::info("info"));
        assert!(!event_handler.has_errors());
        event_handler.handle(&Event::error("error"));
        assert!(event_handler.has_errors());
    }

    // Java: src/test/java/com/google/devtools/build/lib/events/StoredErrorEventHandlerTest.java StoredErrorEventHandlerTest.replayOnWithoutEvents
    #[test]
    fn replay_on_without_events() {
        let event_handler = StoredEventHandler::new();
        let sink = StoredEventHandler::new();

        event_handler.replay_on(&sink);
        assert!(sink.is_empty());
    }

    // Java: src/test/java/com/google/devtools/build/lib/events/StoredErrorEventHandlerTest.java StoredErrorEventHandlerTest.replayOn
    #[test]
    fn replay_on() {
        let event_handler = StoredEventHandler::new();
        let sink = StoredEventHandler::new();

        let events = vec![
            Event::warn("a"),
            Event::error("b"),
            Event::info("c"),
            Event::warn("d"),
        ];
        for e in &events {
            event_handler.handle(e);
        }

        event_handler.replay_on(&sink);
        assert_eq!(sink.get_events(), events);
    }
}
