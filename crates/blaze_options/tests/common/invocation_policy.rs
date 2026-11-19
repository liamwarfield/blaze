//! Port of src/test/java/com/google/devtools/common/options/InvocationPolicyEnforcerTestBase.java.

use std::sync::Arc;

use base64::Engine as _;
use blaze_options::invocation_policy::{
    CommandNameCache, CommandNameCacheInstance, InvocationPolicyEnforcer, InvocationPolicyParser,
};
use blaze_options::{OptionsClass, OptionsParser};
use blaze_protos::JavaMessage;
use blaze_protos::blaze::invocation_policy::InvocationPolicy;

use super::test_options::TestOptions;

pub const BUILD_COMMAND: &str = "build";
pub const TEST_STRING_USER_VALUE: &str = "user value";
pub const TEST_STRING_USER_VALUE_2: &str = "user value 2";
pub const TEST_STRING_POLICY_VALUE: &str = "policy value";
pub const TEST_STRING_POLICY_VALUE_2: &str = "policy value 2";

/// The behaviours a `@TestParameter Behavior` runs with after `assume().that(behavior)
/// .isNotEqualTo(Behavior.UNDEFINED)`.
pub const DEFINED_BEHAVIORS: [&str; 4] = [
    "ALLOW_OVERRIDES",
    "APPEND",
    "FINAL_VALUE_IGNORE_OVERRIDES",
    "FINAL_VALUE_THROW_ON_OVERRIDE",
];

/// The policy built in a test, written in text format (the tests' `InvocationPolicy.Builder`
/// calls give the same message).
pub fn policy(text: &str) -> InvocationPolicy {
    blaze_protos::text_format::parse::<InvocationPolicy>(text)
        .unwrap_or_else(|e| panic!("bad test policy {text:?}: {e}"))
}

// Java: InvocationPolicyEnforcerTestBase.createOptionsPolicyEnforcer
/// Blaze takes the policy as a flag with a Base64 encoded binary proto and parses that, so the
/// test exercises that code path.
/// PORT: the Java test parses `--invocation_policy=<base64>` with a startup-options parser
/// (BlazeServerStartupOptions, which lives in blaze_optdefs, above this crate) and hands the
/// option value to `parsePolicy`; here the value goes to `parsePolicy` directly.
pub fn create_options_policy_enforcer(
    invocation_policy: &InvocationPolicy,
) -> InvocationPolicyEnforcer {
    let mut out = Vec::new();
    invocation_policy.java_encode_raw(&mut out);
    let policy_base64 = base64::engine::general_purpose::STANDARD.encode(&out);
    InvocationPolicyEnforcer::new(
        InvocationPolicyParser::parse_policy(Some(&policy_base64)).unwrap(),
        /* conversion_context= */ None,
    )
}

struct IdentityCommandNameCache;

impl CommandNameCache for IdentityCommandNameCache {
    fn get(&self, command_name: &str) -> Vec<String> {
        vec![command_name.to_string()]
    }
}

// Java: InvocationPolicyEnforcerTestBase.setParser, setCommandNameCache
pub fn set_parser() -> OptionsParser {
    CommandNameCacheInstance::set_command_name_cache(Arc::new(IdentityCommandNameCache));
    OptionsParser::builder()
        .options_classes(&[TestOptions::class()])
        .build()
        .unwrap()
}

// Java: InvocationPolicyEnforcerTestBase.getTestOptions
pub fn get_test_options(parser: &OptionsParser) -> TestOptions {
    parser.get_options::<TestOptions>().unwrap()
}

/// `enforcer.enforce(parser, BUILD_COMMAND, ImmutableList.builder())`.
pub fn enforce(
    enforcer: &InvocationPolicyEnforcer,
    parser: &mut OptionsParser,
) -> Result<(), blaze_options::OptionsParsingException> {
    enforcer.enforce(parser, BUILD_COMMAND, &mut Vec::new())
}

/// `parser.asCompleteListOfParsedOptions()` mapped to `getCommandLineForm`.
pub fn command_line_forms(parser: &OptionsParser) -> Vec<Option<String>> {
    parser
        .as_complete_list_of_parsed_options()
        .iter()
        .map(|p| p.get_command_line_form().map(str::to_string))
        .collect()
}
