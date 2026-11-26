//! Port of `BuildEventTransport`.

use std::any::Any;
use std::sync::Arc;

use blaze_optdefs::misc::build_event_service_options::BesUploadMode;

use super::artifact_uploader::BuildEventArtifactUploader;
use super::build_event::BuildEvent;

/// The failure of closing a transport (`AbruptExitException` with a `BuildProgress` failure
/// detail in Java).
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct TransportCloseError {
    pub message: String,
    /// `FailureDetails.BuildProgress.Code` name.
    pub code: &'static str,
}

/// The future `BuildEventTransport.close()` returns (`ListenableFuture<Void>`): completed when
/// the transport has finished closing, with its failure (`AbruptExitException`) if any.
#[derive(Clone)]
pub struct CloseFuture(
    Arc<(
        parking_lot::Mutex<CloseState>,
        parking_lot::Condvar,
        parking_lot::Mutex<Vec<CancelListener>>,
    )>,
);

/// A listener of [`CloseFuture::add_cancel_listener`].
type CancelListener = Box<dyn FnOnce() + Send>;

fn cancel_listeners(future: &CloseFuture) -> &parking_lot::Mutex<Vec<CancelListener>> {
    &future.0.2
}

/// What waiting for a [`CloseFuture`] gave (`Future.get(timeout)` and its exceptions).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CloseOutcome {
    /// The close completed (with its failure, if any).
    Done(Result<(), TransportCloseError>),
    /// `CancellationException`: the future was cancelled.
    Cancelled,
    /// `TimeoutException`: the wait timed out.
    TimedOut,
}

enum CloseState {
    Pending,
    Done(Result<(), TransportCloseError>),
    /// `cancel(true)`.
    Cancelled,
    /// `closeFuture.setException(e.getCause())` for a `RuntimeException` or `Error`: the panic of
    /// the closing thread, re-raised by the first `get`.
    Panicked(Option<Box<dyn Any + Send>>),
}

impl CloseFuture {
    /// `SettableFuture.create()`.
    pub fn create() -> CloseFuture {
        CloseFuture(Arc::new((
            parking_lot::Mutex::new(CloseState::Pending),
            parking_lot::Condvar::new(),
            parking_lot::Mutex::new(Vec::new()),
        )))
    }

    /// `Futures.immediateFuture(null)` / `immediateFailedFuture(e)`.
    pub fn immediate(result: Result<(), TransportCloseError>) -> CloseFuture {
        let future = CloseFuture::create();
        future.set(result);
        future
    }

    /// `set(null)` / `setException(e)`; the first completion wins.
    pub fn set(&self, result: Result<(), TransportCloseError>) {
        self.complete(CloseState::Done(result));
        // A completed future is never cancelled: its cancel listeners are dropped.
        cancel_listeners(self).lock().clear();
    }

    /// Completes the future with a panic of the closing thread.
    pub fn set_panicked(&self, payload: Box<dyn Any + Send>) {
        self.complete(CloseState::Panicked(Some(payload)));
    }

    fn complete(&self, state: CloseState) -> bool {
        let mut guard = self.0.0.lock();
        if matches!(*guard, CloseState::Pending) {
            *guard = state;
            self.0.1.notify_all();
            true
        } else {
            false
        }
    }

    /// `cancel(true)`: false when the future is already done. The cancel listeners run (on the
    /// calling thread) when it was pending.
    pub fn cancel(&self) -> bool {
        let cancelled = self.complete(CloseState::Cancelled);
        if cancelled {
            let listeners = std::mem::take(&mut *cancel_listeners(self).lock());
            for listener in listeners {
                listener();
            }
        }
        cancelled
    }

    /// `isCancelled()`.
    pub fn is_cancelled(&self) -> bool {
        matches!(*self.0.0.lock(), CloseState::Cancelled)
    }

    /// `addListener(() -> { if (future.isCancelled()) listener.run(); })`: runs `listener` when
    /// the future is cancelled (at once if it already is).
    pub fn add_cancel_listener(&self, listener: Box<dyn FnOnce() + Send>) {
        {
            let mut listeners = cancel_listeners(self).lock();
            if !self.is_done() {
                listeners.push(listener);
                return;
            }
        }
        if self.is_cancelled() {
            listener();
        }
    }

    /// `get(timeout)` (`timeout` `None`: `get()`), reporting a cancellation and a timeout
    /// instead of throwing them.
    pub fn get_outcome(&self, timeout: Option<std::time::Duration>) -> CloseOutcome {
        let deadline = timeout.map(|t| std::time::Instant::now() + t);
        let mut guard = self.0.0.lock();
        while matches!(*guard, CloseState::Pending) {
            match deadline {
                None => self.0.1.wait(&mut guard),
                Some(deadline) => {
                    if self.0.1.wait_until(&mut guard, deadline).timed_out()
                        && matches!(*guard, CloseState::Pending)
                    {
                        return CloseOutcome::TimedOut;
                    }
                }
            }
        }
        match &mut *guard {
            CloseState::Done(result) => CloseOutcome::Done(result.clone()),
            CloseState::Cancelled => CloseOutcome::Cancelled,
            CloseState::Panicked(payload) => match payload.take() {
                Some(payload) => {
                    drop(guard);
                    std::panic::resume_unwind(payload)
                }
                None => panic!("the BEP transport's close failed with a panic"),
            },
            CloseState::Pending => unreachable!(),
        }
    }

    /// `isDone()`.
    pub fn is_done(&self) -> bool {
        !matches!(*self.0.0.lock(), CloseState::Pending)
    }

    /// `get()`: waits for the close to complete.
    pub fn get(&self) -> Result<(), TransportCloseError> {
        let mut guard = self.0.0.lock();
        while matches!(*guard, CloseState::Pending) {
            self.0.1.wait(&mut guard);
        }
        match &mut *guard {
            CloseState::Done(result) => result.clone(),
            // PORT: `get()` of a cancelled future throws `CancellationException`, which the only
            // caller (`waitForBuildEventTransportsToClose`) expects and ignores.
            CloseState::Cancelled => Ok(()),
            CloseState::Panicked(payload) => match payload.take() {
                Some(payload) => {
                    drop(guard);
                    std::panic::resume_unwind(payload)
                }
                None => panic!("the BEP transport's close failed with a panic"),
            },
            CloseState::Pending => unreachable!(),
        }
    }
}

impl std::fmt::Debug for CloseFuture {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(if self.is_done() {
            "CloseFuture(done)"
        } else {
            "CloseFuture(pending)"
        })
    }
}

/// Interface for transports of the build-event stream.
// Java: com/google/devtools/build/lib/buildeventstream/BuildEventTransport.java:31 BuildEventTransport
pub trait BuildEventTransport: Send + Sync {
    /// The name of this transport as can be displayed to a user.
    // Java: com/google/devtools/build/lib/buildeventstream/BuildEventTransport.java:33 BuildEventTransport.name
    fn name(&self) -> String;

    /// Writes a build event to an endpoint. This method will always return quickly and will not
    /// wait for the write to complete.
    // Java: com/google/devtools/build/lib/buildeventstream/BuildEventTransport.java:44 BuildEventTransport.sendBuildEvent
    fn send_build_event(&self, event: &Arc<dyn BuildEvent>);

    /// Initiates a close. Callers may listen to the returned future to be notified when the
    /// close is complete.
    // Java: com/google/devtools/build/lib/buildeventstream/BuildEventTransport.java:55 BuildEventTransport.close
    fn close(&self) -> CloseFuture;

    /// Returns the status of half-close: all client-side data is transmitted but the transport
    /// is still waiting on server-side acknowledgement. Implementations without a sensible
    /// half-close state return the full close future. Only called after `close`.
    // Java: com/google/devtools/build/lib/buildeventstream/BuildEventTransport.java:72 BuildEventTransport.getHalfCloseFuture
    fn get_half_close_future(&self) -> CloseFuture {
        self.close()
    }

    /// Returns how long a caller should wait for the transport to finish uploading events and
    /// closing gracefully. Setting the timeout to `0` means no wait.
    // Java: com/google/devtools/build/lib/buildeventstream/BuildEventTransport.java:78 BuildEventTransport.getTimeout
    fn get_timeout(&self) -> std::time::Duration {
        std::time::Duration::ZERO
    }

    // Java: com/google/devtools/build/lib/buildeventstream/BuildEventTransport.java:86 BuildEventTransport.mayBeSlow
    fn may_be_slow(&self) -> bool;

    /// Which `BesUploadMode` the transport is using.
    // Java: com/google/devtools/build/lib/buildeventstream/BuildEventTransport.java:89 BuildEventTransport.getBesUploadMode
    fn get_bes_upload_mode(&self) -> BesUploadMode;

    /// Returns the artifact uploader used by this transport, if any.
    // Java: com/google/devtools/build/lib/buildeventstream/BuildEventTransport.java:92 BuildEventTransport.getUploader
    fn get_uploader(&self) -> Option<Arc<dyn BuildEventArtifactUploader>>;
}
