//! The credential cache: Caffeine's `Cache<URI, GetCredentialsResponse>` as `CredentialModule`
//! builds it (`Caffeine.newBuilder().ticker(new WallTicker(clock)).expireAfter(expiry).build()`).
//!
//! PORT: Caffeine is a third-party library; this ports the behaviour of that one configuration
//! (variable expiration, no size bound), following Caffeine 3's `BoundedLocalCache`:
//!
//! - An entry's expiration time ("variable time") is `now + min(duration, MAXIMUM_EXPIRY)` with
//!   `duration` from `Expiry.expireAfterCreate` / `expireAfterUpdate`, in `long` arithmetic.
//! - An entry has expired when `now - variableTime >= 0`; an expired entry is absent for every
//!   read, and `get` computes a new value in its place.
//! - `get(key, mappingFunction)` is atomic per key: concurrent callers for the same key wait for
//!   one computation. A mapping function that returns `null` stores nothing; one that throws
//!   stores nothing and the exception reaches the caller.
//! - The ticker is read before looking an entry up, and again after computing a new value (the
//!   entry's creation time).

use std::collections::HashMap;
use std::sync::Arc;

use parking_lot::Mutex;

use super::get_credentials_response::GetCredentialsResponse;
use crate::uri::JavaUri;

/// Caffeine `BoundedLocalCache.MAXIMUM_EXPIRY`: `Long.MAX_VALUE >> 1` (about 150 years).
const MAXIMUM_EXPIRY: i64 = i64::MAX >> 1;

/// `com.github.benmanes.caffeine.cache.Ticker`: a time source, in nanoseconds.
pub trait Ticker: Send + Sync {
    /// Returns the number of nanoseconds elapsed since this ticker's fixed point of reference.
    fn read(&self) -> i64;
}

/// `com.github.benmanes.caffeine.cache.Expiry<URI, GetCredentialsResponse>`: durations in
/// nanoseconds.
pub trait Expiry: Send + Sync {
    fn expire_after_create(
        &self,
        key: &JavaUri,
        value: &GetCredentialsResponse,
        current_time: i64,
    ) -> i64;

    fn expire_after_update(
        &self,
        key: &JavaUri,
        value: &GetCredentialsResponse,
        current_time: i64,
        current_duration: i64,
    ) -> i64;

    fn expire_after_read(
        &self,
        key: &JavaUri,
        value: &GetCredentialsResponse,
        current_time: i64,
        current_duration: i64,
    ) -> i64;
}

/// `com.github.benmanes.caffeine.cache.Policy.CacheEntry`: an entry and when it expires.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CacheEntry {
    pub key: JavaUri,
    pub value: GetCredentialsResponse,
    /// `expiresAt()`: the ticker reading at which the entry expires.
    pub expires_at: i64,
}

struct Node {
    value: GetCredentialsResponse,
    variable_time: i64,
}

type Slot = Arc<Mutex<Option<Node>>>;

/// `Cache<URI, GetCredentialsResponse>`.
pub struct CredentialCache {
    ticker: Arc<dyn Ticker>,
    expiry: Arc<dyn Expiry>,
    data: Mutex<HashMap<JavaUri, Slot>>,
}

impl CredentialCache {
    /// `Caffeine.newBuilder().ticker(ticker).expireAfter(expiry).build()`.
    pub fn new(ticker: Arc<dyn Ticker>, expiry: Arc<dyn Expiry>) -> CredentialCache {
        CredentialCache {
            ticker,
            expiry,
            data: Mutex::new(HashMap::new()),
        }
    }

    fn slot(&self, key: &JavaUri) -> Slot {
        self.data.lock().entry(key.clone()).or_default().clone()
    }

    fn has_expired(node: &Node, now: i64) -> bool {
        now.wrapping_sub(node.variable_time) >= 0
    }

    /// `now + Math.min(duration, MAXIMUM_EXPIRY)`.
    fn variable_time(now: i64, duration: i64) -> i64 {
        now.wrapping_add(duration.min(MAXIMUM_EXPIRY))
    }

    /// `Cache.get(key, mappingFunction)`: the value of `key`, computing it with
    /// `mapping_function` if it is absent or expired. `Ok(None)` when the function returns
    /// `null`.
    pub fn get<E>(
        &self,
        key: &JavaUri,
        mapping_function: impl FnOnce(&JavaUri) -> Result<Option<GetCredentialsResponse>, E>,
    ) -> Result<Option<GetCredentialsResponse>, E> {
        let slot = self.slot(key);
        let mut node = slot.lock();
        let now = self.ticker.read();
        if let Some(n) = node.as_mut()
            && !Self::has_expired(n, now)
        {
            let current_duration = n.variable_time.wrapping_sub(now);
            let duration = self
                .expiry
                .expire_after_read(key, &n.value, now, current_duration);
            if duration != current_duration {
                n.variable_time = Self::variable_time(now, duration);
            }
            return Ok(Some(n.value.clone()));
        }
        *node = None;

        let Some(value) = mapping_function(key)? else {
            return Ok(None);
        };
        let now = self.ticker.read();
        let duration = self.expiry.expire_after_create(key, &value, now);
        *node = Some(Node {
            value: value.clone(),
            variable_time: Self::variable_time(now, duration),
        });
        Ok(Some(value))
    }

    /// `Cache.put(key, value)`.
    pub fn put(&self, key: &JavaUri, value: GetCredentialsResponse) {
        let slot = self.slot(key);
        let mut node = slot.lock();
        let now = self.ticker.read();
        let duration = match node.as_ref() {
            Some(n) if !Self::has_expired(n, now) => {
                let current_duration = n.variable_time.wrapping_sub(now);
                self.expiry
                    .expire_after_update(key, &value, now, current_duration)
            }
            _ => self.expiry.expire_after_create(key, &value, now),
        };
        *node = Some(Node {
            value,
            variable_time: Self::variable_time(now, duration),
        });
    }

    /// `Cache.policy().getEntryIfPresentQuietly(key)`: the entry, unless it is absent or
    /// expired, without touching its expiration.
    pub fn get_entry_if_present_quietly(&self, key: &JavaUri) -> Option<CacheEntry> {
        let slot = self.data.lock().get(key).cloned()?;
        let node = slot.lock();
        let now = self.ticker.read();
        let n = node.as_ref()?;
        if Self::has_expired(n, now) {
            return None;
        }
        Some(CacheEntry {
            key: key.clone(),
            value: n.value.clone(),
            expires_at: n.variable_time,
        })
    }

    /// `Cache.invalidateAll()`.
    pub fn invalidate_all(&self) {
        self.data.lock().clear();
    }

    /// `Cache.cleanUp()`: removes expired entries.
    pub fn clean_up(&self) {
        let now = self.ticker.read();
        self.data.lock().retain(|_, slot| match slot.try_lock() {
            Some(node) => node.as_ref().is_some_and(|n| !Self::has_expired(n, now)),
            // A computation is in progress for this key.
            None => true,
        });
    }
}
