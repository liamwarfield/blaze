//! Ports of `Reporter` and `ReporterStream`.

use std::io;
use std::sync::{Arc, Weak};

use blaze_util::unported;
use parking_lot::{Mutex, RwLock};

use crate::event::Event;
use crate::event_kind::EventKind;
use crate::handler::{EventHandler, ExtendedEventHandler, Postable};
use crate::io::{OutErr, OutputStream};
use crate::location::Location;
use crate::output_filter::{OutputFilter, output_everything};

/// The receiving end of `Reporter.post`: Guava's `EventBus` in Java.
///
/// PORT: Guava's `EventBus` dispatches on the posted object's class to `@Subscribe` methods. The
/// subscriber registry belongs to the runtime crate; the reporter only needs to post into it.
pub trait EventBus: Send + Sync {
    fn post(&self, event: Arc<dyn Postable>);
}

fn same_handler(a: &Arc<dyn EventHandler>, b: &Arc<dyn EventHandler>) -> bool {
    std::ptr::addr_eq(Arc::as_ptr(a), Arc::as_ptr(b))
}

#[derive(Default)]
struct AnsiHandlers {
    ansi_allowing_handler: Option<Arc<dyn EventHandler>>,
    ansi_stripping_handler: Option<Arc<dyn EventHandler>>,
    ansi_allowing_handler_registered: bool,
}

/// The reporter is the primary means of reporting events such as errors, warnings, progress
/// information and diagnostic information to the user. It is not intended as a logging mechanism
/// for developer-only messages. Port of `Reporter`.
///
/// The reporter instance is consumed by the build system, and passes events using
/// [`EventHandler::handle`] or [`ExtendedEventHandler::post`] to [`EventHandler`] instances. The
/// latter only occurs to the handlers that are also [`ExtendedEventHandler`]s. Additionally, when
/// events are passed using `post` they are also posted to the [`EventBus`] registered in this
/// reporter.
///
/// Thread-safe: calls to `handle` may be made on any thread.
// Java: com/google/devtools/build/lib/events/Reporter.java:42 Reporter
pub struct Reporter {
    /// Set of [`EventHandler`] registered in this reporter.
    event_handlers: RwLock<Vec<Arc<dyn EventHandler>>>,
    /// [`EventBus`] registered in this reporter. Calls to `post` will propagate events to the
    /// event bus.
    event_bus: RwLock<Option<Arc<dyn EventBus>>>,
    /// An OutErr that sends all of its output to this Reporter. Each write will (when flushed)
    /// get mapped to an EventKind.STDOUT or EventKind.STDERR event.
    out_err_to_reporter: OutErr,
    output_filter: RwLock<Arc<dyn OutputFilter>>,
    ansi: Mutex<AnsiHandlers>,
}

impl Reporter {
    /// PORT: the reporter's own `OutErr` points back at it, so reporters are always created
    /// inside an `Arc` (the stream holds a weak reference).
    fn build(
        event_bus: Option<Arc<dyn EventBus>>,
        handlers: Vec<Arc<dyn EventHandler>>,
    ) -> Arc<Reporter> {
        Arc::new_cyclic(|weak: &Weak<Reporter>| {
            let weak: Weak<dyn EventHandler> = weak.clone();
            Reporter {
                event_handlers: RwLock::new(handlers),
                event_bus: RwLock::new(event_bus),
                out_err_to_reporter: OutErr::create(
                    Arc::new(ReporterStream::new_weak(weak.clone(), EventKind::Stdout)),
                    Arc::new(ReporterStream::new_weak(weak, EventKind::Stderr)),
                ),
                output_filter: RwLock::new(output_everything()),
                ansi: Mutex::new(AnsiHandlers::default()),
            }
        })
    }

    // Java: com/google/devtools/build/lib/events/Reporter.java:62 Reporter.Reporter
    pub fn new(event_bus: Option<Arc<dyn EventBus>>) -> Arc<Reporter> {
        Reporter::build(event_bus, Vec::new())
    }

    /// A copy constructor, to make it convenient to replicate a reporter config for temporary
    /// configuration changes.
    // Java: com/google/devtools/build/lib/events/Reporter.java:70 Reporter.Reporter
    pub fn copy_of(template: &Reporter) -> Arc<Reporter> {
        Reporter::build(
            template.event_bus.read().clone(),
            template.event_handlers.read().clone(),
        )
    }

    /// Constructor which configures a reporter with the specified handlers.
    // Java: com/google/devtools/build/lib/events/Reporter.java:76 Reporter.Reporter
    pub fn with_handlers(
        event_bus: Option<Arc<dyn EventBus>>,
        handlers: impl IntoIterator<Item = Arc<dyn EventHandler>>,
    ) -> Arc<Reporter> {
        let reporter = Reporter::new(event_bus);
        for handler in handlers {
            reporter.add_handler(handler);
        }
        reporter
    }

    // Java: com/google/devtools/build/lib/events/Reporter.java:83 Reporter.outErrForReporter
    pub fn out_err_for_reporter(rep: Arc<dyn EventHandler>) -> OutErr {
        // We don't use BufferedOutputStream here, because in general the Blaze
        // code base assumes that the output streams are not buffered.
        OutErr::create(
            Arc::new(ReporterStream::new(rep.clone(), EventKind::Stdout)),
            Arc::new(ReporterStream::new(rep, EventKind::Stderr)),
        )
    }

    /// Returns an OutErr that sends all of its output to this Reporter. Each write to the OutErr
    /// will cause an EventKind.STDOUT or EventKind.STDERR event.
    // Java: com/google/devtools/build/lib/events/Reporter.java:94 Reporter.getOutErr
    pub fn get_out_err(&self) -> &OutErr {
        &self.out_err_to_reporter
    }

    /// Registers an [`EventHandler`] in this reporter.
    // Java: com/google/devtools/build/lib/events/Reporter.java:99 Reporter.addHandler
    pub fn add_handler(&self, handler: Arc<dyn EventHandler>) {
        self.event_handlers.write().push(handler);
    }

    /// Removes an [`EventHandler`] from this reporter. If the handler wasn't registered in this
    /// reporter this method is a no-op.
    ///
    /// PORT: Java removes the first handler `equals` to the argument; handlers use identity.
    // Java: com/google/devtools/build/lib/events/Reporter.java:108 Reporter.removeHandler
    pub fn remove_handler(&self, handler: &Arc<dyn EventHandler>) {
        let mut handlers = self.event_handlers.write();
        if let Some(i) = handlers.iter().position(|h| same_handler(h, handler)) {
            handlers.remove(i);
        }
    }

    // Java: com/google/devtools/build/lib/events/Reporter.java:148 Reporter.clearEventBus
    pub fn clear_event_bus(&self) {
        *self.event_bus.write() = None;
    }

    /// Reports the start of a particular task. Is a wrapper around report() with event kind
    /// START. Should always be matched by a corresponding call to finishTask() with the same
    /// message, except that the leading percentage progress indicator (if any) in the message may
    /// differ.
    // Java: com/google/devtools/build/lib/events/Reporter.java:159 Reporter.startTask
    pub fn start_task(&self, location: Option<Location>, message: impl Into<String>) {
        self.handle(&Event::of_at(EventKind::Start, location, message));
    }

    /// Reports the end of a particular task. Is a wrapper around report() with event kind FINISH.
    // Java: com/google/devtools/build/lib/events/Reporter.java:170 Reporter.finishTask
    pub fn finish_task(&self, location: Option<Location>, message: impl Into<String>) {
        self.handle(&Event::of_at(EventKind::Finish, location, message));
    }

    // Java: com/google/devtools/build/lib/events/Reporter.java:174 Reporter.error
    pub fn error(&self, location: Option<Location>, message: impl Into<String>) {
        self.error_with_cause(location, message, /* error= */ None);
    }

    // Java: com/google/devtools/build/lib/events/Reporter.java:178 Reporter.error
    pub fn error_with_cause(
        &self,
        location: Option<Location>,
        message: impl Into<String>,
        error: Option<&dyn std::error::Error>,
    ) {
        self.handle(&Event::error_at(location, message));
        if error.is_some() {
            // Java prints the Throwable's JVM stack trace to getOutErr().getErrorStream(); that
            // text has no Rust equivalent yet.
            unported!("Throwable.printStackTrace");
        }
    }

    /// Returns true iff the given tag matches the output filter.
    // Java: com/google/devtools/build/lib/events/Reporter.java:188 Reporter.showOutput
    pub fn show_output(&self, tag: &str) -> bool {
        self.output_filter.read().show_output(tag)
    }

    // Java: com/google/devtools/build/lib/events/Reporter.java:192 Reporter.setOutputFilter
    pub fn set_output_filter(&self, output_filter: Arc<dyn OutputFilter>) {
        *self.output_filter.write() = output_filter;
    }

    /// Registers an ANSI-control-code-allowing EventHandler with an ANSI-stripping EventHandler
    /// that is already registered with the reporter. The ANSI-stripping handler can then be
    /// replaced with the ANSI-allowing handler by calling `switch_to_ansi_allowing_handler` which
    /// calls `remove_handler` for the ANSI-stripping handler and then `add_handler` for the
    /// ANSI-allowing handler.
    // Java: com/google/devtools/build/lib/events/Reporter.java:203 Reporter.registerAnsiAllowingHandler
    pub fn register_ansi_allowing_handler(
        &self,
        ansi_stripping_handler: Arc<dyn EventHandler>,
        ansi_allowing_handler: Arc<dyn EventHandler>,
    ) {
        let mut ansi = self.ansi.lock();
        ansi.ansi_allowing_handler = Some(ansi_allowing_handler);
        ansi.ansi_stripping_handler = Some(ansi_stripping_handler);
        ansi.ansi_allowing_handler_registered = true;
    }

    /// Restores the ANSI-allowing EventHandler registered using
    /// `register_ansi_allowing_handler`.
    // Java: com/google/devtools/build/lib/events/Reporter.java:215 Reporter.switchToAnsiAllowingHandler
    pub fn switch_to_ansi_allowing_handler(&self) {
        let mut ansi = self.ansi.lock();
        if ansi.ansi_allowing_handler_registered {
            if let Some(stripping) = ansi.ansi_stripping_handler.take() {
                self.remove_handler(&stripping);
            }
            if let Some(allowing) = ansi.ansi_allowing_handler.take() {
                self.add_handler(allowing);
            }
            ansi.ansi_allowing_handler_registered = false;
        }
    }
}

impl EventHandler for Reporter {
    /// Handle the provided [`Event`] using all the [`EventHandler`] registered in this reporter.
    // Java: com/google/devtools/build/lib/events/Reporter.java:118 Reporter.handle
    fn handle(&self, e: &Event) {
        if e.get_kind() != EventKind::Error
            && e.get_kind() != EventKind::Debug
            && let Some(tag) = e.get_tag()
            && !self.show_output(tag)
        {
            return;
        }

        // PORT: Java iterates a ConcurrentLinkedQueue (weakly consistent); we iterate a snapshot
        // so handlers may add or remove handlers while being called.
        let handlers = self.event_handlers.read().clone();
        for handler in &handlers {
            handler.handle(e);
        }
    }

    fn as_extended_event_handler(&self) -> Option<&dyn ExtendedEventHandler> {
        Some(self)
    }
}

impl ExtendedEventHandler for Reporter {
    /// Post the provided [`Postable`] to the [`EventBus`] and all the [`ExtendedEventHandler`]
    /// registered in this reporter.
    // Java: com/google/devtools/build/lib/events/Reporter.java:136 Reporter.post
    fn post(&self, obj: Arc<dyn Postable>) {
        let event_bus = self.event_bus.read().clone();
        if let Some(event_bus) = event_bus {
            event_bus.post(obj.clone());
        }

        let handlers = self.event_handlers.read().clone();
        for event_handler in &handlers {
            if let Some(extended) = event_handler.as_extended_event_handler() {
                extended.post(obj.clone());
            }
        }
    }
}

enum HandlerRef {
    Strong(Arc<dyn EventHandler>),
    /// The reporter's own streams (see [`Reporter::build`]).
    Weak(Weak<dyn EventHandler>),
}

/// An OutputStream that delegates all writes to an EventHandler. Port of `ReporterStream`.
// Java: com/google/devtools/build/lib/events/ReporterStream.java:24 ReporterStream
pub struct ReporterStream {
    handler: HandlerRef,
    event_kind: EventKind,
}

impl ReporterStream {
    // Java: com/google/devtools/build/lib/events/ReporterStream.java:28 ReporterStream.ReporterStream
    pub fn new(handler: Arc<dyn EventHandler>, event_kind: EventKind) -> Self {
        ReporterStream {
            handler: HandlerRef::Strong(handler),
            event_kind,
        }
    }

    fn new_weak(handler: Weak<dyn EventHandler>, event_kind: EventKind) -> Self {
        ReporterStream {
            handler: HandlerRef::Weak(handler),
            event_kind,
        }
    }

    /// `write(int b)`: a single byte.
    // Java: com/google/devtools/build/lib/events/ReporterStream.java:44 ReporterStream.write
    pub fn write_byte(&self, b: u8) {
        self.emit(vec![b]);
    }

    fn emit(&self, bytes: Vec<u8>) {
        let event = Event::of_bytes_at(self.event_kind, None, bytes);
        match &self.handler {
            HandlerRef::Strong(h) => h.handle(&event),
            HandlerRef::Weak(w) => {
                if let Some(h) = w.upgrade() {
                    h.handle(&event);
                }
            }
        }
    }
}

impl OutputStream for ReporterStream {
    // Java: com/google/devtools/build/lib/events/ReporterStream.java:54 ReporterStream.write
    fn write(&self, bytes: &[u8]) -> io::Result<()> {
        self.emit(bytes.to_vec());
        Ok(())
    }

    // Java: com/google/devtools/build/lib/events/ReporterStream.java:39 ReporterStream.flush
    fn flush(&self) -> io::Result<()> {
        // NOP.
        Ok(())
    }

    // Java: com/google/devtools/build/lib/events/ReporterStream.java:34 ReporterStream.close
    fn close(&self) -> io::Result<()> {
        // NOP.
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::collectors::EventCollector;
    use crate::output_filter::RegexOutputFilter;

    fn location() -> Location {
        Location::from_file_line_column("/path/to/workspace/my/sample/path.txt", 3, 4)
    }

    /// `new AbstractEventHandler(EventKind.ERRORS) { handle: out.append(message) }`; like the
    /// Java test handler it ignores its mask.
    #[derive(Default)]
    struct OutAppender {
        out: Mutex<String>,
    }

    impl EventHandler for OutAppender {
        fn handle(&self, event: &Event) {
            self.out.lock().push_str(&event.get_message());
        }
    }

    // Java: src/test/java/com/google/devtools/build/lib/events/ReporterTest.java ReporterTest.reporterShowOutput
    #[test]
    fn reporter_show_output() {
        let reporter = Reporter::new(None);
        reporter.set_output_filter(RegexOutputFilter::for_regex("naughty").unwrap());
        let collector = Arc::new(EventCollector::new());
        reporter.add_handler(collector.clone());
        let interesting = Event::warn_at(None, "show-me").with_tag(Some("naughty"));

        reporter.handle(&interesting);
        reporter.handle(&Event::warn_at(None, "ignore-me").with_tag(Some("good")));

        assert_eq!(vec![interesting], collector.iter().collect::<Vec<_>>());
    }

    // Java: src/test/java/com/google/devtools/build/lib/events/ReporterTest.java ReporterTest.reporterCollectsEvents
    #[test]
    fn reporter_collects_events() {
        let reporter = Reporter::new(None);
        let want = vec![Event::warn("xyz"), Event::error("err")];
        let collector = Arc::new(EventCollector::new());
        reporter.add_handler(collector.clone());
        for e in &want {
            reporter.handle(e);
        }
        let got: Vec<Event> = collector.iter().collect();
        assert_eq!(want, got);
    }

    // Java: src/test/java/com/google/devtools/build/lib/events/ReporterTest.java ReporterTest.reporterCopyConstructorCopiesHandlersList
    #[test]
    fn reporter_copy_constructor_copies_handlers_list() {
        let reporter = Reporter::new(None);
        let out_appender = Arc::new(OutAppender::default());
        let h: Arc<dyn EventHandler> = out_appender.clone();
        reporter.add_handler(h.clone());
        reporter.add_handler(h.clone());
        let copied_reporter = Reporter::copy_of(&reporter);
        copied_reporter.add_handler(h.clone()); // Should have 3 handlers now.
        reporter.add_handler(h.clone());
        reporter.add_handler(h.clone()); // Should have 4 handlers now.
        copied_reporter.handle(&Event::error_at(Some(location()), "."));
        assert_eq!(*out_appender.out.lock(), "..."); // The copied reporter has 3 handlers.
        out_appender.out.lock().clear();
        reporter.handle(&Event::error_at(Some(location()), "."));
        assert_eq!(*out_appender.out.lock(), "...."); // The old reporter has 4 handlers.
    }

    // Java: src/test/java/com/google/devtools/build/lib/events/ReporterTest.java ReporterTest.removeHandlerUndoesAddHandler
    #[test]
    fn remove_handler_undoes_add_handler() {
        let reporter = Reporter::new(None);
        let out_appender = Arc::new(OutAppender::default());
        let h: Arc<dyn EventHandler> = out_appender.clone();
        assert!(out_appender.out.lock().is_empty());
        reporter.add_handler(h.clone());
        reporter.handle(&Event::error_at(Some(location()), "Event gets registered."));
        assert_eq!(*out_appender.out.lock(), "Event gets registered.");
        out_appender.out.lock().clear();
        reporter.remove_handler(&h);
        reporter.handle(&Event::error_at(Some(location()), "Event gets ignored."));
        assert!(out_appender.out.lock().is_empty());
    }

    #[derive(Default)]
    struct FakeExtendedEventHandler {
        called_post: Mutex<i32>,
    }

    impl EventHandler for FakeExtendedEventHandler {
        fn handle(&self, _event: &Event) {
            panic!("UnsupportedOperationException");
        }

        fn as_extended_event_handler(&self) -> Option<&dyn ExtendedEventHandler> {
            Some(self)
        }
    }

    impl ExtendedEventHandler for FakeExtendedEventHandler {
        fn post(&self, _obj: Arc<dyn Postable>) {
            *self.called_post.lock() += 1;
        }
    }

    #[derive(Debug)]
    struct FakePostable;

    impl Postable for FakePostable {}

    // Java: src/test/java/com/google/devtools/build/lib/events/ReporterTest.java ReporterTest.propagatePostCalls
    #[test]
    fn propagate_post_calls() {
        let reporter = Reporter::new(None);
        let extended_event_handler = Arc::new(FakeExtendedEventHandler::default());
        assert_eq!(*extended_event_handler.called_post.lock(), 0);

        reporter.add_handler(extended_event_handler.clone());
        reporter.post(Arc::new(FakePostable));

        assert_eq!(*extended_event_handler.called_post.lock(), 1);
    }

    // Java: src/test/java/com/google/devtools/build/lib/events/SimpleReportersTest.java SimpleReportersTest.addsHandlers
    #[test]
    fn adds_handlers() {
        #[derive(Default)]
        struct Counter(Mutex<i32>);
        impl EventHandler for Counter {
            fn handle(&self, _event: &Event) {
                *self.0.lock() += 1;
            }
        }
        let handler = Arc::new(Counter::default());

        let reporter = Reporter::with_handlers(None, [handler.clone() as Arc<dyn EventHandler>]);
        reporter.handle(&Event::info_at(Some(location()), "Add to handlerCount."));
        reporter.handle(&Event::info_at(Some(location()), "Add to handlerCount."));
        reporter.handle(&Event::info_at(Some(location()), "Add to handlerCount."));
        assert_eq!(*handler.0.lock(), 3);
    }

    // Java: src/test/java/com/google/devtools/build/lib/events/ReporterStreamTest.java ReporterStreamTest.reporterStream
    #[test]
    fn reporter_stream() {
        struct OutAppender(Mutex<String>);
        impl EventHandler for OutAppender {
            fn handle(&self, event: &Event) {
                self.0.lock().push_str(&format!(
                    "[{}: {}]\n",
                    event.get_kind(),
                    event.get_message()
                ));
            }
        }
        let reporter = Reporter::new(None);
        let out = Arc::new(OutAppender(Mutex::new(String::new())));
        assert!(out.0.lock().is_empty());
        reporter.add_handler(out.clone());
        let rep: Arc<dyn EventHandler> = reporter.clone();
        {
            // PrintWriter(stream, autoFlush=true).println writes the line and separator in one
            // flush.
            let warn_writer = ReporterStream::new(rep.clone(), EventKind::Warning);
            let info_writer = ReporterStream::new(rep, EventKind::Info);
            info_writer.write(b"some info\n").unwrap();
            warn_writer.write(b"a warning\n").unwrap();
        }
        reporter.get_out_err().print_out_ln("some output");
        reporter.get_out_err().print_err_ln("an error");
        assert_eq!(
            *out.0.lock(),
            "[INFO: some info\n]\n\
             [WARNING: a warning\n]\n\
             [STDOUT: some output\n]\n\
             [STDERR: an error\n]\n"
        );
    }
}
