//! Port of `CredentialModuleTest`.
// Java: src/test/java/com/google/devtools/build/lib/authandtls/credentialhelper/CredentialModuleTest.java

use std::sync::Arc;
use std::time::Duration;

use blaze_events::Clock;
use blaze_repo_fetch::authandtls::Headers;
use blaze_repo_fetch::authandtls::credentialhelper::{
    CredentialModule, GetCredentialsResponse, JavaInstant,
};
use blaze_repo_fetch::uri::JavaUri;
use parking_lot::Mutex;

/// `ManualClock` (starts at the epoch).
#[derive(Default)]
struct ManualClock {
    millis: Mutex<i64>,
}

impl ManualClock {
    fn now(&self) -> JavaInstant {
        JavaInstant::of_epoch_milli(*self.millis.lock())
    }

    fn advance(&self, duration: Duration) {
        *self.millis.lock() += duration.as_millis() as i64;
    }
}

impl Clock for ManualClock {
    fn current_time_millis(&self) -> i64 {
        *self.millis.lock()
    }

    fn nano_time(&self) -> i64 {
        *self.millis.lock() * 1_000_000
    }
}

fn minutes(m: u64) -> Duration {
    Duration::from_secs(60 * m)
}

struct Test {
    clock: Arc<ManualClock>,
    module: CredentialModule,
    test_uri: JavaUri,
}

impl Test {
    fn new() -> Test {
        let clock = Arc::new(ManualClock::default());
        Test {
            module: CredentialModule::new(clock.clone()),
            clock,
            test_uri: JavaUri::create("https://example.com").unwrap(),
        }
    }

    fn init_module(&self, command_name: &str, cache_timeout: Duration) {
        self.module.before_command(command_name, cache_timeout);
    }

    fn put(&self, response: GetCredentialsResponse) {
        self.module
            .get_credential_cache()
            .put(&self.test_uri, response);
    }

    fn assert_in_cache(&self, expiry: JavaInstant) {
        let entry = self
            .module
            .get_credential_cache()
            .get_entry_if_present_quietly(&self.test_uri)
            .expect("entry is in the cache");
        assert_eq!(JavaInstant::of_epoch_nano(entry.expires_at), expiry);
    }

    fn assert_not_in_cache(&self) {
        assert!(
            self.module
                .get_credential_cache()
                .get_entry_if_present_quietly(&self.test_uri)
                .is_none()
        );
    }
}

fn default_response() -> GetCredentialsResponse {
    GetCredentialsResponse::new(Headers::new(), None)
}

#[test]
fn put_with_explicit_expiration() {
    let t = Test::new();
    t.init_module("build", minutes(60));

    let expiry = t.clock.now().plus(minutes(123));

    t.put(GetCredentialsResponse::new(Headers::new(), Some(expiry)));
    t.assert_in_cache(expiry);

    t.clock.advance(minutes(122));
    t.assert_in_cache(expiry);

    t.clock.advance(minutes(2));
    t.assert_not_in_cache();
}

#[test]
fn put_with_nonzero_default_expiration() {
    let t = Test::new();
    t.init_module("build", minutes(60));

    let expiry = t.clock.now().plus(minutes(60));

    t.put(default_response());
    t.assert_in_cache(expiry);

    t.clock.advance(minutes(59));
    t.assert_in_cache(expiry);

    t.clock.advance(minutes(2));
    t.assert_not_in_cache();
}

#[test]
fn put_with_zero_default_expiration() {
    let t = Test::new();
    t.init_module("build", Duration::ZERO);

    t.put(default_response());

    t.assert_not_in_cache();
}

#[test]
fn keeping_default_does_not_clear_cache() {
    let t = Test::new();
    t.init_module("build", minutes(60));

    t.put(default_response());
    t.assert_in_cache(t.clock.now().plus(minutes(60)));

    t.init_module("build", minutes(60));

    t.assert_in_cache(t.clock.now().plus(minutes(60)));
}

#[test]
fn changing_default_to_smaller_value_clears_cache() {
    let t = Test::new();
    t.init_module("build", minutes(60));

    t.put(default_response());
    t.assert_in_cache(t.clock.now().plus(minutes(60)));

    t.init_module("build", minutes(30));

    t.assert_not_in_cache();
}

#[test]
fn changing_default_to_larger_value_clears_cache() {
    let t = Test::new();
    t.init_module("build", minutes(30));

    t.put(default_response());
    t.assert_in_cache(t.clock.now().plus(minutes(30)));

    t.init_module("build", minutes(60));

    t.assert_not_in_cache();
}

#[test]
fn clean_command_clears_cache() {
    let t = Test::new();
    t.init_module("build", minutes(60));

    t.put(default_response());
    t.assert_in_cache(t.clock.now().plus(minutes(60)));

    t.init_module("clean", minutes(60));

    t.assert_not_in_cache();
}
