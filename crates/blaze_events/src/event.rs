//! Port of `com.google.devtools.build.lib.events.Event`.

use std::any::{Any, TypeId};
use std::borrow::Cow;
use std::collections::hash_map::DefaultHasher;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::io;
use std::sync::Arc;

use crate::event_kind::EventKind;
use crate::handler::{EventHandler, ExtendedEventHandler, Reportable};
use crate::location::Location;

/// A value that can be attached to an [`Event`] as a property.
///
/// PORT: Java keys event properties by `Class<T>` in an `ImmutableClassToInstanceMap`. Rust keys
/// them by `TypeId`; any `Send + Sync + Debug + PartialEq + Hash` type is a property type (the
/// blanket impl below), matching Java's `equals`/`hashCode` on the values.
pub trait EventProperty: Any + Send + Sync + fmt::Debug {
    /// `equals` against another property value of any type.
    fn dyn_eq(&self, other: &dyn EventProperty) -> bool;
    /// `hashCode` of this value.
    fn dyn_hash(&self, state: &mut dyn Hasher);
}

impl<T: Any + Send + Sync + fmt::Debug + PartialEq + Hash> EventProperty for T {
    fn dyn_eq(&self, other: &dyn EventProperty) -> bool {
        let other: &dyn Any = other;
        other.downcast_ref::<T>() == Some(self)
    }

    fn dyn_hash(&self, mut state: &mut dyn Hasher) {
        self.hash(&mut state);
    }
}

#[derive(Clone)]
struct PropertyEntry {
    type_id: TypeId,
    /// Sort key. PORT: Java orders entries by `Class.getName`; Rust uses `type_name`. The order
    /// is not observable (equality and hashing are order-independent, as for Java maps).
    type_name: &'static str,
    value: Arc<dyn EventProperty>,
}

impl PropertyEntry {
    fn new<T: EventProperty>(value: T) -> Self {
        PropertyEntry {
            type_id: TypeId::of::<T>(),
            type_name: std::any::type_name::<T>(),
            value: Arc::new(value),
        }
    }
}

/// Message storage: Java keeps either a `String` or the UTF-8 `byte[]` of a message, and the two
/// forms are never equal to each other.
// Java: com/google/devtools/build/lib/events/Event.java:51 Event.message
#[derive(Clone, PartialEq, Eq, Hash)]
enum Message {
    Str(Arc<str>),
    Bytes(Arc<[u8]>),
}

/// A situation encountered by the build system that's worth reporting. Port of `Event`.
///
/// An event specifies an [`EventKind`], a message, and (optionally) additional properties.
/// Cloning is cheap (shared, immutable contents).
// Java: com/google/devtools/build/lib/events/Event.java:41 Event
#[derive(Clone)]
pub struct Event {
    kind: EventKind,
    message: Message,
    /// Ordered by type name (see [`PropertyEntry::type_name`]).
    properties: Arc<[PropertyEntry]>,
}

/// Process output associated with an event. The contents is just-about-certainly on disk, so
/// special care should be taken when accessing it. Port of `Event.ProcessOutput`.
// Java: com/google/devtools/build/lib/events/Event.java:437 Event.ProcessOutput
pub trait ProcessOutput: Send + Sync {
    /// Returns the string representation of the path containing the process's stdout for
    /// logging/debugging purposes.
    fn get_std_out_path(&self) -> String;
    fn get_std_out_size(&self) -> io::Result<i64>;
    fn get_std_out(&self) -> Vec<u8>;
    /// Returns the string representation of the path containing the process's stderr for
    /// logging/debugging purposes.
    fn get_std_err_path(&self) -> String;
    fn get_std_err_size(&self) -> io::Result<i64>;
    fn get_std_err(&self) -> Vec<u8>;
}

/// The event property holding a [`ProcessOutput`].
///
/// PORT: Java stores the `ProcessOutput` instance under `ProcessOutput.class`; implementations
/// do not override `equals`, so equality is identity. This newtype gives the trait object that
/// identity equality so it can be a property.
#[derive(Clone)]
pub struct ProcessOutputProperty(pub Arc<dyn ProcessOutput>);

impl PartialEq for ProcessOutputProperty {
    fn eq(&self, other: &Self) -> bool {
        std::ptr::addr_eq(Arc::as_ptr(&self.0), Arc::as_ptr(&other.0))
    }
}

impl Eq for ProcessOutputProperty {}

impl Hash for ProcessOutputProperty {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (Arc::as_ptr(&self.0) as *const () as usize).hash(state);
    }
}

impl fmt::Debug for ProcessOutputProperty {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ProcessOutput")
            .field("stdout", &self.0.get_std_out_path())
            .field("stderr", &self.0.get_std_err_path())
            .finish()
    }
}

impl Event {
    // Java: com/google/devtools/build/lib/events/Event.java:63 Event.Event
    fn new(kind: EventKind, message: Message, properties: Arc<[PropertyEntry]>) -> Self {
        Event {
            kind,
            message,
            properties,
        }
    }

    // Java: com/google/devtools/build/lib/events/Event.java:82 Event.getKind
    pub fn get_kind(&self) -> EventKind {
        self.kind
    }

    /// Returns the message. A byte-array message is decoded as UTF-8.
    // Java: com/google/devtools/build/lib/events/Event.java:86 Event.getMessage
    pub fn get_message(&self) -> Cow<'_, str> {
        match &self.message {
            Message::Str(s) => Cow::Borrowed(s),
            // PORT: Java's `new String(bytes, UTF_8)` replaces malformed input with U+FFFD, as
            // `from_utf8_lossy` does.
            Message::Bytes(b) => String::from_utf8_lossy(b),
        }
    }

    /// Returns this event's message as bytes. If this event was instantiated using a string, the
    /// returned bytes are its UTF-8 encoding.
    // Java: com/google/devtools/build/lib/events/Event.java:95 Event.getMessageBytes
    pub fn get_message_bytes(&self) -> &[u8] {
        match &self.message {
            Message::Str(s) => s.as_bytes(),
            Message::Bytes(b) => b,
        }
    }

    /// Returns the property value associated with `T` if any, and `None` otherwise.
    // Java: com/google/devtools/build/lib/events/Event.java:101 Event.getProperty
    pub fn get_property<T: EventProperty>(&self) -> Option<&T> {
        let id = TypeId::of::<T>();
        self.properties
            .iter()
            .find(|e| e.type_id == id)
            .and_then(|e| (e.value.as_ref() as &dyn Any).downcast_ref::<T>())
    }

    /// Returns an event that has the same type, message, and properties as this one, and
    /// additionally associates `property_value` (if `Some`) with `T`; if `property_value` is
    /// `None`, the returned event has no property associated with `T`.
    ///
    /// If this event has no property associated with `T` and `property_value` is `None`, this
    /// returns (a clone of) this event. In any case, this event does not change.
    // Java: com/google/devtools/build/lib/events/Event.java:126 Event.withProperty
    pub fn with_property<T: EventProperty>(&self, property_value: Option<T>) -> Event {
        let id = TypeId::of::<T>();
        let contains_key = self.properties.iter().any(|e| e.type_id == id);
        let new_properties: Vec<PropertyEntry> = match property_value {
            Some(value) if !contains_key => {
                let mut entries: Vec<PropertyEntry> = self.properties.to_vec();
                entries.push(PropertyEntry::new(value));
                entries.sort_by(|a, b| a.type_name.cmp(b.type_name));
                entries
            }
            Some(value) => {
                let mut replacement = Some(PropertyEntry::new(value));
                self.properties
                    .iter()
                    .map(|e| {
                        if e.type_id == id {
                            replacement.take().expect("type ids are unique")
                        } else {
                            e.clone()
                        }
                    })
                    .collect()
            }
            None if contains_key => self
                .properties
                .iter()
                .filter(|e| e.type_id != id)
                .cloned()
                .collect(),
            // !containsKey and propertyValue is null, so there's nothing to change.
            None => return self.clone(),
        };
        Event::new(self.kind, self.message.clone(), new_properties.into())
    }

    /// Behaves like [`Event::with_property`] with `T = String`. Additionally, if this event
    /// already has a `String` property with value `tag`, or if `tag` is `None` and the event has
    /// no `String` property, then this returns (a clone of) this event.
    // Java: com/google/devtools/build/lib/events/Event.java:176 Event.withTag
    pub fn with_tag(&self, tag: Option<&str>) -> Event {
        if tag == self.get_tag() {
            return self.clone();
        }
        self.with_property::<String>(tag.map(str::to_owned))
    }

    /// Returns a new event with the provided [`ProcessOutput`] property.
    // Java: com/google/devtools/build/lib/events/Event.java:187 Event.withProcessOutput
    pub fn with_process_output(&self, process_output: Arc<dyn ProcessOutput>) -> Event {
        self.with_property(Some(ProcessOutputProperty(process_output)))
    }

    /// Returns the `String` property, if any, associated with the event. When present, this value
    /// typically describes some property of the action that generated the event.
    // Java: com/google/devtools/build/lib/events/Event.java:198 Event.getTag
    pub fn get_tag(&self) -> Option<&str> {
        self.get_property::<String>().map(String::as_str)
    }

    // Java: com/google/devtools/build/lib/events/Event.java:203 Event.getProcessOutput
    pub fn get_process_output(&self) -> Option<&Arc<dyn ProcessOutput>> {
        self.get_property::<ProcessOutputProperty>().map(|p| &p.0)
    }

    /// Returns the stdout bytes associated with this event if any, and `None` otherwise.
    // Java: com/google/devtools/build/lib/events/Event.java:209 Event.getStdOut
    pub fn get_std_out(&self) -> Option<Vec<u8>> {
        self.get_process_output().map(|p| p.get_std_out())
    }

    /// Returns the stderr bytes associated with this event if any, and `None` otherwise.
    // Java: com/google/devtools/build/lib/events/Event.java:219 Event.getStdErr
    pub fn get_std_err(&self) -> Option<Vec<u8>> {
        self.get_process_output().map(|p| p.get_std_err())
    }

    /// Returns the location of this event, if any. Returns `None` iff the event wasn't associated
    /// with any particular location, for example, a progress message.
    // Java: com/google/devtools/build/lib/events/Event.java:232 Event.getLocation
    pub fn get_location(&self) -> Option<&Location> {
        self.get_property::<Location>()
    }

    /// Constructs an event with the provided kind and string message.
    // Java: com/google/devtools/build/lib/events/Event.java:289 Event.of
    pub fn of(kind: EventKind, message: impl Into<String>) -> Event {
        Event::new(kind, Message::Str(message.into().into()), Arc::new([]))
    }

    /// Constructs an event with the provided kind, string message, and single property value.
    // Java: com/google/devtools/build/lib/events/Event.java:299 Event.of
    pub fn of_with_property<T: EventProperty>(
        kind: EventKind,
        message: impl Into<String>,
        property_value: T,
    ) -> Event {
        Event::new(
            kind,
            Message::Str(message.into().into()),
            Arc::new([PropertyEntry::new(property_value)]),
        )
    }

    /// Constructs an event with the provided kind and byte message.
    // Java: com/google/devtools/build/lib/events/Event.java:305 Event.of
    pub fn of_bytes(kind: EventKind, message_bytes: impl Into<Vec<u8>>) -> Event {
        Event::new(
            kind,
            Message::Bytes(message_bytes.into().into()),
            Arc::new([]),
        )
    }

    /// Constructs an event with the provided kind, byte message, and single property value.
    // Java: com/google/devtools/build/lib/events/Event.java:315 Event.of
    pub fn of_bytes_with_property<T: EventProperty>(
        kind: EventKind,
        message_bytes: impl Into<Vec<u8>>,
        property_value: T,
    ) -> Event {
        Event::new(
            kind,
            Message::Bytes(message_bytes.into().into()),
            Arc::new([PropertyEntry::new(property_value)]),
        )
    }

    /// Constructs an event with the provided kind and string message, with an optional location.
    // Java: com/google/devtools/build/lib/events/Event.java:325 Event.of
    pub fn of_at(kind: EventKind, location: Option<Location>, message: impl Into<String>) -> Event {
        match location {
            None => Event::of(kind, message),
            Some(location) => Event::of_with_property(kind, message, location),
        }
    }

    /// Constructs an event with a byte message instead of a string. The bytes must be decodable
    /// as UTF-8 text.
    // Java: com/google/devtools/build/lib/events/Event.java:334 Event.of
    pub fn of_bytes_at(
        kind: EventKind,
        location: Option<Location>,
        message_bytes: impl Into<Vec<u8>>,
    ) -> Event {
        match location {
            None => Event::of_bytes(kind, message_bytes),
            Some(location) => Event::of_bytes_with_property(kind, message_bytes, location),
        }
    }

    /// Constructs an event with kind [`EventKind::Fatal`].
    // Java: com/google/devtools/build/lib/events/Event.java:341 Event.fatal
    pub fn fatal(message: impl Into<String>) -> Event {
        Event::of(EventKind::Fatal, message)
    }

    /// Constructs an event with kind [`EventKind::Error`], with an optional location.
    // Java: com/google/devtools/build/lib/events/Event.java:346 Event.error
    pub fn error_at(location: Option<Location>, message: impl Into<String>) -> Event {
        Event::of_at(EventKind::Error, location, message)
    }

    /// Constructs an event with kind [`EventKind::Error`].
    // Java: com/google/devtools/build/lib/events/Event.java:353 Event.error
    pub fn error(message: impl Into<String>) -> Event {
        Event::of(EventKind::Error, message)
    }

    /// Constructs an event with kind [`EventKind::Warning`], with an optional location.
    // Java: com/google/devtools/build/lib/events/Event.java:358 Event.warn
    pub fn warn_at(location: Option<Location>, message: impl Into<String>) -> Event {
        Event::of_at(EventKind::Warning, location, message)
    }

    /// Constructs an event with kind [`EventKind::Warning`].
    // Java: com/google/devtools/build/lib/events/Event.java:365 Event.warn
    pub fn warn(message: impl Into<String>) -> Event {
        Event::of(EventKind::Warning, message)
    }

    /// Constructs an event with kind [`EventKind::Info`], with an optional location.
    // Java: com/google/devtools/build/lib/events/Event.java:370 Event.info
    pub fn info_at(location: Option<Location>, message: impl Into<String>) -> Event {
        Event::of_at(EventKind::Info, location, message)
    }

    /// Constructs an event with kind [`EventKind::Info`].
    // Java: com/google/devtools/build/lib/events/Event.java:377 Event.info
    pub fn info(message: impl Into<String>) -> Event {
        Event::of(EventKind::Info, message)
    }

    /// Constructs an event with kind [`EventKind::Progress`], with an optional location.
    // Java: com/google/devtools/build/lib/events/Event.java:384 Event.progress
    pub fn progress_at(location: Option<Location>, message: impl Into<String>) -> Event {
        Event::of_at(EventKind::Progress, location, message)
    }

    /// Constructs an event with kind [`EventKind::Progress`].
    // Java: com/google/devtools/build/lib/events/Event.java:391 Event.progress
    pub fn progress(message: impl Into<String>) -> Event {
        Event::of(EventKind::Progress, message)
    }

    /// Constructs an event with kind [`EventKind::Debug`], with an optional location.
    // Java: com/google/devtools/build/lib/events/Event.java:396 Event.debug
    pub fn debug_at(location: Option<Location>, message: impl Into<String>) -> Event {
        Event::of_at(EventKind::Debug, location, message)
    }

    /// Constructs an event with kind [`EventKind::Debug`].
    // Java: com/google/devtools/build/lib/events/Event.java:403 Event.debug
    pub fn debug(message: impl Into<String>) -> Event {
        Event::of(EventKind::Debug, message)
    }

    /// Replays a sequence of events on `handler`.
    // Java: com/google/devtools/build/lib/events/Event.java:408 Event.replayEventsOn
    pub fn replay_events_on<'a>(
        handler: &dyn EventHandler,
        events: impl IntoIterator<Item = &'a Event>,
    ) {
        for event in events {
            handler.handle(event);
        }
    }

    /// Converts a list of syntax errors to events and replays them on `handler`.
    ///
    /// PORT: Java takes `List<SyntaxError>` from the Starlark interpreter; Blaze's interpreter is
    /// starlark-rust, so callers pass each error's `(location, message)`.
    // Java: com/google/devtools/build/lib/events/Event.java:415 Event.replayEventsOn
    pub fn replay_syntax_errors_on(
        handler: &dyn EventHandler,
        errors: impl IntoIterator<Item = (Location, String)>,
    ) {
        for (location, message) in errors {
            handler.handle(&Event::error_at(Some(location), message));
        }
    }

    // Java: com/google/devtools/build/lib/events/Event.java:425 Event.makeDebugPrintHandler
    // PORT: not ported here. It adapts StarlarkThread.PrintHandler (which supplies the caller
    // location); the starlark-rust integration crate owns that adapter and must emit
    // `Event::debug_at(Some(caller_location), msg)`.
}

impl Reportable for Event {
    // Java: com/google/devtools/build/lib/events/Event.java:70 Event.reportTo
    fn report_to(&self, handler: &dyn ExtendedEventHandler) {
        handler.handle(self);
    }

    // Java: com/google/devtools/build/lib/events/Event.java:75 Event.storeForReplay
    fn store_for_replay(&self) -> bool {
        self.kind != EventKind::Progress
            && self.kind != EventKind::Warning
            && self.kind != EventKind::Info
            && self.kind != EventKind::Debug
    }
}

/// Returns the event formatted as `"ERROR foo.bzl:1:2: oops"`.
// Java: com/google/devtools/build/lib/events/Event.java:238 Event.toString
impl fmt::Display for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // TODO(adonovan): <no location> is just noise.
        write!(f, "{} ", self.kind)?;
        match self.get_location() {
            Some(location) => write!(f, "{location}")?,
            None => f.write_str("<no location>")?,
        }
        write!(f, ": {}", self.get_message())
    }
}

impl fmt::Debug for Event {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

// Java: com/google/devtools/build/lib/events/Event.java:272 Event.equals
impl PartialEq for Event {
    fn eq(&self, that: &Self) -> bool {
        if self.kind != that.kind || self.message != that.message {
            return false;
        }
        // Map equality: same keys, equal values, order-independent.
        self.properties.len() == that.properties.len()
            && self.properties.iter().all(|e| {
                that.properties
                    .iter()
                    .find(|o| o.type_id == e.type_id)
                    .is_some_and(|o| e.value.dyn_eq(o.value.as_ref()))
            })
    }
}

impl Eq for Event {}

// Java: com/google/devtools/build/lib/events/Event.java:249 Event.hashCode
impl Hash for Event {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.kind.hash(state);
        self.message.hash(state);
        // Java's map hashCode is the sum of its entries' hashes, so it does not depend on order.
        let mut sum: u64 = 0;
        for e in self.properties.iter() {
            let mut h = DefaultHasher::new();
            e.type_id.hash(&mut h);
            e.value.dyn_hash(&mut h);
            sum = sum.wrapping_add(h.finish());
        }
        sum.hash(state);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handler::tests::RecordingEventHandler;

    fn hash_of(e: &Event) -> u64 {
        let mut h = DefaultHasher::new();
        e.hash(&mut h);
        h.finish()
    }

    // Java: src/test/java/com/google/devtools/build/lib/events/EventTest.java EventTest.eventKindMessage
    #[test]
    fn event_kind_message() {
        let event = Event::of(EventKind::Warning, "myMessage");
        assert_eq!(event.get_message(), "myMessage");
        assert_eq!(event.get_kind(), EventKind::Warning);
    }

    // Java: src/test/java/com/google/devtools/build/lib/events/EventTest.java EventTest.eventMessageEncoding
    #[test]
    fn event_message_encoding() {
        // Java "ἳf" is U+1F33 followed by 'f'.
        let message = "Bazel \u{1f33}f";

        let string_event = Event::of(EventKind::Warning, message);
        let string_event2 = Event::of(EventKind::Warning, "Bazel \u{1f33}f");
        assert_eq!(string_event.get_message(), message);
        assert_eq!(string_event.get_message_bytes(), message.as_bytes());

        let byte_array_event = Event::of_bytes(EventKind::Warning, message.as_bytes());
        let byte_array_event2 = Event::of_bytes(EventKind::Warning, "Bazel \u{1f33}f".as_bytes());
        assert_eq!(byte_array_event.get_message(), message);
        assert_eq!(byte_array_event.get_message_bytes(), message.as_bytes());

        // EqualsTester: two groups, equal within, unequal across, consistent hashes.
        assert_eq!(string_event, string_event2);
        assert_eq!(hash_of(&string_event), hash_of(&string_event2));
        assert_eq!(byte_array_event, byte_array_event2);
        assert_eq!(hash_of(&byte_array_event), hash_of(&byte_array_event2));
        assert_ne!(string_event, byte_array_event);
        assert_ne!(byte_array_event2, string_event2);
    }

    // Java: src/test/java/com/google/devtools/build/lib/events/EventTest.java EventTest.eventLocationSensitiveToString
    #[test]
    fn event_location_sensitive_to_string() {
        let file = "/path/to/workspace/my/sample/path.txt";
        let location = Location::from_file_line_column(file, 3, 4);
        let event = Event::of_with_property(EventKind::Warning, "myMessage", location.clone());

        assert_eq!(event.get_location(), Some(&location));
        assert_eq!(event.to_string(), format!("WARNING {file}:3:4: myMessage"));
    }

    // Java: src/test/java/com/google/devtools/build/lib/events/EventTest.java EventTest.messageReference
    #[test]
    fn message_reference() {
        let message_bytes = b"message".to_vec();
        let event = Event::of_bytes(EventKind::Warning, message_bytes.clone());
        assert_eq!(event.get_message_bytes(), &message_bytes[..]);
    }

    #[derive(Debug, PartialEq, Hash)]
    struct Object;

    // Java: src/test/java/com/google/devtools/build/lib/events/EventTest.java EventTest.noProperties
    #[test]
    fn no_properties() {
        let event = Event::of(EventKind::Warning, "myMessage");
        assert!(event.get_property::<Object>().is_none());
        assert_eq!(event, Event::of(EventKind::Warning, "myMessage"));
    }

    // Java: src/test/java/com/google/devtools/build/lib/events/EventTest.java EventTest.oneProperty
    #[test]
    fn one_property() {
        let event =
            Event::of_with_property(EventKind::Warning, "myMessage", "myProperty".to_string());
        assert!(event.get_property::<Object>().is_none());
        assert_eq!(
            event.get_property::<String>().map(String::as_str),
            Some("myProperty")
        );
        assert_eq!(
            event,
            Event::of_with_property(EventKind::Warning, "myMessage", "myProperty".to_string())
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/events/EventTest.java EventTest.withAddedProperty
    #[test]
    fn with_added_property() {
        let event =
            Event::of_with_property(EventKind::Warning, "myMessage", "myProperty".to_string());
        let location = Location::from_file_line_column("file", 1, 2);
        let two_property_event = event.with_property(Some(location.clone()));

        assert_ne!(event, two_property_event);
        assert_eq!(event.get_tag(), Some("myProperty"));
        assert!(event.get_property::<Location>().is_none());
        assert_eq!(two_property_event.get_tag(), Some("myProperty"));
        assert_eq!(
            two_property_event.get_property::<Location>(),
            Some(&location)
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/events/EventTest.java EventTest.withReplacedProperty
    #[test]
    fn with_replaced_property() {
        let location = Location::from_file_line_column("file", 1, 2);
        let event = Event::of(EventKind::Warning, "myMessage")
            .with_property(Some("myProperty".to_string()))
            .with_property(Some(location.clone()));
        let replaced_property_event = event.with_property(Some("yourProperty".to_string()));

        assert_ne!(event, replaced_property_event);
        assert_eq!(event.get_tag(), Some("myProperty"));
        assert_eq!(event.get_property::<Location>(), Some(&location));
        assert_eq!(replaced_property_event.get_tag(), Some("yourProperty"));
        assert_eq!(
            replaced_property_event.get_property::<Location>(),
            Some(&location)
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/events/EventTest.java EventTest.withRemovedProperty
    #[test]
    fn with_removed_property() {
        let location = Location::from_file_line_column("file", 1, 2);
        let event = Event::of(EventKind::Warning, "myMessage")
            .with_property(Some("myProperty".to_string()))
            .with_property(Some(location.clone()));
        let removed_property_event = event.with_property::<Location>(None);

        assert_ne!(event, removed_property_event);
        assert_eq!(event.get_tag(), Some("myProperty"));
        assert_eq!(event.get_property::<Location>(), Some(&location));
        assert_eq!(removed_property_event.get_tag(), Some("myProperty"));
        assert!(removed_property_event.get_property::<Location>().is_none());
        // PORT: Java asserts the same instance; Rust returns an equal clone sharing contents.
        let again = removed_property_event.with_property::<Location>(None);
        assert_eq!(again, removed_property_event);
        assert!(Arc::ptr_eq(
            &again.properties,
            &removed_property_event.properties
        ));
    }

    // Java: src/test/java/com/google/devtools/build/lib/events/EventTest.java EventTest.propertyOrderAgnostic
    #[test]
    fn property_order_agnostic() {
        let location = Location::from_file_line_column("file", 1, 2);
        let string_first_event = Event::of(EventKind::Warning, "myMessage")
            .with_property(Some("myProperty".to_string()))
            .with_property(Some(location.clone()));
        let location_first_event = Event::of(EventKind::Warning, "myMessage")
            .with_property(Some(location))
            .with_property(Some("myProperty".to_string()));
        assert_eq!(string_first_event, location_first_event);
        assert_eq!(hash_of(&string_first_event), hash_of(&location_first_event));
    }

    // Java: src/test/java/com/google/devtools/build/lib/events/EventTest.java EventTest.withTag
    #[test]
    fn with_tag() {
        let event = Event::of(EventKind::Warning, "myMessage").with_tag(Some("myTag"));
        assert_eq!(event.get_tag(), Some("myTag"));
        let same = event.with_tag(Some("myTag"));
        assert!(Arc::ptr_eq(&same.properties, &event.properties));

        let without_tag = event.with_tag(None);
        assert_eq!(without_tag.get_tag(), None);
        let same = without_tag.with_tag(None);
        assert!(Arc::ptr_eq(&same.properties, &without_tag.properties));
    }

    // Java: src/test/java/com/google/devtools/build/lib/events/EventTest.java EventTest.tagIsSameAsStringProperty
    #[test]
    fn tag_is_same_as_string_property() {
        assert_eq!(
            Event::of_with_property(EventKind::Warning, "myMessage", "myProperty".to_string())
                .get_tag(),
            Some("myProperty")
        );
    }

    struct TestProcessOutput {
        stdout: Vec<u8>,
        stderr: Vec<u8>,
    }

    impl ProcessOutput for TestProcessOutput {
        fn get_std_out_path(&self) -> String {
            "/stdout".to_string()
        }
        fn get_std_out_size(&self) -> io::Result<i64> {
            Ok(self.stdout.len() as i64)
        }
        fn get_std_out(&self) -> Vec<u8> {
            self.stdout.clone()
        }
        fn get_std_err_path(&self) -> String {
            "/stderr".to_string()
        }
        fn get_std_err_size(&self) -> io::Result<i64> {
            Ok(self.stderr.len() as i64)
        }
        fn get_std_err(&self) -> Vec<u8> {
            self.stderr.clone()
        }
    }

    // Java: src/test/java/com/google/devtools/build/lib/events/EventTest.java EventTest.testWithProcessOutput
    #[test]
    fn test_with_process_output() {
        let stdout = b"some stdout output".to_vec();
        let stderr = b"some stderr error".to_vec();
        let test_process_output: Arc<dyn ProcessOutput> = Arc::new(TestProcessOutput {
            stdout: stdout.clone(),
            stderr: stderr.clone(),
        });

        let event = Event::of(EventKind::Warning, "myMessage");
        let event_with_process_output = event.with_process_output(test_process_output);

        assert_ne!(event_with_process_output, event);
        assert!(event.get_process_output().is_none());
        let po = event_with_process_output.get_process_output().unwrap();

        assert_eq!(
            event_with_process_output.get_std_out(),
            Some(stdout.clone())
        );
        assert_eq!(po.get_std_out(), stdout);
        assert_eq!(po.get_std_out_size().unwrap(), stdout.len() as i64);

        assert_eq!(
            event_with_process_output.get_std_err(),
            Some(stderr.clone())
        );
        assert_eq!(po.get_std_err(), stderr);
        assert_eq!(po.get_std_err_size().unwrap(), stderr.len() as i64);
    }

    // Java: src/test/java/com/google/devtools/build/lib/events/EventTest.java EventTest.replayEventsOn
    #[test]
    fn replay_events_on() {
        let events = [
            Event::of(EventKind::Info, "someInfo"),
            Event::of(EventKind::Warning, "someWarning"),
        ];
        let mock = RecordingEventHandler::default();
        Event::replay_events_on(&mock, &events);
        assert_eq!(mock.events(), events.to_vec());
    }

    // Java: src/test/java/com/google/devtools/build/lib/events/EventTest.java EventTest.replaySyntaxErrorsOn
    #[test]
    fn replay_syntax_errors_on() {
        let location1 = Location::from_file_line_column("someFile", 3, 4);
        let location2 = Location::from_file_line_column("someOtherFile", 5, 6);
        let syntax_errors = vec![
            (location1.clone(), "message1".to_string()),
            (location2.clone(), "message2".to_string()),
        ];
        let mock = RecordingEventHandler::default();
        Event::replay_syntax_errors_on(&mock, syntax_errors);
        assert_eq!(
            mock.events(),
            vec![
                Event::error_at(Some(location1), "message1"),
                Event::error_at(Some(location2), "message2"),
            ]
        );
    }

    // Java: src/test/java/com/google/devtools/build/lib/events/EventTest.java EventTest.debugPrintHandler
    #[test]
    #[ignore = "needs the starlark-rust print handler integration (Event.makeDebugPrintHandler)"]
    fn debug_print_handler() {}
}
