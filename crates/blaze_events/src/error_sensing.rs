//! Port of `com.google.devtools.build.lib.events.ErrorSensingEventHandler`.

use std::marker::PhantomData;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};

use parking_lot::Mutex;

use crate::event::{Event, EventProperty};
use crate::event_kind::EventKind;
use crate::handler::{DelegatingEventHandler, EventHandler, ExtendedEventHandler, Postable};

/// Passes through any events, and notes if any of them were errors. It is thread-safe as long as
/// the target eventHandler is thread-safe.
///
/// Optionally retains the first error event property value associated with a specified type.
/// Port of `ErrorSensingEventHandler<T>`.
// Java: com/google/devtools/build/lib/events/ErrorSensingEventHandler.java:26 ErrorSensingEventHandler
pub struct ErrorSensingEventHandler<T: EventProperty + Clone = ()> {
    base: DelegatingEventHandler,
    /// PORT: Java's nullable `Class<T> errorPropertyClass`; `true` iff it is non-null.
    error_property_class: bool,
    error_property: Mutex<Option<T>>,
    has_errors: AtomicBool,
    _marker: PhantomData<fn() -> T>,
}

impl<T: EventProperty + Clone> ErrorSensingEventHandler<T> {
    /// `track_error_property` is Java's `errorPropertyClass != null`, with `T` the class.
    // Java: com/google/devtools/build/lib/events/ErrorSensingEventHandler.java:32 ErrorSensingEventHandler.ErrorSensingEventHandler
    pub fn new(event_handler: Arc<dyn ExtendedEventHandler>, track_error_property: bool) -> Self {
        ErrorSensingEventHandler {
            base: DelegatingEventHandler::new(event_handler),
            error_property_class: track_error_property,
            error_property: Mutex::new(None),
            has_errors: AtomicBool::new(false),
            _marker: PhantomData,
        }
    }

    /// Returns whether any of the events on this objects were errors.
    // Java: com/google/devtools/build/lib/events/ErrorSensingEventHandler.java:58 ErrorSensingEventHandler.hasErrors
    pub fn has_errors(&self) -> bool {
        self.has_errors.load(Ordering::SeqCst)
    }

    /// Returns the retained error event property value, if any.
    // Java: com/google/devtools/build/lib/events/ErrorSensingEventHandler.java:64 ErrorSensingEventHandler.getErrorProperty
    pub fn get_error_property(&self) -> Option<T> {
        self.error_property.lock().clone()
    }
}

impl ErrorSensingEventHandler<()> {
    // Java: com/google/devtools/build/lib/events/ErrorSensingEventHandler.java:38 ErrorSensingEventHandler.withoutPropertyValueTracking
    pub fn without_property_value_tracking(
        event_handler: Arc<dyn ExtendedEventHandler>,
    ) -> ErrorSensingEventHandler<()> {
        ErrorSensingEventHandler::new(event_handler, /* errorPropertyClass= */ false)
    }
}

impl<T: EventProperty + Clone> EventHandler for ErrorSensingEventHandler<T> {
    // Java: com/google/devtools/build/lib/events/ErrorSensingEventHandler.java:44 ErrorSensingEventHandler.handle
    fn handle(&self, e: &Event) {
        if e.get_kind() == EventKind::Error {
            self.has_errors.store(true, Ordering::SeqCst);
            if self.error_property_class
                && let Some(property_value) = e.get_property::<T>()
            {
                // compareAndSet(/*expect=*/ null, /*update=*/ propertyValue)
                let mut slot = self.error_property.lock();
                if slot.is_none() {
                    *slot = Some(property_value.clone());
                }
            }
        }
        self.base.handle(e);
    }

    fn as_extended_event_handler(&self) -> Option<&dyn ExtendedEventHandler> {
        Some(self)
    }
}

impl<T: EventProperty + Clone> ExtendedEventHandler for ErrorSensingEventHandler<T> {
    fn post(&self, obj: Arc<dyn Postable>) {
        self.base.post(obj);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::handler::tests::RecordingEventHandler;

    // Java: src/test/java/com/google/devtools/build/lib/events/ErrorSensingEventHandlerTest.java ErrorSensingEventHandlerTest.delegation
    #[test]
    fn delegation() {
        let delegate = Arc::new(RecordingEventHandler::default());
        let subject = ErrorSensingEventHandler::without_property_value_tracking(delegate.clone());
        let event = Event::of(EventKind::Info, "message");

        subject.handle(&event);

        assert_eq!(delegate.events(), vec![event]);
    }

    // Java: src/test/java/com/google/devtools/build/lib/events/ErrorSensingEventHandlerTest.java ErrorSensingEventHandlerTest.rememberError
    #[test]
    fn remember_error() {
        let delegate = Arc::new(RecordingEventHandler::default());
        let subject = ErrorSensingEventHandler::without_property_value_tracking(delegate);

        subject.handle(&Event::of(EventKind::Info, "message"));

        assert!(!subject.has_errors());

        subject.handle(&Event::of(EventKind::Error, "anError"));

        assert!(subject.has_errors());
    }

    // Java: src/test/java/com/google/devtools/build/lib/events/ErrorSensingEventHandlerTest.java ErrorSensingEventHandlerTest.rememberErrorProperty
    #[test]
    fn remember_error_property() {
        let delegate = Arc::new(RecordingEventHandler::default());

        let without_tracking =
            ErrorSensingEventHandler::without_property_value_tracking(delegate.clone());
        let with_tracking = ErrorSensingEventHandler::<String>::new(delegate, true);

        let nonerror_event =
            Event::info("nonerror").with_property(Some("propertyValue".to_string()));
        without_tracking.handle(&nonerror_event);
        with_tracking.handle(&nonerror_event);

        assert_eq!(without_tracking.get_error_property(), None);
        assert_eq!(with_tracking.get_error_property(), None);

        let error_event = Event::error("anError").with_property(Some("propertyValue".to_string()));
        without_tracking.handle(&error_event);
        with_tracking.handle(&error_event);

        assert_eq!(without_tracking.get_error_property(), None);
        assert_eq!(
            with_tracking.get_error_property().as_deref(),
            Some("propertyValue")
        );

        with_tracking
            .handle(&Event::error("anotherError").with_property(Some("ignoredValue".to_string())));
        assert_eq!(
            with_tracking.get_error_property().as_deref(),
            Some("propertyValue")
        );
    }
}
