//! Port of `src/test/java/com/google/devtools/build/lib/authandtls/credentialhelper/CredentialModuleTest.java`.

use std::sync::Arc;
use std::sync::atomic::{AtomicI64, Ordering};
use std::time::Duration;

use blaze_events::ui::Clock;
use blaze_repo_fetch::authandtls::credentialhelper::{
    CredentialCache, CredentialModule, GetCredentialsResponse, Instant,
};
use blaze_repo_fetch::uri::JavaUri;

/// `com.google.devtools.build.lib.testutil.ManualClock`: starts at the epoch.
#[derive(Default)]
struct ManualClock {
    millis: AtomicI64,
}

impl ManualClock {
    fn now(&self) -> Instant {
        Instant::of_epoch_milli(self.millis.load(Ordering::SeqCst))
    }

    fn advance(&self, duration: Duration) {
        self.millis
            .fetch_add(duration.as_millis() as i64, Ordering::SeqCst);
    }
}

impl Clock for ManualClock {
    fn current_time_millis(&self) -> i64 {
        self.millis.load(Ordering::SeqCst)
    }

    fn nano_time(&self) -> i64 {
        self.millis.load(Ordering::SeqCst) * 1_000_000
    }
}

struct Fixture {
    clock: Arc<ManualClock>,
    module: CredentialModule,
}

fn minutes(n: u64) -> Duration {
    Duration::from_secs(n * 60)
}

fn test_uri() -> JavaUri {
    JavaUri::create("https://example.com").unwrap()
}

fn default_response() -> GetCredentialsResponse {
    GetCredentialsResponse::new_builder().build()
}

impl Fixture {
    fn new() -> Fixture {
        let clock = Arc::new(ManualClock::default());
        let module = CredentialModule::with_clock(clock.clone());
        Fixture { clock, module }
    }

    fn cache(&self) -> &CredentialCache {
        self.module.get_credential_cache()
    }

    fn init_module(&self, command_name: &str, cache_timeout: Duration) {
        self.module.before_command(command_name, cache_timeout);
    }

    fn assert_in_cache(&self, uri: &JavaUri, expiry: Instant) {
        let entry = self.cache().get_entry_if_present_quietly(uri);
        assert!(entry.is_some());
        assert_eq!(
            Instant::of_epoch_second_and_nanos(0, entry.unwrap().expires_at),
            expiry
        );
    }

    fn assert_not_in_cache(&self, uri: &JavaUri) {
        assert!(self.cache().get_entry_if_present_quietly(uri).is_none());
    }
}

#[test]
fn put_with_explicit_expiration() {
    let f = Fixture::new();
    f.init_module("build", minutes(60));

    let expiry = f.clock.now().plus(minutes(123));

    let mut response = GetCredentialsResponse::new_builder();
    response.set_expires(expiry);
    f.cache().put(&test_uri(), response.build());
    f.assert_in_cache(&test_uri(), expiry);

    f.clock.advance(minutes(122));
    f.assert_in_cache(&test_uri(), expiry);

    f.clock.advance(minutes(2));
    f.assert_not_in_cache(&test_uri());
}

#[test]
fn put_with_nonzero_default_expiration() {
    let f = Fixture::new();
    f.init_module("build", minutes(60));

    let expiry = f.clock.now().plus(minutes(60));

    f.cache().put(&test_uri(), default_response());
    f.assert_in_cache(&test_uri(), expiry);

    f.clock.advance(minutes(59));
    f.assert_in_cache(&test_uri(), expiry);

    f.clock.advance(minutes(2));
    f.assert_not_in_cache(&test_uri());
}

#[test]
fn put_with_zero_default_expiration() {
    let f = Fixture::new();
    f.init_module("build", Duration::ZERO);

    f.cache().put(&test_uri(), default_response());

    f.assert_not_in_cache(&test_uri());
}

#[test]
fn keeping_default_does_not_clear_cache() {
    let f = Fixture::new();
    f.init_module("build", minutes(60));

    f.cache().put(&test_uri(), default_response());
    f.assert_in_cache(&test_uri(), f.clock.now().plus(minutes(60)));

    f.init_module("build", minutes(60));

    f.assert_in_cache(&test_uri(), f.clock.now().plus(minutes(60)));
}

#[test]
fn changing_default_to_smaller_value_clears_cache() {
    let f = Fixture::new();
    f.init_module("build", minutes(60));

    f.cache().put(&test_uri(), default_response());
    f.assert_in_cache(&test_uri(), f.clock.now().plus(minutes(60)));

    f.init_module("build", minutes(30));

    f.assert_not_in_cache(&test_uri());
}

#[test]
fn changing_default_to_larger_value_clears_cache() {
    let f = Fixture::new();
    f.init_module("build", minutes(30));

    f.cache().put(&test_uri(), default_response());
    f.assert_in_cache(&test_uri(), f.clock.now().plus(minutes(30)));

    f.init_module("build", minutes(60));

    f.assert_not_in_cache(&test_uri());
}

#[test]
fn clean_command_clears_cache() {
    let f = Fixture::new();
    f.init_module("build", minutes(60));

    f.cache().put(&test_uri(), default_response());
    f.assert_in_cache(&test_uri(), f.clock.now().plus(minutes(60)));

    f.init_module("clean", minutes(60));

    f.assert_not_in_cache(&test_uri());
}

// The tests below have no Java counterpart: they pin the Caffeine behaviour the credentials rely
// on (`get` computes once, `null` stores nothing, an error stores nothing, expired entries are
// recomputed) and the loud failure while the runtime does not call `before_command`.

#[test]
fn get_computes_once_until_expiry() {
    let f = Fixture::new();
    f.init_module("build", minutes(60));
    let calls = std::cell::Cell::new(0);
    let compute = |_: &JavaUri| -> Result<Option<GetCredentialsResponse>, ()> {
        calls.set(calls.get() + 1);
        Ok(Some(default_response()))
    };

    assert_eq!(
        f.cache().get(&test_uri(), compute),
        Ok(Some(default_response()))
    );
    assert_eq!(
        f.cache().get(&test_uri(), compute),
        Ok(Some(default_response()))
    );
    assert_eq!(calls.get(), 1);

    f.clock.advance(minutes(61));
    assert_eq!(
        f.cache().get(&test_uri(), compute),
        Ok(Some(default_response()))
    );
    assert_eq!(calls.get(), 2);
}

#[test]
fn get_stores_neither_null_nor_errors() {
    let f = Fixture::new();
    f.init_module("build", minutes(60));

    assert_eq!(
        f.cache()
            .get(&test_uri(), |_| -> Result<_, ()> { Ok(None) }),
        Ok(None)
    );
    f.assert_not_in_cache(&test_uri());
    assert_eq!(
        f.cache()
            .get(&test_uri(), |_| -> Result<_, &str> { Err("boom") }),
        Err("boom")
    );
    f.assert_not_in_cache(&test_uri());
}

#[test]
#[should_panic(expected = "not ported yet: CredentialModule.beforeCommand (missing hook")]
fn default_duration_without_before_command_fails_loudly() {
    let f = Fixture::new();
    f.cache().put(&test_uri(), default_response());
}
