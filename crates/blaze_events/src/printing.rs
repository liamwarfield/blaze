//! Port of `com.google.devtools.build.lib.events.PrintingEventHandler`.

use std::io;
use std::sync::LazyLock;

use parking_lot::Mutex;

use crate::event::Event;
use crate::event_kind::{EventKind, EventKindSet};
use crate::handler::{AbstractEventHandler, EventHandler};
use crate::io::OutErr;

/// An event handler that prints to an OutErr stream pair in a canonical format, for example:
///
/// ```text
/// ERROR: /home/jrluser/src/workspace/x/BUILD:23:1: syntax error.
/// ```
///
/// This syntax is parseable by Emacs's compile.el. Port of `PrintingEventHandler`.
// Java: com/google/devtools/build/lib/events/PrintingEventHandler.java:29 PrintingEventHandler
pub struct PrintingEventHandler {
    base: AbstractEventHandler,
    out_err: Mutex<OutErr>,
}

impl PrintingEventHandler {
    /// A convenient event-handler for terminal applications that prints all errors and warnings
    /// it encounters to the error stream. STDOUT and STDERR events pass their output directly
    /// through to the corresponding streams.
    // Java: com/google/devtools/build/lib/events/PrintingEventHandler.java:38 PrintingEventHandler.ERRORS_AND_WARNINGS_TO_STDERR
    pub fn errors_and_warnings_to_stderr() -> &'static PrintingEventHandler {
        static HANDLER: LazyLock<PrintingEventHandler> =
            LazyLock::new(|| PrintingEventHandler::new(EventKind::ERRORS_AND_WARNINGS_AND_OUTPUT));
        &HANDLER
    }

    /// A convenient event-handler for terminal applications that prints all errors it encounters
    /// to the error stream. STDOUT and STDERR events pass their output directly through to the
    /// corresponding streams.
    // Java: com/google/devtools/build/lib/events/PrintingEventHandler.java:47 PrintingEventHandler.ERRORS_TO_STDERR
    pub fn errors_to_stderr() -> &'static PrintingEventHandler {
        static HANDLER: LazyLock<PrintingEventHandler> =
            LazyLock::new(|| PrintingEventHandler::new(EventKind::ERRORS_AND_OUTPUT));
        &HANDLER
    }

    /// Setup a printing event handler that prints events matching the mask.
    // Java: com/google/devtools/build/lib/events/PrintingEventHandler.java:55 PrintingEventHandler.PrintingEventHandler
    pub fn with_out_err(out_err: OutErr, mask: EventKindSet) -> Self {
        PrintingEventHandler {
            base: AbstractEventHandler::new(mask),
            out_err: Mutex::new(out_err),
        }
    }

    /// Setup a printing event handler that prints events matching the mask. Events are printed to
    /// the process's stdout and stderr unless/until redirected by a call to `set_out_err`.
    // Java: com/google/devtools/build/lib/events/PrintingEventHandler.java:64 PrintingEventHandler.PrintingEventHandler
    pub fn new(mask: EventKindSet) -> Self {
        Self::with_out_err(OutErr::system_out_err().clone(), mask)
    }

    pub fn get_event_mask(&self) -> EventKindSet {
        self.base.get_event_mask()
    }

    /// Redirect all output to the specified OutErr stream pair. Returns the previous OutErr.
    // Java: com/google/devtools/build/lib/events/PrintingEventHandler.java:72 PrintingEventHandler.setOutErr
    pub fn set_out_err(&self, out_err: OutErr) -> OutErr {
        std::mem::replace(&mut *self.out_err.lock(), out_err)
    }

    fn print(out_err: &OutErr, event: &Event) -> io::Result<()> {
        match event.get_kind() {
            EventKind::Stdout => {
                out_err
                    .get_output_stream()
                    .write(event.get_message_bytes())?;
                out_err.get_output_stream().flush()?;
            }
            EventKind::Stderr => {
                out_err
                    .get_error_stream()
                    .write(event.get_message_bytes())?;
                out_err.get_error_stream().flush()?;
            }
            _ => {
                let mut builder = String::new();
                builder.push_str(event.get_kind().name());
                builder.push_str(": ");
                if let Some(location) = event.get_location() {
                    builder.push_str(&location.to_string());
                    builder.push_str(": ");
                }
                builder.push_str(&event.get_message());
                builder.push('\n');
                out_err.get_error_stream().write(builder.as_bytes())?;
                out_err.get_error_stream().flush()?;
            }
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/events/PrintingEventHandler.java:121 PrintingEventHandler.handleFollowUpEvents
    fn handle_follow_up_events(&self, event: &Event) {
        if let Some(stderr) = event.get_std_err() {
            self.handle(&Event::of_bytes_at(EventKind::Stderr, None, stderr));
        }
        if let Some(stdout) = event.get_std_out() {
            self.handle(&Event::of_bytes_at(EventKind::Stdout, None, stdout));
        }
    }
}

impl EventHandler for PrintingEventHandler {
    /// Print a description of the specified event to the appropriate output or error stream.
    // Java: com/google/devtools/build/lib/events/PrintingEventHandler.java:83 PrintingEventHandler.handle
    fn handle(&self, event: &Event) {
        if !self.get_event_mask().contains(event.get_kind()) {
            self.handle_follow_up_events(event);
            return;
        }
        let out_err = self.out_err.lock().clone();
        if let Err(e) = PrintingEventHandler::print(&out_err, event) {
            // Note: we can't print to System.out or System.err here, because those will normally
            // be set to streams which translate I/O to STDOUT and STDERR events, which would
            // result in infinite recursion.
            // PORT: Java prints IOException.getMessage(); io::Error's text differs.
            out_err.print_err_ln(&e.to_string());
        }
        self.handle_follow_up_events(event);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::io::RecordingOutErr;
    use crate::location::Location;

    // Java: src/test/java/com/google/devtools/build/lib/events/PrintingEventHandlerTest.java PrintingEventHandlerTest.collectsEvents
    #[test]
    fn collects_events() {
        let location =
            Location::from_file_line_column("/path/to/workspace/my/sample/path.txt", 3, 4);
        let event = Event::of_at(
            EventKind::Warning,
            Some(location),
            "This is not an error message.",
        );
        let recording_out_err = RecordingOutErr::new();
        let handler = PrintingEventHandler::with_out_err(
            recording_out_err.out_err().clone(),
            EventKind::ERRORS_AND_WARNINGS,
        );
        handler.handle(&event);
        assert_eq!(
            recording_out_err.err_as_latin1(),
            "WARNING: /path/to/workspace/my/sample/path.txt:3:4: \
             This is not an error message.\n"
        );
        assert!(recording_out_err.out_as_latin1().is_empty());
    }
}
