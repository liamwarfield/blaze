//! Port of `lib/authandtls/CallCredentialsProvider.java` and
//! `lib/authandtls/GoogleAuthCallCredentialsProvider.java`.

use std::io;
use std::sync::{Arc, LazyLock};
use std::time::{Duration, Instant};

use parking_lot::Mutex;

use super::Credentials;
use super::google_auth_utils::GoogleAuthLibraryCallCredentials;
use crate::grpc::CallCredentials;

/// Interface for providing [`CallCredentials`]. Implementations must be thread-safe.
// Java: com/google/devtools/build/lib/authandtls/CallCredentialsProvider.java:22 CallCredentialsProvider
pub trait CallCredentialsProvider: Send + Sync {
    /// Returns the current [`CallCredentials`]. May be `None`, in which case no authentication
    /// is required.
    // Java: com/google/devtools/build/lib/authandtls/CallCredentialsProvider.java:28 CallCredentialsProvider.getCallCredentials
    fn get_call_credentials(&self) -> Option<Arc<dyn CallCredentials>>;

    /// Refresh the authorization data, discarding any cached state.
    ///
    /// For use by the transport to allow retry after getting an error indicating there may be
    /// invalid tokens or other cached state.
    ///
    /// Fails if there was an error getting up-to-date access.
    // Java: com/google/devtools/build/lib/authandtls/CallCredentialsProvider.java:38 CallCredentialsProvider.refresh
    fn refresh(&self) -> io::Result<()>;
}

/// A no-op implementation that has no credentials and performs no refreshes.
// Java: com/google/devtools/build/lib/authandtls/CallCredentialsProvider.java:41 CallCredentialsProvider.NoCredentials
#[derive(Debug, Clone, Copy, Default)]
pub struct NoCredentials;

impl CallCredentialsProvider for NoCredentials {
    // Java: com/google/devtools/build/lib/authandtls/CallCredentialsProvider.java:45 CallCredentialsProvider.NoCredentials.getCallCredentials
    fn get_call_credentials(&self) -> Option<Arc<dyn CallCredentials>> {
        None
    }

    // Java: com/google/devtools/build/lib/authandtls/CallCredentialsProvider.java:50 CallCredentialsProvider.NoCredentials.refresh
    fn refresh(&self) -> io::Result<()> {
        Ok(())
    }
}

/// `CallCredentialsProvider.NO_CREDENTIALS`.
// Java: com/google/devtools/build/lib/authandtls/CallCredentialsProvider.java:53 CallCredentialsProvider.NO_CREDENTIALS
pub static NO_CREDENTIALS: LazyLock<Arc<dyn CallCredentialsProvider>> =
    LazyLock::new(|| -> Arc<dyn CallCredentialsProvider> { Arc::new(NoCredentials) });

/// A [`CallCredentialsProvider`] implementation which uses [`Credentials`].
// Java: com/google/devtools/build/lib/authandtls/GoogleAuthCallCredentialsProvider.java:28 GoogleAuthCallCredentialsProvider
pub struct GoogleAuthCallCredentialsProvider {
    credentials: Arc<dyn Credentials>,
    call_credentials: Arc<dyn CallCredentials>,
    /// Guava `Stopwatch.createStarted()`: the instant of the last `reset().start()`. The lock is
    /// Java's `synchronized (this)`.
    refresh_stopwatch: Mutex<Instant>,
}

impl GoogleAuthCallCredentialsProvider {
    // Java: com/google/devtools/build/lib/authandtls/GoogleAuthCallCredentialsProvider.java:34 GoogleAuthCallCredentialsProvider.<init>
    pub fn new(credentials: Arc<dyn Credentials>) -> GoogleAuthCallCredentialsProvider {
        // callCredentials = MoreCallCredentials.from(credentials);
        let call_credentials: Arc<dyn CallCredentials> =
            Arc::new(GoogleAuthLibraryCallCredentials::new(credentials.clone()));
        GoogleAuthCallCredentialsProvider {
            credentials,
            call_credentials,
            refresh_stopwatch: Mutex::new(Instant::now()),
        }
    }
}

impl CallCredentialsProvider for GoogleAuthCallCredentialsProvider {
    // Java: com/google/devtools/build/lib/authandtls/GoogleAuthCallCredentialsProvider.java:43 GoogleAuthCallCredentialsProvider.getCallCredentials
    fn get_call_credentials(&self) -> Option<Arc<dyn CallCredentials>> {
        Some(self.call_credentials.clone())
    }

    // Java: com/google/devtools/build/lib/authandtls/GoogleAuthCallCredentialsProvider.java:48 GoogleAuthCallCredentialsProvider.refresh
    fn refresh(&self) -> io::Result<()> {
        let mut refresh_stopwatch = self.refresh_stopwatch.lock();
        // Call credentials.refresh() at most once per second. The one second was arbitrarily
        // chosen, as a small enough value that we don't expect to interfere with actual token
        // lifetimes, but it should just make sure that potentially hundreds of threads don't call
        // this method at the same time.
        if refresh_stopwatch.elapsed() > Duration::from_secs(1) {
            self.credentials.refresh()?;
            *refresh_stopwatch = Instant::now();
        }
        Ok(())
    }
}
