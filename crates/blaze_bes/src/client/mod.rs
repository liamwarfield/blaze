//! Port of `buildeventservice/client`: the client interface ([`BuildEventServiceClient`]), the
//! request builders ([`proto_util`]) and the gRPC implementation ([`grpc_client`]).

pub mod grpc_client;
pub mod proto_util;

use std::sync::Arc;
use std::time::Duration;

use indexmap::IndexSet;
use parking_lot::{Condvar, Mutex};

pub use tonic::Status;

/// `InterruptedException`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Interrupted;

/// An instant (`java.time.Instant`): seconds and nanoseconds since the epoch.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Instant {
    /// `getEpochSecond()`.
    pub epoch_second: i64,
    /// `getNano()`, in `0..1_000_000_000`.
    pub nano: i32,
}

impl Instant {
    /// `Instant.ofEpochMilli(millis)`.
    pub fn of_epoch_milli(millis: i64) -> Instant {
        Instant {
            epoch_second: millis.div_euclid(1000),
            nano: (millis.rem_euclid(1000) * 1_000_000) as i32,
        }
    }

    /// `Instant.now()` (`JavaClock.now()`).
    pub fn now() -> Instant {
        let d = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default();
        Instant {
            epoch_second: d.as_secs() as i64,
            nano: d.subsec_nanos() as i32,
        }
    }
}

/// Context for a build command.
// Java: com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceClient.java:36 BuildEventServiceClient.CommandContext
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommandContext {
    pub build_id: String,
    pub invocation_id: String,
    pub attempt_number: i32,
    /// An `ImmutableSet` (insertion order).
    pub keywords: IndexSet<String>,
    pub project_id: Option<String>,
    pub check_preceding_lifecycle_events: bool,
}

impl CommandContext {
    /// The record's compact constructor (`checkArgument(attemptNumber >= 1)`).
    // Java: com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceClient.java:43 BuildEventServiceClient.CommandContext.<init>
    pub fn new(
        build_id: String,
        invocation_id: String,
        attempt_number: i32,
        keywords: IndexSet<String>,
        project_id: Option<String>,
        check_preceding_lifecycle_events: bool,
    ) -> CommandContext {
        blaze_util::check_argument!(attempt_number >= 1);
        CommandContext {
            build_id,
            invocation_id,
            attempt_number,
            keywords,
            project_id,
            check_preceding_lifecycle_events,
        }
    }
}

/// The status of an invocation.
// Java: com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceClient.java:69 BuildEventServiceClient.InvocationStatus
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InvocationStatus {
    /// No information is available about the invocation status.
    Unknown,
    /// The invocation succeeded.
    Succeeded,
    /// The invocation failed.
    Failed,
}

/// A lifecycle event.
// Java: com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceClient.java:79 BuildEventServiceClient.LifecycleEvent
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LifecycleEvent {
    /// The lifecycle event signalling that the build was enqueued.
    BuildEnqueued { event_time: Instant },
    /// The lifecycle event signalling that the invocation was started.
    InvocationStarted { event_time: Instant },
    /// The lifecycle event signalling that the invocation was finished.
    InvocationFinished {
        event_time: Instant,
        status: InvocationStatus,
    },
    /// The lifecycle event signalling that the build was finished.
    BuildFinished {
        event_time: Instant,
        status: InvocationStatus,
    },
}

/// An event sent over a [`StreamContext`].
// Java: com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceClient.java:106 BuildEventServiceClient.StreamEvent
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StreamEvent {
    /// An event containing a `BuildEventStreamProtos.BuildEvent` (`payload`: in wire format).
    BazelEvent {
        event_time: Instant,
        sequence_number: i64,
        payload: Vec<u8>,
    },
    /// An event signalling the end of the stream.
    StreamFinished {
        event_time: Instant,
        sequence_number: i64,
    },
}

impl StreamEvent {
    /// `eventTime()`.
    pub fn event_time(&self) -> Instant {
        match self {
            StreamEvent::BazelEvent { event_time, .. }
            | StreamEvent::StreamFinished { event_time, .. } => *event_time,
        }
    }

    /// `sequenceNumber()`.
    pub fn sequence_number(&self) -> i64 {
        match self {
            StreamEvent::BazelEvent {
                sequence_number, ..
            }
            | StreamEvent::StreamFinished {
                sequence_number, ..
            } => *sequence_number,
        }
    }
}

/// Callback for ACKed build events: called whenever an ACK from the BES server is received.
/// ACKs are expected to be received in sequence. Implementations must be thread-safe.
// Java: com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceClient.java:129 BuildEventServiceClient.AckCallback
pub type AckCallback = Arc<dyn Fn(i64) + Send + Sync>;

/// `SettableFuture<Status>`: the completed status of a stream. The first `set` wins; listeners
/// run on the thread that sets it (or at once when it is already set).
#[derive(Clone, Default)]
pub struct StatusFuture(Arc<StatusFutureInner>);

#[derive(Default)]
struct StatusFutureInner {
    state: Mutex<StatusFutureState>,
    cond: Condvar,
}

#[derive(Default)]
struct StatusFutureState {
    status: Option<Status>,
    listeners: Vec<Box<dyn FnOnce(Status) + Send>>,
}

impl StatusFuture {
    /// `SettableFuture.create()`.
    pub fn create() -> StatusFuture {
        StatusFuture::default()
    }

    /// `Futures.immediateFuture(status)`.
    pub fn immediate(status: Status) -> StatusFuture {
        let future = StatusFuture::create();
        future.set(status);
        future
    }

    /// `set(status)`: false when already set.
    pub fn set(&self, status: Status) -> bool {
        let listeners = {
            let mut state = self.0.state.lock();
            if state.status.is_some() {
                return false;
            }
            state.status = Some(status.clone());
            self.0.cond.notify_all();
            std::mem::take(&mut state.listeners)
        };
        for listener in listeners {
            listener(status.clone());
        }
        true
    }

    /// `isDone()`.
    pub fn is_done(&self) -> bool {
        self.0.state.lock().status.is_some()
    }

    /// `get()`.
    pub fn get(&self) -> Status {
        let mut state = self.0.state.lock();
        loop {
            if let Some(status) = &state.status {
                return status.clone();
            }
            self.0.cond.wait(&mut state);
        }
    }

    /// `get(timeout)`: `None` on timeout.
    pub fn get_timeout(&self, timeout: Duration) -> Option<Status> {
        let deadline = std::time::Instant::now() + timeout;
        let mut state = self.0.state.lock();
        loop {
            if let Some(status) = &state.status {
                return Some(status.clone());
            }
            if self.0.cond.wait_until(&mut state, deadline).timed_out() {
                return state.status.clone();
            }
        }
    }

    /// `Futures.addCallback(future, onSuccess)`.
    pub fn add_listener(&self, listener: impl FnOnce(Status) + Send + 'static) {
        let status = {
            let mut state = self.0.state.lock();
            match &state.status {
                Some(status) => status.clone(),
                None => {
                    state.listeners.push(Box::new(listener));
                    return;
                }
            }
        };
        listener(status);
    }
}

/// A handle to a bidirectional stream.
// Java: com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceClient.java:139 BuildEventServiceClient.StreamContext
pub trait StreamContext: Send + Sync {
    /// The completed status of the stream. The future will never fail, but in case of error will
    /// contain a corresponding status.
    fn get_status(&self) -> StatusFuture;

    /// Sends a [`StreamEvent`] over the currently open stream. In case of error, this method
    /// will fail silently and report the error via the future returned by
    /// [`StreamContext::get_status`].
    ///
    /// This method may block due to flow control.
    fn send_over_stream(&self, stream_event: StreamEvent) -> Result<(), Interrupted>;

    /// Half closes the currently opened stream. This method does not block. Callers should
    /// block on the future returned by [`StreamContext::get_status`] in order to make sure that
    /// all `ack_callback` calls have been received.
    fn half_close_stream(&self);

    /// Closes the currently opened stream with an error. This method does not block. Callers
    /// should block on the future returned by [`StreamContext::get_status`] in order to make
    /// sure that all ack callback calls have been received. This method is NOOP if the stream
    /// was already finished.
    fn abort_stream(&self, status: Status);
}

/// Interface used to abstract the Stubby and gRPC client implementations.
// Java: com/google/devtools/build/lib/buildeventservice/client/BuildEventServiceClient.java:34 BuildEventServiceClient
pub trait BuildEventServiceClient: Send + Sync {
    /// Makes a blocking RPC call that publishes a [`LifecycleEvent`].
    fn publish(
        &self,
        command_context: &CommandContext,
        lifecycle_event: &LifecycleEvent,
    ) -> Result<(), PublishError>;

    /// Starts a new stream with the given [`CommandContext`] and [`AckCallback`]. Callers must
    /// wait on the returned future contained in the [`StreamContext`] in order to guarantee that
    /// all callback calls have been received.
    fn open_stream(
        &self,
        command_context: &CommandContext,
        callback: AckCallback,
    ) -> Result<Box<dyn StreamContext>, Interrupted>;

    /// Called once to dispose resources that this client might be holding (such as thread
    /// pools). This should be the last method called on this object.
    fn shutdown(&self);

    /// If possible, returns a user readable error message for a given status exception
    /// (`userReadableError(Throwable t)` for the `StatusException` the uploader passes).
    fn user_readable_error(&self, status: &Status) -> String;
}

/// What [`BuildEventServiceClient::publish`] throws: `StatusException` or
/// `InterruptedException`.
#[derive(Debug, Clone)]
pub enum PublishError {
    Status(Status),
    Interrupted,
}

/// `io.grpc.Status.Code.name()`.
pub fn code_name(code: tonic::Code) -> &'static str {
    blaze_remote::common::errors::code_name(code)
}

/// `io.grpc.Status.toString()`: `Status{code=…, description=…, cause=…}`.
///
/// PORT: Java prints the cause as its stack trace (`Throwables.getStackTraceAsString`); the
/// port prints the chain of `std::error::Error` sources of the status, one message per line.
// Java: io/grpc/Status.java Status.toString
pub fn status_to_string(status: &Status) -> String {
    let description = if status.message().is_empty() {
        "null".to_string()
    } else {
        status.message().to_string()
    };
    let cause = match std::error::Error::source(status) {
        None => "null".to_string(),
        Some(source) => {
            let mut text = source.to_string();
            let mut next = source.source();
            while let Some(e) = next {
                text.push_str("\nCaused by: ");
                text.push_str(&e.to_string());
                next = e.source();
            }
            text
        }
    };
    format!(
        "Status{{code={}, description={description}, cause={cause}}}",
        code_name(status.code())
    )
}

/// `Status.CODE.withDescription(description)`.
pub fn status_with_description(code: tonic::Code, description: &str) -> Status {
    Status::new(code, description)
}
