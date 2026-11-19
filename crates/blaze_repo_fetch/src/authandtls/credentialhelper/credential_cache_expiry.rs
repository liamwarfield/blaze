//! Port of `CredentialCacheExpiry`.

use std::time::Duration;

use parking_lot::Mutex;

use super::credential_cache::Expiry;
use super::get_credentials_response::GetCredentialsResponse;
use super::java_time::Instant;
use super::unchecked_exception::UncheckedException;
use crate::uri::JavaUri;

/// Java `Duration.toNanos()`: fails with an `ArithmeticException` outside a `long`.
fn to_nanos(nanos: i128) -> i64 {
    i64::try_from(nanos).unwrap_or_else(|_| {
        UncheckedException::throw("java.lang.ArithmeticException", "long overflow")
    })
}

/// The expiration policy of the credential cache: a response expires at its `expires` time, or
/// after the default cache duration (`--credential_helper_cache_duration`) when it has none.
// Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialCacheExpiry.java:26 CredentialCacheExpiry
pub struct CredentialCacheExpiry {
    /// Java starts at `Duration.ZERO` and `CredentialModule.beforeCommand` sets the option's
    /// value before every command.
    ///
    /// PORT: `None` until the first `set_default_cache_duration`, i.e. until
    /// `CredentialModule.before_command` ran (`blaze_runtime`'s `CredentialModule` calls it for
    /// every command). A caller that skips it would get Java's `ZERO` silently (one helper run
    /// per request); here a response without `expires` then fails loudly.
    default_cache_duration: Mutex<Option<Duration>>,
}

impl CredentialCacheExpiry {
    pub fn new() -> CredentialCacheExpiry {
        CredentialCacheExpiry {
            default_cache_duration: Mutex::new(None),
        }
    }

    /// Sets the default cache duration for [`GetCredentialsResponse`]s that don't set `expires`.
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialCacheExpiry.java:33 CredentialCacheExpiry.setDefaultCacheDuration
    pub fn set_default_cache_duration(&self, duration: Duration) {
        *self.default_cache_duration.lock() = Some(duration);
    }

    /// The cache duration, in nanoseconds (Java `Duration` then `toNanos()`).
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialCacheExpiry.java:37 CredentialCacheExpiry.getCacheDuration
    fn get_cache_duration_nanos(&self, response: &GetCredentialsResponse, now: Instant) -> i64 {
        let Some(expires) = response.expires() else {
            let default_cache_duration = *self.default_cache_duration.lock();
            return match default_cache_duration {
                Some(duration) => to_nanos(duration.as_nanos() as i128),
                None => blaze_util::unported!(
                    "CredentialModule.beforeCommand (missing hook: \
                     blaze_repo_fetch::authandtls::credentialhelper::CredentialModule::\
                     instance().before_command(command name, --credential_helper_cache_duration) \
                     was not called before a credential was cached)"
                ),
            };
        };

        // Duration.between(now, expires.get())
        to_nanos(expires.epoch_nanos() - now.epoch_nanos())
    }
}

impl Expiry for CredentialCacheExpiry {
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialCacheExpiry.java:48 CredentialCacheExpiry.expireAfterCreate
    fn expire_after_create(
        &self,
        _uri: &JavaUri,
        response: &GetCredentialsResponse,
        current_time: i64,
    ) -> i64 {
        // currentTime is in nanos since epoch (see WallTicker).
        let now = Instant::of_epoch_second_and_nanos(0, current_time);

        self.get_cache_duration_nanos(response, now)
    }

    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialCacheExpiry.java:59 CredentialCacheExpiry.expireAfterUpdate
    fn expire_after_update(
        &self,
        _uri: &JavaUri,
        response: &GetCredentialsResponse,
        current_time: i64,
        _current_duration: i64,
    ) -> i64 {
        // currentTime is in nanos since epoch (see WallTicker).
        let now = Instant::of_epoch_second_and_nanos(0, current_time);

        self.get_cache_duration_nanos(response, now)
    }

    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialCacheExpiry.java:71 CredentialCacheExpiry.expireAfterRead
    fn expire_after_read(
        &self,
        _uri: &JavaUri,
        _response: &GetCredentialsResponse,
        _current_time: i64,
        current_duration: i64,
    ) -> i64 {
        // Don't extend the duration on read access.
        current_duration
    }
}
