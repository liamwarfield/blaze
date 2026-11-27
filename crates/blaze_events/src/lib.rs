//! Port of Bazel's event system (`lib/events`) and the terminal formatting of events
//! (the event-printing path of `lib/runtime/UiEventHandler` and `lib/util/io/AnsiTerminal*`).
//!
//! All handlers are `Send + Sync`: Bazel reports events from many threads.

#![allow(clippy::too_many_arguments, clippy::new_without_default)]

pub mod collectors;
pub mod error_sensing;
pub mod event;
pub mod event_kind;
pub mod handler;
pub mod io;
pub mod java_format;
pub mod time_utilities;
pub mod location;
pub mod output_filter;
pub mod printing;
pub mod reporter;
pub mod terminal;
pub mod terminal_writers;
pub mod ui;

pub use collectors::{EventCollector, StoredEventHandler};
pub use error_sensing::ErrorSensingEventHandler;
pub use event::{Event, EventProperty, ProcessOutput, ProcessOutputProperty};
pub use event_kind::{EventKind, EventKindSet};
pub use handler::{
    AbstractEventHandler, DelegatingEventHandler, EventHandler, ExtendedEventHandler,
    FetchProgress, NOOP, NullEventHandler, Postable, Reportable, replay_posts_on,
};
pub use io::{
    AnsiStrippingOutputStream, ByteArrayOutputStream, OutErr, OutputStream, RecordingOutErr,
    WriteOutputStream,
};
pub use location::Location;
pub use output_filter::{OutputFilter, RegexOutputFilter, output_everything, output_nothing};
pub use printing::PrintingEventHandler;
pub use reporter::{EventBus, Reporter, ReporterStream};
pub use terminal::{AnsiTerminal, AnsiTerminalPrinter, AnsiTerminalWriter, Color, Mode};
pub use ui::{
    Clock, JavaClock, LocationPrinter, UiEventHandler, UiEventHandlerOptions, ansi_strip_err,
    ansi_strip_out, format_date, format_event, format_timestamp, nano_time_of,
    set_event_kind_color, write_event_text,
};
