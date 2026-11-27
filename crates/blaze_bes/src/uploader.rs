//! Uploader of Build Events to the Build Event Service (BES).
//!
//! The purpose of this type is to manage the interaction between the BES client and the BES
//! server. It implements an event loop pattern based on the commands defined by [`Command`].

use std::collections::VecDeque;
use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use blaze_bep::buildeventstream::artifact_uploader::{
    BuildEventArtifactUploader, PathConverterFuture, UploadFailure,
};
use blaze_bep::buildeventstream::build_event::BuildEvent;
use blaze_bep::buildeventstream::build_event_context::{
    ArtifactGroupNamer, BuildEventContext, PathConverter,
};
use blaze_bep::buildeventstream::transport::{CloseFuture, TransportCloseError};
use blaze_events::EventBus;
use blaze_optdefs::misc::build_event_context::OutputGroupFileMode;
use blaze_optdefs::misc::build_event_protocol_options::{
    BuildEventProtocolOptions, OutputGroupFileModes,
};
use blaze_protos::JavaMessage;
use blaze_protos::build_event_stream as bes;
use blaze_protos::failure_details::build_progress::Code as BuildProgressCode;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail};
use blaze_shell::thread_interrupt::{self, ThreadInterrupt};
use blaze_util::ExitCode;
use parking_lot::{Condvar, Mutex};

use crate::availability_event::{
    BuildEventServiceAvailabilityEvent, LargeBuildEventSerializedEvent,
};
use crate::client::{
    BuildEventServiceClient, CommandContext, Instant, Interrupted, InvocationStatus,
    LifecycleEvent, PublishError, Status, StreamContext, StreamEvent, status_to_string,
    status_with_description,
};

/// `Sleeper`: sleeps on the calling thread; an interrupt ends the sleep.
pub trait Sleeper: Send + Sync {
    /// `sleepMillis(millis)`.
    fn sleep_millis(&self, millis: i64) -> Result<(), Interrupted>;
}

/// `JavaSleeper`: `Thread.sleep(millis)`.
#[derive(Debug, Default)]
pub struct JavaSleeper;

impl Sleeper for JavaSleeper {
    // Java: com/google/devtools/build/lib/util/JavaSleeper.java:23 JavaSleeper.sleepMillis
    fn sleep_millis(&self, millis: i64) -> Result<(), Interrupted> {
        blaze_remote::runtime::sleep(Duration::from_millis(millis.max(0) as u64))
            .map_err(|_| Interrupted)
    }
}

/// The clock of the uploader (`Clock.now()`).
pub trait BesClock: Send + Sync {
    fn now(&self) -> Instant;
}

/// `BlazeClock`: the system clock.
#[derive(Debug, Default)]
pub struct JavaClock;

impl BesClock for JavaClock {
    fn now(&self) -> Instant {
        Instant::now()
    }
}

/// Commands to drive the event loop.
// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceUploader.java:80 BuildEventServiceUploader.Command
enum Command {
    /// Tells the event loop to open a new BES stream.
    OpenStream,
    /// Tells the event loop that the streaming RPC completed.
    StreamComplete(Status),
    /// Tells the event loop that an ACK was received.
    AckReceived(i64),
    /// Tells the event loop to send a build event.
    SendRegularBuildEvent {
        sequence_number: i64,
        creation_time: Instant,
        event: Arc<dyn BuildEvent>,
        local_file_upload_progress: PathConverterFuture,
    },
    /// Tells the event loop that this is the last event of the stream.
    SendLastBuildEvent {
        sequence_number: i64,
        creation_time: Instant,
    },
    /// Tells the event loop to retransmit a serialized build event.
    SendSerializedBuildEvent(StreamEvent),
}

/// `BlockingDeque<Command>` (`LinkedBlockingDeque`): `takeFirst` waits interruptibly.
#[derive(Default)]
struct CommandQueue {
    queue: Mutex<VecDeque<Command>>,
    cond: Condvar,
}

impl CommandQueue {
    fn add_first(&self, command: Command) {
        self.queue.lock().push_front(command);
        self.cond.notify_all();
    }

    fn add_last(&self, command: Command) {
        self.queue.lock().push_back(command);
        self.cond.notify_all();
    }

    fn poll_first(&self) -> Option<Command> {
        self.queue.lock().pop_front()
    }

    fn len(&self) -> usize {
        self.queue.lock().len()
    }

    /// `takeFirst()`: waits for a command; an interrupt of the calling thread ends the wait with
    /// `InterruptedException` (and clears the flag).
    fn take_first(self: &Arc<Self>) -> Result<Command, Interrupted> {
        let thread = ThreadInterrupt::current();
        // The listener only wakes the wait; the flag is checked under the lock.
        let queue = self.clone();
        let _registration = thread.add_interrupt_listener(Arc::new(move || {
            let _guard = queue.queue.lock();
            queue.cond.notify_all();
        }));
        let mut queue = self.queue.lock();
        loop {
            if thread_interrupt::interrupted() {
                return Err(Interrupted);
            }
            if let Some(command) = queue.pop_front() {
                return Ok(command);
            }
            self.cond.wait(&mut queue);
        }
    }
}

/// The state guarded by `lock`.
struct LockedState {
    started_close: bool,
    invocation_status: InvocationStatus,
    /// The thread that calls the lifecycle RPCs and does the build event upload. It's started
    /// lazily on the first call to `enqueue_event` or `close` (which ever comes first).
    upload_thread: Option<ThreadInterrupt>,
    interrupt_caused_by_cancel: bool,
}

/// Why `publishBuildEvents` (or `run`) failed.
enum UploadError {
    /// `DetailedStatusException`.
    DetailedStatus(DetailedStatusException),
    /// `LocalFileUploadException` with its cause's message.
    LocalFileUpload(UploadFailure),
    /// `InterruptedException`.
    Interrupted,
}

impl From<Interrupted> for UploadError {
    fn from(_: Interrupted) -> Self {
        UploadError::Interrupted
    }
}

/// A wrapper of the `StatusException` with the `BuildProgress.Code` and a message.
// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceUploader.java:850 BuildEventServiceUploader.DetailedStatusException
struct DetailedStatusException {
    status: Status,
    bp_code: BuildProgressCode,
    extended_message: String,
}

/// Uploader of Build Events to the Build Event Service (BES).
// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceUploader.java:76 BuildEventServiceUploader
pub struct BuildEventServiceUploader {
    inner: Arc<Inner>,
}

struct Inner {
    bes_client: Arc<dyn BuildEventServiceClient>,
    build_event_uploader: Arc<dyn BuildEventArtifactUploader>,
    build_event_protocol_options: Arc<BuildEventProtocolOptions>,
    command_context: CommandContext,
    publish_lifecycle_events: bool,
    sleeper: Arc<dyn Sleeper>,
    clock: Arc<dyn BesClock>,
    namer: Arc<dyn ArtifactGroupNamer>,
    event_bus: Arc<dyn EventBus>,
    /// An instant in time determined by the build tool's native launcher; matches
    /// `BuildStartingEvent.getRequest().getStartTime()`.
    command_start_time: Instant,
    /// An instant *after* `command_start_time` indicating when the uploader was initialized to
    /// begin reporting build events.
    event_stream_start_time: Instant,
    /// The command queue contains two types of commands: commands containing build events,
    /// sorted by sequence number, to be sent to the server; and commands that are used by
    /// `publish_build_events` to change state.
    command_queue: Arc<CommandQueue>,
    /// Computes sequence numbers for build events. As per the BES protocol, sequence numbers
    /// must be consecutive monotonically increasing natural numbers.
    next_seq_num: AtomicI64,
    lock: Mutex<LockedState>,
    close_future: CloseFuture,
    half_close_future: CloseFuture,
    stream_context: Mutex<Option<Box<dyn StreamContext>>>,
}

/// The builder (`BuildEventServiceUploader.Builder`).
// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceUploader.java:759 BuildEventServiceUploader.Builder
pub struct Builder {
    pub bes_client: Arc<dyn BuildEventServiceClient>,
    pub local_file_uploader: Arc<dyn BuildEventArtifactUploader>,
    pub bep_options: Arc<BuildEventProtocolOptions>,
    pub publish_lifecycle_events: bool,
    pub sleeper: Arc<dyn Sleeper>,
    pub clock: Arc<dyn BesClock>,
    pub artifact_group_namer: Arc<dyn ArtifactGroupNamer>,
    pub event_bus: Arc<dyn EventBus>,
    pub command_context: CommandContext,
    pub command_start_time: Instant,
}

impl Builder {
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceUploader.java:833 BuildEventServiceUploader.Builder.build
    pub fn build(self) -> BuildEventServiceUploader {
        let event_stream_start_time = self.clock.now();
        let inner = Arc::new(Inner {
            bes_client: self.bes_client,
            build_event_uploader: self.local_file_uploader,
            build_event_protocol_options: self.bep_options,
            command_context: self.command_context,
            publish_lifecycle_events: self.publish_lifecycle_events,
            sleeper: self.sleeper,
            clock: self.clock,
            namer: self.artifact_group_namer,
            event_bus: self.event_bus,
            command_start_time: self.command_start_time,
            event_stream_start_time,
            command_queue: Arc::new(CommandQueue::default()),
            next_seq_num: AtomicI64::new(1),
            lock: Mutex::new(LockedState {
                started_close: false,
                invocation_status: InvocationStatus::Unknown,
                upload_thread: None,
                interrupt_caused_by_cancel: false,
            }),
            close_future: CloseFuture::create(),
            half_close_future: CloseFuture::create(),
            stream_context: Mutex::new(None),
        });
        // Ensure the half-close future is closed once the upload is complete. This is usually
        // a no-op, but makes sure we half-close in case of error / interrupt (see `run`).
        BuildEventServiceUploader { inner }
    }
}

impl BuildEventServiceUploader {
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceUploader.java:204 BuildEventServiceUploader.getBuildEventUploader
    pub fn get_build_event_uploader(&self) -> Arc<dyn BuildEventArtifactUploader> {
        self.inner.build_event_uploader.clone()
    }

    /// Enqueues an event for uploading to a BES backend.
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceUploader.java:209 BuildEventServiceUploader.enqueueEvent
    pub fn enqueue_event(&self, event: &Arc<dyn BuildEvent>) {
        let inner = &self.inner;
        // This needs to happen outside a synchronized block as it may trigger stdout/stderr and
        // lead to a deadlock. See b/109725432
        let local_file_upload_future = inner
            .build_event_uploader
            .upload_referenced_local_files(&event.referenced_local_files());

        // The generation of the sequence number and the addition to the command queue should be
        // atomic since BES expects the events in that exact order.
        let mut state = inner.lock.lock();
        if state.started_close {
            return;
        }
        // BuildCompletingEvent marks the end of the build in the BEP event stream.
        if let Some(exit_code) = event.build_completing_exit_code() {
            state.invocation_status = if exit_code == 0 {
                InvocationStatus::Succeeded
            } else {
                InvocationStatus::Failed
            };
        } else if event.is_aborted_event() && is_build_finished_id(event.as_ref()) {
            // An AbortedEvent with a build finished ID means we are crashing.
            state.invocation_status = InvocationStatus::Failed;
        }
        Inner::ensure_upload_thread_started(inner, &mut state);
        inner
            .command_queue
            .add_last(Command::SendRegularBuildEvent {
                sequence_number: inner.next_seq_num.fetch_add(1, Ordering::SeqCst),
                creation_time: inner.clock.now(),
                event: event.clone(),
                local_file_upload_progress: local_file_upload_future,
            });
    }

    /// Gracefully stops the BES upload. All events enqueued before the call to close will be
    /// uploaded and events enqueued after the call will be discarded.
    ///
    /// The returned future completes when the upload completes. It's guaranteed to never fail.
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceUploader.java:250 BuildEventServiceUploader.close
    pub fn close(&self) -> CloseFuture {
        let inner = &self.inner;
        let mut state = inner.lock.lock();
        Inner::ensure_upload_thread_started(inner, &mut state);
        if state.started_close {
            return inner.close_future.clone();
        }
        state.started_close = true;
        // Enqueue the last event which will terminate the upload.
        inner.command_queue.add_last(Command::SendLastBuildEvent {
            sequence_number: inner.next_seq_num.fetch_add(1, Ordering::SeqCst),
            creation_time: inner.clock.now(),
        });
        drop(state);

        // Make sure to cancel any pending uploads if the closing is cancelled.
        let weak = Arc::downgrade(inner);
        inner.close_future.add_cancel_listener(Box::new(move || {
            if let Some(inner) = weak.upgrade() {
                // closeFuture.addListener(() -> halfCloseFuture.setFuture(closeFuture)): a
                // cancelled close future cancels the half-close future (unless it is done).
                inner.half_close_future.cancel();
                inner.close_on_cancel();
            }
        }));
        inner.close_future.clone()
    }

    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceUploader.java:298 BuildEventServiceUploader.getHalfCloseFuture
    pub fn get_half_close_future(&self) -> CloseFuture {
        self.inner.half_close_future.clone()
    }
}

/// `event.getEventId().hasBuildFinished()`.
fn is_build_finished_id(event: &dyn BuildEvent) -> bool {
    matches!(
        event.get_event_id().id,
        Some(bes::build_event_id::Id::BuildFinished(_))
    )
}

/// `RETRYABLE_STATUS_CODES`: every code except `INVALID_ARGUMENT` and `PERMISSION_DENIED`.
// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceUploader.java:819 BuildEventServiceUploader.shouldRetryStatus
fn should_retry_status(status: &Status) -> bool {
    !matches!(
        status.code(),
        tonic::Code::InvalidArgument | tonic::Code::PermissionDenied
    )
}

// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceUploader.java:823 BuildEventServiceUploader.shouldStartNewInvocation
fn should_start_new_invocation(status: &Status) -> bool {
    status.code() == tonic::Code::FailedPrecondition
}

// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceUploader.java:794 BuildEventServiceUploader.lastEventNotSentStatus
fn last_event_not_sent_status() -> Status {
    status_with_description(
        tonic::Code::FailedPrecondition,
        "Server closed stream with status OK but not all events have been sent",
    )
}

// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceUploader.java:799 BuildEventServiceUploader.ackQueueNotEmptyStatus
fn ack_queue_not_empty_status(ack_queue_size: usize) -> Status {
    status_with_description(
        tonic::Code::FailedPrecondition,
        &format!(
            "Server closed stream with status OK but not all ACKs have been received \
             (ackQueue={ack_queue_size})"
        ),
    )
}

impl Inner {
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceUploader.java:675 BuildEventServiceUploader.ensureUploadThreadStarted
    fn ensure_upload_thread_started(this: &Arc<Inner>, state: &mut LockedState) {
        if state.upload_thread.is_none() {
            let (sender, receiver) = std::sync::mpsc::sync_channel(1);
            let inner = this.clone();
            std::thread::Builder::new()
                .name("bes-uploader".to_string())
                .spawn(move || {
                    let _ = sender.send(ThreadInterrupt::current());
                    inner.run();
                })
                .expect("failed to start the bes-uploader thread");
            state.upload_thread = Some(
                receiver
                    .recv()
                    .expect("the bes-uploader thread did not start"),
            );
        }
    }

    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceUploader.java:279 BuildEventServiceUploader.closeOnCancel
    fn close_on_cancel(&self) {
        let mut state = self.lock.lock();
        state.interrupt_caused_by_cancel = true;
        Self::close_now(&state);
    }

    /// Stops the upload immediately. Enqueued events that have not been sent yet will be lost.
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceUploader.java:287 BuildEventServiceUploader.closeNow
    fn close_now(state: &LockedState) {
        if let Some(upload_thread) = &state.upload_thread {
            if upload_thread.is_interrupted() {
                return;
            }
            upload_thread.interrupt();
        }
    }

    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceUploader.java:302 BuildEventServiceUploader.logAndSetException
    fn log_and_set_exception(
        &self,
        message: &str,
        bp_code: BuildProgressCode,
        cause_message: &str,
    ) -> FailureDetail {
        // logger.atSevere(): logging is not ported.
        let failure_detail = FailureDetail {
            message: format!("{message} {cause_message}"),
            category: Some(failure_detail::Category::BuildProgress(
                failure_details::BuildProgress {
                    code: bp_code as i32,
                },
            )),
        };
        // closeFuture.setException(new AbruptExitException(detailedExitCode, cause))
        self.close_future.set(Err(TransportCloseError {
            message: failure_detail.message.clone(),
            code: bp_code.as_str_name(),
        }));
        failure_detail
    }

    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceUploader.java:315 BuildEventServiceUploader.run
    fn run(self: Arc<Self>) {
        let this = self.clone();
        let outcome =
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(move || this.run_inner()));
        match outcome {
            Ok(result) => {
                match result {
                    Ok(()) => {
                        self.event_bus
                            .post(Arc::new(BuildEventServiceAvailabilityEvent::of_success()));
                    }
                    Err(UploadError::Interrupted) => {
                        let caused_by_cancel = self.lock.lock().interrupt_caused_by_cancel;
                        if !caused_by_cancel {
                            // Preconditions.checkState throws from the catch block: the finally
                            // block still runs (the close future completes normally), then the
                            // IllegalStateException ends the thread.
                            self.finish_run(Ok(()));
                            panic!(
                                "IllegalStateException: Unexpected interrupt on BES uploader thread"
                            );
                        }
                    }
                    Err(UploadError::DetailedStatus(e)) => {
                        let is_transient = should_retry_status(&e.status);
                        let exit_code = if is_transient {
                            ExitCode::TRANSIENT_BUILD_EVENT_SERVICE_UPLOAD_ERROR
                        } else {
                            ExitCode::PERSISTENT_BUILD_EVENT_SERVICE_UPLOAD_ERROR
                        };
                        let failure_detail = self.log_and_set_exception(
                            &e.extended_message,
                            e.bp_code,
                            &self.bes_client.user_readable_error(&e.status),
                        );
                        self.event_bus
                            .post(Arc::new(BuildEventServiceAvailabilityEvent::new(
                                exit_code,
                                Some(failure_detail.clone()),
                            )));
                        self.finish_run(Err(close_error(&failure_detail)));
                        return;
                    }
                    Err(UploadError::LocalFileUpload(failure)) => {
                        let cause_message = match failure {
                            // A CancellationException is unchecked: `Future.get()` throws it
                            // directly, and `catch (Throwable e)` fails the close future with it
                            // (whose waiter treats it as a bug).
                            UploadFailure::Cancelled => {
                                self.close_future.set_panicked(Box::new(
                                    "java.util.concurrent.CancellationException: Task was cancelled."
                                        .to_string(),
                                ));
                                self.finish_run(Ok(()));
                                return;
                            }
                            // userReadableError(cause): `cause.getMessage()`.
                            UploadFailure::Failed(e) => io_exception_message(&e),
                        };
                        let failure_detail = self.log_and_set_exception(
                            "The Build Event Protocol local file upload failed:",
                            BuildProgressCode::BesUploadLocalFileError,
                            &cause_message,
                        );
                        self.event_bus
                            .post(Arc::new(BuildEventServiceAvailabilityEvent::new(
                                ExitCode::TRANSIENT_BUILD_EVENT_SERVICE_UPLOAD_ERROR,
                                Some(failure_detail.clone()),
                            )));
                        self.finish_run(Err(close_error(&failure_detail)));
                        return;
                    }
                }
                self.finish_run(Ok(()));
            }
            Err(payload) => {
                // catch (Throwable e): closeFuture.setException(e); throw e (after the finally
                // block). The close future holds the panic for its waiter.
                self.close_future.set_panicked(payload);
                self.finish_run(Ok(()));
            }
        }
    }

    /// The `finally` of `run`: release the uploader and complete the close future (a no-op when
    /// it already failed) and, through its listener (`halfCloseFuture.setFuture(closeFuture)`),
    /// the half-close future with `close_result`, the close future's outcome.
    ///
    /// PORT: when the close future holds a panic (a bug), the half-close future completes
    /// normally.
    fn finish_run(&self, close_result: Result<(), TransportCloseError>) {
        self.build_event_uploader.release();
        // MoreExecutors.shutdownAndAwaitTermination(timeoutExecutor, ...): no executor here.
        self.close_future.set(Ok(()));
        if self.close_future.is_cancelled() {
            self.half_close_future.cancel();
        } else {
            self.half_close_future.set(close_result);
        }
    }

    /// The `try` of `run`, up to the success event.
    fn run_inner(&self) -> Result<(), UploadError> {
        if self.publish_lifecycle_events {
            self.publish_lifecycle_event(&LifecycleEvent::BuildEnqueued {
                event_time: self.command_start_time,
            })?;
            self.publish_lifecycle_event(&LifecycleEvent::InvocationStarted {
                event_time: self.event_stream_start_time,
            })?;
        }
        let published = self.publish_build_events();
        // finally
        if self.publish_lifecycle_events {
            let invocation_status = self.lock.lock().invocation_status;
            let now = self.clock.now();
            self.publish_lifecycle_event(&LifecycleEvent::InvocationFinished {
                event_time: now,
                status: invocation_status,
            })?;
            self.publish_lifecycle_event(&LifecycleEvent::BuildFinished {
                event_time: now,
                status: invocation_status,
            })?;
        }
        published
    }

    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceUploader.java:384 BuildEventServiceUploader.createSerializedRegularBuildEvent
    fn create_serialized_regular_build_event(
        &self,
        path_converter: Box<dyn PathConverter>,
        event: &dyn BuildEvent,
    ) -> Result<bes::BuildEvent, Interrupted> {
        let ctx = UploaderContext {
            output_group_modes: self
                .build_event_protocol_options
                .get_output_group_file_modes_mapping(),
            path_converter,
            namer: self.namer.as_ref(),
            options: &self.build_event_protocol_options,
        };
        let serialized_bep_event = event.as_stream_proto(&ctx).map_err(|_| Interrupted)?;
        // TODO(lpino): Remove this logging once we can make every single event smaller than
        // 1MB as protobuf recommends.
        let size = serialized_bep_event.java_encoded_len();
        if size > LargeBuildEventSerializedEvent::SIZE_OF_LARGE_BUILD_EVENTS_IN_BYTES {
            // serializedBepEvent.getId().toString(): the id in text format.
            let id = serialized_bep_event.id.clone().unwrap_or_default();
            self.event_bus
                .post(Arc::new(LargeBuildEventSerializedEvent::new(
                    blaze_protos::text_format::message_to_string(&id),
                    size as i32,
                )));
        }
        Ok(serialized_bep_event)
    }

    fn send_over_stream(&self, stream_event: StreamEvent) -> Result<(), Interrupted> {
        match &*self.stream_context.lock() {
            Some(stream_context) => stream_context.send_over_stream(stream_event),
            None => panic!("NullPointerException: streamContext"),
        }
    }

    fn abort_stream(&self, status: Status) {
        match &*self.stream_context.lock() {
            Some(stream_context) => stream_context.abort_stream(status),
            None => panic!("NullPointerException: streamContext"),
        }
    }

    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceUploader.java:422 BuildEventServiceUploader.publishBuildEvents
    fn publish_build_events(&self) -> Result<(), UploadError> {
        self.command_queue.add_first(Command::OpenStream);

        // Every build event sent to the server needs to be acknowledged by it. This queue stores
        // the build events that have been sent and still have to be acknowledged by the server.
        // The build events are stored in the order they were sent.
        let mut ack_queue: VecDeque<StreamEvent> = VecDeque::new();
        let mut last_event_sent = false;
        let mut acks_received = 0;
        let mut retry_attempt = 0;
        // `cmd`: the command being processed when an exception ended the loop (only a
        // SendRegularBuildEvent matters, for its uploads; dropping it cancels them).
        let result = (|| -> Result<(), UploadError> {
            loop {
                let cmd = self.command_queue.take_first()?;
                match cmd {
                    Command::OpenStream => {
                        // Invariant: commandQueue only contains commands of type
                        // SendRegularBuildEvent or SendLastBuildEvent.
                        // logger.atInfo(): logging is not ported.
                        let queue = self.command_queue.clone();
                        let stream_context = self.bes_client.open_stream(
                            &self.command_context,
                            Arc::new(move |ack| queue.add_last(Command::AckReceived(ack))),
                        )?;
                        let queue = self.command_queue.clone();
                        add_stream_status_listener(&*stream_context, move |status| {
                            queue.add_last(Command::StreamComplete(status))
                        });
                        *self.stream_context.lock() = Some(stream_context);
                    }
                    Command::SendRegularBuildEvent {
                        sequence_number,
                        creation_time,
                        event,
                        local_file_upload_progress,
                    } => {
                        // Invariant: commandQueue may contain commands of any type
                        let path_converter = self.wait_for_uploads(local_file_upload_progress)?;
                        let serialized_regular_build_event = self
                            .create_serialized_regular_build_event(
                                path_converter,
                                event.as_ref(),
                            )?;
                        let bazel_event = StreamEvent::BazelEvent {
                            event_time: creation_time,
                            sequence_number,
                            payload: serialized_regular_build_event.to_byte_array(),
                        };
                        ack_queue.push_back(bazel_event.clone());
                        self.send_over_stream(bazel_event)?;
                    }
                    Command::SendSerializedBuildEvent(request) => {
                        let is_stream_finished =
                            matches!(request, StreamEvent::StreamFinished { .. });
                        ack_queue.push_back(request.clone());
                        self.send_over_stream(request)?;
                        // Re-close the stream if we are re-sending the last event.
                        if is_stream_finished {
                            self.half_close_event_uploading_stream();
                        }
                    }
                    Command::SendLastBuildEvent {
                        sequence_number,
                        creation_time,
                    } => {
                        // Invariant: the commandQueue may contain commands of any type
                        last_event_sent = true;
                        let stream_finished_event = StreamEvent::StreamFinished {
                            event_time: creation_time,
                            sequence_number,
                        };
                        ack_queue.push_back(stream_finished_event.clone());
                        self.send_over_stream(stream_finished_event)?;
                        self.half_close_event_uploading_stream();
                    }
                    Command::AckReceived(actual_seq_num) => {
                        // Invariant: the commandQueue may contain commands of any type
                        if let Some(expected) = ack_queue.pop_front() {
                            if expected.sequence_number() == actual_seq_num {
                                acks_received += 1;
                            } else {
                                let message = format!(
                                    "Expected ACK with seqNum={} but received ACK with seqNum={}",
                                    expected.sequence_number(),
                                    actual_seq_num
                                );
                                ack_queue.push_front(expected);
                                self.abort_stream(status_with_description(
                                    tonic::Code::FailedPrecondition,
                                    &message,
                                ));
                            }
                        } else {
                            let message = format!(
                                "Received ACK (seqNum={actual_seq_num}) when no ACK was expected"
                            );
                            self.abort_stream(status_with_description(
                                tonic::Code::FailedPrecondition,
                                &message,
                            ));
                        }
                    }
                    Command::StreamComplete(stream_status) => {
                        // Invariant: the commandQueue only contains commands of type
                        // SendRegularBuildEvent or SendLastBuildEvent.
                        *self.stream_context.lock() = None;
                        if stream_status.code() == tonic::Code::Ok {
                            if last_event_sent && ack_queue.is_empty() {
                                // Upload successful. Break out from the while(true) loop.
                                return Ok(());
                            } else {
                                let status = if last_event_sent {
                                    ack_queue_not_empty_status(ack_queue.len())
                                } else {
                                    last_event_not_sent_status()
                                };
                                let bp_code = if last_event_sent {
                                    BuildProgressCode::BesStreamCompletedWithUnackEventsError
                                } else {
                                    BuildProgressCode::BesStreamCompletedWithUnsentEventsError
                                };
                                let description = status.message().to_string();
                                return Err(UploadError::DetailedStatus(self.with_failure_detail(
                                    status,
                                    bp_code,
                                    &description,
                                )));
                            }
                        } else if last_event_sent && ack_queue.is_empty() {
                            let description = description_or_null(&stream_status);
                            return Err(UploadError::DetailedStatus(self.with_failure_detail(
                                stream_status,
                                BuildProgressCode::BesStreamCompletedWithRemoteError,
                                &description,
                            )));
                        }

                        if !should_retry_status(&stream_status)
                            || should_start_new_invocation(&stream_status)
                        {
                            let message = format!(
                                "Not retrying publishBuildEvents: status='{}'",
                                status_to_string(&stream_status)
                            );
                            let detailed_code = if should_start_new_invocation(&stream_status) {
                                BuildProgressCode::BesUploadTimeoutError
                            } else {
                                BuildProgressCode::BesStreamNotRetryingFailure
                            };
                            return Err(UploadError::DetailedStatus(self.with_failure_detail(
                                stream_status,
                                detailed_code,
                                &message,
                            )));
                        }
                        if retry_attempt == self.build_event_protocol_options.bes_upload_max_retries
                        {
                            let message = format!(
                                "Not retrying publishBuildEvents, no more attempts left: \
                                 status='{}'",
                                status_to_string(&stream_status)
                            );
                            return Err(UploadError::DetailedStatus(self.with_failure_detail(
                                stream_status,
                                BuildProgressCode::BesUploadRetryLimitExceededFailure,
                                &message,
                            )));
                        }

                        // Retry logic
                        // Adds build event commands from the ackQueue to the front of the
                        // commandQueue, so that the commands in the commandQueue are sorted by
                        // sequence number (ascending).
                        while let Some(unacked) = ack_queue.pop_back() {
                            self.command_queue
                                .add_first(Command::SendSerializedBuildEvent(unacked));
                        }
                        let sleep_millis = self.retry_sleep_millis(retry_attempt);
                        self.sleeper.sleep_millis(sleep_millis)?;
                        // If we made progress, meaning the server ACKed events that we sent,
                        // then reset the retry counter to 0.
                        if acks_received > 0 {
                            retry_attempt = 0;
                        } else {
                            retry_attempt += 1;
                        }
                        acks_received = 0;
                        self.command_queue.add_first(Command::OpenStream);
                    }
                }
            }
        })();
        if matches!(
            result,
            Err(UploadError::Interrupted | UploadError::LocalFileUpload(_))
        ) {
            // logger.atInfo() of the queues: logging is not ported.
            let stream_context = self.stream_context.lock();
            if let Some(stream_context) = &*stream_context {
                stream_context.abort_stream(Status::cancelled(""));
            }
        }
        // finally: cancel all pending local file uploads. Dropping a command's future cancels
        // its upload (the uploader's promise is then completed into nothing).
        ack_queue.clear();
        while self.command_queue.poll_first().is_some() {}
        let _ = self.command_queue.len();
        result
    }

    /// Half-closes the uploading stream, which can happen when we send the final event or when
    /// we re-send the final event.
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceUploader.java:634 BuildEventServiceUploader.halfCloseEventUploadingStream
    fn half_close_event_uploading_stream(&self) {
        if let Some(stream_context) = &*self.stream_context.lock() {
            stream_context.half_close_stream();
        }
        self.half_close_future.set(Ok(()));
        // logger.atInfo(): logging is not ported.
    }

    /// Sends a [`LifecycleEvent`] to the BES backend.
    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceUploader.java:648 BuildEventServiceUploader.publishLifecycleEvent
    fn publish_lifecycle_event(&self, lifecycle_event: &LifecycleEvent) -> Result<(), UploadError> {
        let mut retry_attempt = 0;
        let mut cause: Option<Status> = None;
        while retry_attempt <= self.build_event_protocol_options.bes_upload_max_retries {
            match self
                .bes_client
                .publish(&self.command_context, lifecycle_event)
            {
                Ok(()) => return Ok(()),
                Err(PublishError::Interrupted) => return Err(UploadError::Interrupted),
                Err(PublishError::Status(status)) => {
                    if !should_retry_status(&status) || should_start_new_invocation(&status) {
                        let message = format!(
                            "Not retrying publishLifecycleEvent: status='{}'",
                            status_to_string(&status)
                        );
                        return Err(UploadError::DetailedStatus(self.with_failure_detail(
                            status,
                            BuildProgressCode::BesStreamNotRetryingFailure,
                            &message,
                        )));
                    }
                    let sleep_millis = self.retry_sleep_millis(retry_attempt);
                    cause = Some(status);
                    self.sleeper.sleep_millis(sleep_millis)?;
                    retry_attempt += 1;
                }
            }
        }
        // All retry attempts failed
        let cause = cause.expect("NullPointerException: no retry attempt was made");
        Err(UploadError::DetailedStatus(self.with_failure_detail(
            cause,
            BuildProgressCode::BesUploadRetryLimitExceededFailure,
            &format!("All {} retry attempts failed.", retry_attempt - 1),
        )))
    }

    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceUploader.java:684 BuildEventServiceUploader.waitForUploads
    fn wait_for_uploads(
        &self,
        local_file_upload_progress: PathConverterFuture,
    ) -> Result<Box<dyn PathConverter>, UploadError> {
        // Wait for the local file and pending remote uploads to complete.
        // PORT: waitForRemoteUploads: an event's remote uploads are resolved values
        // (`BuildEvent::remote_uploads`). In Bazel only `BuildToolLogs` has remote uploads, the
        // URI futures of logs uploaded while they are written (`addUriFuture`: the Skyframe
        // memory dump with --experimental_stream_log_file_uploads, ExecutionGraphModule's dump,
        // BuildEventArtifactInstrumentationOutput); each of those fails loudly in Blaze, so no
        // remote upload can be pending here.
        //
        // localFileUploadProgress().get(): an interrupt of this thread (a cancelled close)
        // ends the wait with InterruptedException; the dropped future cancels the upload.
        let mut future = local_file_upload_progress;
        loop {
            if thread_interrupt::interrupted() {
                return Err(UploadError::Interrupted);
            }
            if let Some(result) = future.get_timeout(Duration::from_millis(10)) {
                return result.map_err(|failure| {
                    // logger.atWarning(): logging is not ported.
                    UploadError::LocalFileUpload(failure)
                });
            }
        }
    }

    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceUploader.java:827 BuildEventServiceUploader.retrySleepMillis
    fn retry_sleep_millis(&self, attempt: i32) -> i64 {
        blaze_util::check_argument!(attempt >= 0, "attempt must be nonnegative: {}", attempt);
        // This somewhat matches the backoff used for gRPC connection backoffs.
        (self
            .build_event_protocol_options
            .bes_upload_retry_initial_delay
            .as_millis() as f64
            * 1.6f64.powi(attempt)) as i64
    }

    // Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceUploader.java:835 BuildEventServiceUploader.withFailureDetail
    fn with_failure_detail(
        &self,
        status: Status,
        bp_code: BuildProgressCode,
        message: &str,
    ) -> DetailedStatusException {
        let message = format!("{message} {}", self.bes_client.user_readable_error(&status));
        DetailedStatusException {
            status,
            bp_code,
            extended_message: format!("The Build Event Protocol upload failed: {message}"),
        }
    }
}

/// The close failure of an `AbruptExitException` with `failure_detail`.
fn close_error(failure_detail: &FailureDetail) -> TransportCloseError {
    let code = match &failure_detail.category {
        Some(failure_detail::Category::BuildProgress(p)) => {
            BuildProgressCode::try_from(p.code).unwrap_or(BuildProgressCode::BuildProgressUnknown)
        }
        _ => BuildProgressCode::BuildProgressUnknown,
    };
    TransportCloseError {
        message: failure_detail.message.clone(),
        code: code.as_str_name(),
    }
}

/// `Status.getDescription()` as `String.valueOf` prints it.
fn description_or_null(status: &Status) -> String {
    if status.message().is_empty() {
        "null".to_string()
    } else {
        status.message().to_string()
    }
}

/// `IOException.getMessage()` of an upload failure.
fn io_exception_message(e: &std::io::Error) -> String {
    match e.get_ref().and_then(|r| {
        r.downcast_ref::<blaze_bep::buildeventstream::artifact_uploader::JavaThrowable>()
    }) {
        Some(java) => java.to_string(),
        None => e.to_string(),
    }
}

// Java: com/google/devtools/build/lib/buildeventservice/BuildEventServiceUploader.java:804 BuildEventServiceUploader.addStreamStatusListener
fn add_stream_status_listener(
    stream_context: &dyn StreamContext,
    on_done: impl FnOnce(Status) + Send + 'static,
) {
    stream_context.get_status().add_listener(on_done);
}

/// The `BuildEventContext` of `createSerializedRegularBuildEvent`.
struct UploaderContext<'a> {
    output_group_modes: OutputGroupFileModes,
    path_converter: Box<dyn PathConverter>,
    namer: &'a dyn ArtifactGroupNamer,
    options: &'a BuildEventProtocolOptions,
}

impl BuildEventContext for UploaderContext<'_> {
    fn path_converter(&self) -> &dyn PathConverter {
        self.path_converter.as_ref()
    }
    fn artifact_group_namer(&self) -> &dyn ArtifactGroupNamer {
        self.namer
    }
    fn get_options(&self) -> &BuildEventProtocolOptions {
        self.options
    }
    fn get_file_mode_for_output_group(&self, output_group: &str) -> OutputGroupFileMode {
        self.output_group_modes.get_mode(output_group)
    }
}
