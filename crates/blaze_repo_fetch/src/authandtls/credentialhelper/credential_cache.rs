//! The credential cache: `CredentialModule`'s Caffeine cache of [`GetCredentialsResponse`]s by
//! URI, with `CredentialCacheExpiry` and `WallTicker`.

use std::collections::HashMap;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use blaze_events::{Clock, JavaClock};
use parking_lot::Mutex;

use super::GetCredentialsResponse;
use crate::errors::IoException;
use crate::uri::JavaUri;

/// WallTicker is a Ticker which reports wall time since the unix epoch.
///
/// We use this instead of Caffeine's `Ticker.SystemTicker` because the latter uses monotonic
/// time (which doesn't increment the time source while the system is asleep) with an
/// unspecified reference point (which is unhelpful when computing the cache duration for
/// credentials whose expiry is a fixed point in time, not a fixed duration).
// Java: com/google/devtools/build/lib/authandtls/credentialhelper/WallTicker.java:29 WallTicker
pub struct WallTicker {
    clock: Arc<dyn Clock>,
}

impl WallTicker {
    pub fn new(clock: Arc<dyn Clock>) -> WallTicker {
        WallTicker { clock }
    }

    /// Documented to return a value in nanoseconds.
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/WallTicker.java:37 WallTicker.read
    pub fn read(&self) -> i64 {
        // Duration.ofMillis(clock.currentTimeMillis()).toNanos()
        self.clock
            .current_time_millis()
            .checked_mul(1_000_000)
            .expect("java.lang.ArithmeticException: long overflow")
    }
}

/// The expiry policy of the credential cache: an entry expires at the response's `expires`, or
/// after the default cache duration when the response has none. A read does not extend it.
// Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialCacheExpiry.java:28 CredentialCacheExpiry
#[derive(Debug, Default)]
pub struct CredentialCacheExpiry {
    default_cache_duration: Mutex<Duration>,
}

impl CredentialCacheExpiry {
    /// Sets the default cache duration for responses that don't set `expiry`.
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialCacheExpiry.java:35 CredentialCacheExpiry.setDefaultCacheDuration
    pub fn set_default_cache_duration(&self, duration: Duration) {
        *self.default_cache_duration.lock() = duration;
    }

    /// The cache duration in nanoseconds (`Duration.toNanos()`, which throws on overflow).
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialCacheExpiry.java:39 CredentialCacheExpiry.getCacheDuration
    fn get_cache_duration_nanos(&self, response: &GetCredentialsResponse, now_nanos: i64) -> i64 {
        let nanos: i128 = match response.expires() {
            None => self.default_cache_duration.lock().as_nanos() as i128,
            // Duration.between(now, expires)
            Some(expires) => expires.epoch_nanos() - now_nanos as i128,
        };
        i64::try_from(nanos).expect("java.lang.ArithmeticException: long overflow")
    }

    /// `currentTime` is in nanos since epoch (see [`WallTicker`]).
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialCacheExpiry.java:51 CredentialCacheExpiry.expireAfterCreate
    pub fn expire_after_create(
        &self,
        _uri: &JavaUri,
        response: &GetCredentialsResponse,
        current_time: i64,
    ) -> i64 {
        self.get_cache_duration_nanos(response, current_time)
    }

    /// `currentTime` is in nanos since epoch (see [`WallTicker`]).
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialCacheExpiry.java:62 CredentialCacheExpiry.expireAfterUpdate
    pub fn expire_after_update(
        &self,
        _uri: &JavaUri,
        response: &GetCredentialsResponse,
        current_time: i64,
        _current_duration: i64,
    ) -> i64 {
        self.get_cache_duration_nanos(response, current_time)
    }

    /// Don't extend the duration on read access.
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialCacheExpiry.java:73 CredentialCacheExpiry.expireAfterRead
    pub fn expire_after_read(
        &self,
        _uri: &JavaUri,
        _response: &GetCredentialsResponse,
        _current_time: i64,
        current_duration: i64,
    ) -> i64 {
        current_duration
    }
}

/// Caffeine's `BoundedLocalCache.MAXIMUM_EXPIRY` (`Long.MAX_VALUE >> 1`): the longest duration
/// an entry is kept.
const MAXIMUM_EXPIRY: i64 = i64::MAX >> 1;

/// An entry of the cache, as `cache.policy().getEntryIfPresentQuietly(uri)` returns it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheEntry {
    pub value: GetCredentialsResponse,
    /// `CacheEntry.expiresAt()`: the ticker time (nanoseconds since the epoch) the entry expires
    /// at.
    pub expires_at: i64,
}

/// One key's slot: Caffeine computes the value of a key atomically (a second caller for the
/// same key waits for the first computation), so each key has its own lock.
type Slot = Arc<Mutex<Option<CacheEntry>>>;

/// `Caffeine.newBuilder().ticker(new WallTicker(clock)).expireAfter(credentialCacheExpiry)
/// .build()`: a `Cache<URI, GetCredentialsResponse>` with per-entry expiry.
///
/// PORT: the operations Bazel uses (`get(key, mappingFunction)`, `put`, `invalidateAll`,
/// `cleanUp`, `policy().getEntryIfPresentQuietly`) with Caffeine's expiry rules: an entry with
/// expiry time `t` is absent from `now >= t`, a duration is capped at `MAXIMUM_EXPIRY`, and a
/// read does not change it. No size bound (Bazel sets none).
pub struct CredentialCache {
    ticker: WallTicker,
    expiry: Arc<CredentialCacheExpiry>,
    slots: Mutex<HashMap<JavaUri, Slot>>,
}

impl std::fmt::Debug for CredentialCache {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("CredentialCache").finish_non_exhaustive()
    }
}

impl CredentialCache {
    pub fn new(ticker: WallTicker, expiry: Arc<CredentialCacheExpiry>) -> CredentialCache {
        CredentialCache {
            ticker,
            expiry,
            slots: Mutex::new(HashMap::new()),
        }
    }

    fn slot(&self, uri: &JavaUri) -> Slot {
        self.slots.lock().entry(uri.clone()).or_default().clone()
    }

    /// `now + min(duration, MAXIMUM_EXPIRY)`.
    fn expiration_time(now: i64, duration: i64) -> i64 {
        now.saturating_add(duration.min(MAXIMUM_EXPIRY))
    }

    /// `Cache.get(key, mappingFunction)`: the cached value if present and not expired, else the
    /// value the function computes, which is stored unless it is `None` (Caffeine's `null`). An
    /// error of the function reaches the caller and stores nothing.
    // Java: com/github/benmanes/caffeine/cache/Cache.java Cache.get
    pub fn get(
        &self,
        uri: &JavaUri,
        mapping_function: impl FnOnce(&JavaUri) -> Result<Option<GetCredentialsResponse>, IoException>,
    ) -> Result<Option<GetCredentialsResponse>, IoException> {
        let slot = self.slot(uri);
        let mut entry = slot.lock();
        let now = self.ticker.read();
        if let Some(present) = entry.as_mut()
            && now < present.expires_at
        {
            let duration = self.expiry.expire_after_read(
                uri,
                &present.value,
                now,
                present.expires_at.saturating_sub(now),
            );
            present.expires_at = Self::expiration_time(now, duration);
            return Ok(Some(present.value.clone()));
        }
        match mapping_function(uri)? {
            None => {
                *entry = None;
                Ok(None)
            }
            Some(value) => {
                let duration = self.expiry.expire_after_create(uri, &value, now);
                *entry = Some(CacheEntry {
                    value: value.clone(),
                    expires_at: Self::expiration_time(now, duration),
                });
                Ok(Some(value))
            }
        }
    }

    /// `Cache.put(key, value)`.
    // Java: com/github/benmanes/caffeine/cache/Cache.java Cache.put
    pub fn put(&self, uri: &JavaUri, value: GetCredentialsResponse) {
        let slot = self.slot(uri);
        let mut entry = slot.lock();
        let now = self.ticker.read();
        let duration = match entry.as_ref() {
            Some(present) if now < present.expires_at => self.expiry.expire_after_update(
                uri,
                &value,
                now,
                present.expires_at.saturating_sub(now),
            ),
            _ => self.expiry.expire_after_create(uri, &value, now),
        };
        *entry = Some(CacheEntry {
            value,
            expires_at: Self::expiration_time(now, duration),
        });
    }

    /// `Cache.invalidateAll()`.
    // Java: com/github/benmanes/caffeine/cache/Cache.java Cache.invalidateAll
    pub fn invalidate_all(&self) {
        self.slots.lock().clear();
    }

    /// `Cache.cleanUp()`: expired entries are dropped.
    // Java: com/github/benmanes/caffeine/cache/Cache.java Cache.cleanUp
    pub fn clean_up(&self) {
        let now = self.ticker.read();
        self.slots.lock().retain(|_, slot| {
            // A slot being computed (locked) is kept.
            match slot.try_lock() {
                Some(entry) => entry.as_ref().is_some_and(|e| now < e.expires_at),
                None => true,
            }
        });
    }

    /// `cache.policy().getEntryIfPresentQuietly(uri)`: the entry if present and not expired,
    /// without touching its expiry.
    // Java: com/github/benmanes/caffeine/cache/Policy.java Policy.getEntryIfPresentQuietly
    pub fn get_entry_if_present_quietly(&self, uri: &JavaUri) -> Option<CacheEntry> {
        let slot = self.slots.lock().get(uri).cloned()?;
        let entry = slot.lock();
        let now = self.ticker.read();
        entry.as_ref().filter(|e| now < e.expires_at).cloned()
    }
}

/// A module whose sole purpose is to hold the credential cache which is shared by other modules.
///
/// PORT: Bazel has one `BlazeRuntime` per server process and its users reach the module through
/// `runtime.getBlazeModule(CredentialModule.class)`; here [`credential_module`] is that one
/// instance, and `blaze_runtime`'s `CredentialModule` calls [`CredentialModule::before_command`]
/// on it.
// Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialModule.java:31 CredentialModule
pub struct CredentialModule {
    credential_cache_expiry: Arc<CredentialCacheExpiry>,
    credential_cache: Arc<CredentialCache>,
    last_default_cache_duration: Mutex<Duration>,
}

impl CredentialModule {
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialModule.java:41 CredentialModule.<init>
    pub fn new(clock: Arc<dyn Clock>) -> CredentialModule {
        let credential_cache_expiry = Arc::new(CredentialCacheExpiry::default());
        let credential_cache = Arc::new(CredentialCache::new(
            WallTicker::new(clock),
            credential_cache_expiry.clone(),
        ));
        CredentialModule {
            credential_cache_expiry,
            credential_cache,
            last_default_cache_duration: Mutex::new(Duration::ZERO),
        }
    }

    /// Returns the credential cache.
    // Java: com/google/devtools/build/lib/authandtls/credentialhelper/CredentialModule.java:51 CredentialModule.getCredentialCache
    pub fn get_credential_cache(&self) -> &Arc<CredentialCache> {
        &self.credential_cache
    }

    /// `beforeCommand(env)`, given `env.getCommandName()` and
    /// `AuthAndTLSOptions.credentialHelperCacheTimeout`.
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

/// The runtime's `CredentialModule` (see [`CredentialModule`]).
pub fn credential_module() -> &'static CredentialModule {
    static MODULE: OnceLock<CredentialModule> = OnceLock::new();
    MODULE.get_or_init(|| CredentialModule::new(Arc::new(JavaClock)))
}
