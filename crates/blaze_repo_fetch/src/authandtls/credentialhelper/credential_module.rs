//! Port of `CredentialModule`.

use std::sync::{Arc, LazyLock};
use std::time::Duration;

use blaze_events::ui::{Clock, JavaClock};
use parking_lot::Mutex;

use super::credential_cache::CredentialCache;
use super::credential_cache_expiry::CredentialCacheExpiry;
use super::wall_ticker::WallTicker;

/// A module whose sole purpose is to hold the credential cache which is shared by other modules.
///
/// PORT: the `BlazeModule` side (registration, `getCommonCommandOptions`) is
/// `blaze_runtime::bazel_modules::CredentialModule`; this struct holds the module's state, which
/// the repository and remote set-up read and which `blaze_runtime` cannot hand them (it does not
/// depend on this crate). Java's users reach the one instance with
/// `runtime.getBlazeModule(CredentialModule.class)`; here that is [`CredentialModule::instance`]
/// (one runtime per process).
// Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialModule.java:31 CredentialModule
pub struct CredentialModule {
    credential_cache_expiry: Arc<CredentialCacheExpiry>,
    credential_cache: Arc<CredentialCache>,
    last_default_cache_duration: Mutex<Duration>,
}

static INSTANCE: LazyLock<CredentialModule> = LazyLock::new(CredentialModule::new);

impl CredentialModule {
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialModule.java:36 CredentialModule.<init>
    pub fn new() -> CredentialModule {
        CredentialModule::with_clock(Arc::new(JavaClock))
    }

    /// `@VisibleForTesting CredentialModule(Clock clock)`.
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialModule.java:41 CredentialModule.<init>
    pub fn with_clock(clock: Arc<dyn Clock>) -> CredentialModule {
        let credential_cache_expiry = Arc::new(CredentialCacheExpiry::new());
        let credential_cache = Arc::new(CredentialCache::new(
            Arc::new(WallTicker::new(clock)),
            credential_cache_expiry.clone(),
        ));
        CredentialModule {
            credential_cache_expiry,
            credential_cache,
            last_default_cache_duration: Mutex::new(Duration::ZERO),
        }
    }

    /// The module of this process (`runtime.getBlazeModule(CredentialModule.class)`).
    pub fn instance() -> &'static CredentialModule {
        &INSTANCE
    }

    /// Returns the credential cache.
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialModule.java:51 CredentialModule.getCredentialCache
    pub fn get_credential_cache(&self) -> &Arc<CredentialCache> {
        &self.credential_cache
    }

    /// `beforeCommand(env)`.
    ///
    /// * `command_name`: `env.getCommandName()`.
    /// * `default_cache_duration`: `env.getOptions().getOptions(AuthAndTLSOptions.class)
    ///   .credentialHelperCacheTimeout` (`--credential_helper_cache_duration`).
    ///
    /// `blaze_runtime::bazel_modules::CredentialModule::before_command` calls this on
    /// [`CredentialModule::instance`] for every command.
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialModule.java:61 CredentialModule.beforeCommand
    pub fn before_command(&self, command_name: &str, default_cache_duration: Duration) {
        let default_cache_duration_changed = {
            let mut last = self.last_default_cache_duration.lock();
            let changed = default_cache_duration != *last;
            *last = default_cache_duration;
            changed
        };

        // Clear the cache on clean or when the default cache duration changes.
        if command_name == "clean" || default_cache_duration_changed {
            self.credential_cache.invalidate_all();
            self.credential_cache.clean_up();
        }

        // Update the expiration policy for future entries.
        self.credential_cache_expiry
            .set_default_cache_duration(default_cache_duration);
    }
}
