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
pub struct CloseFuture(Arc<(parking_lot::Mutex<CloseState>, parking_lot::Condvar)>);

enum CloseState {
    Pending,
    Done(Result<(), TransportCloseError>),
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
    }

    /// Completes the future with a panic of the closing thread.
    pub fn set_panicked(&self, payload: Box<dyn Any + Send>) {
        self.complete(CloseState::Panicked(Some(payload)));
    }

    fn complete(&self, state: CloseState) {
        let mut guard = self.0.0.lock();
        if matches!(*guard, CloseState::Pending) {
            *guard = state;
            self.0.1.notify_all();
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
