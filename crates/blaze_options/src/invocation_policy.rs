//! Invocation policies: `InvocationPolicyParser`, `InvocationPolicyEnforcer`, `CommandNameCache`
//! and `OptionAndRawValue` (package `com.google.devtools.common.options`).
//!
//! PORT: the enforcer's `logger.at(loglevel)` lines go to Bazel's server log (`java.log`) only;
//! Blaze has no server log, so they are not written and the `Level` argument is not kept.

use std::sync::{Arc, RwLock};

use blaze_protos::JavaMessage;
use blaze_protos::blaze::invocation_policy::{
    FlagPolicy, InvocationPolicy, SetValue, UseDefault, allow_values, disallow_values,
    flag_policy::Operation, set_value::Behavior,
};
use blaze_protos::descriptors::pool;
use blaze_protos::dynamic_message::DynamicMessage;
use indexmap::IndexMap;

use crate::converter::ConversionContext;
use crate::definition::OptionDefinitionRef;
use crate::exception::OptionsParsingException;
use crate::parsed::{OptionInstanceOrigin, ParsedOptionRef};
use crate::parser::{OptionDescription, OptionsParser};
use crate::priority::{OptionPriority, PriorityCategory};
use crate::value::{Value, java_list_to_string, java_string_of};
use crate::value_description::OptionValueDescription;

// ---------------------------------------------------------------------------------------------
// OptionAndRawValue

/// An option that was added to a Blaze invocation by the invocation policy, with its raw
/// (unparsed) value.
// Java: com/google/devtools/common/options/OptionAndRawValue.java:24 OptionAndRawValue
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct OptionAndRawValue {
    option_name: String,
    /// `None` for Void options.
    raw_value: Option<String>,
}

impl OptionAndRawValue {
    // Java: com/google/devtools/common/options/OptionAndRawValue.java:25 OptionAndRawValue.create
    pub fn create(option_name: impl Into<String>, raw_value: Option<String>) -> OptionAndRawValue {
        OptionAndRawValue {
            option_name: option_name.into(),
            raw_value,
        }
    }

    // Java: com/google/devtools/common/options/OptionAndRawValue.java:29 OptionAndRawValue.getOptionName
    pub fn get_option_name(&self) -> &str {
        &self.option_name
    }

    // Java: com/google/devtools/common/options/OptionAndRawValue.java:33 OptionAndRawValue.getRawValue
    pub fn get_raw_value(&self) -> Option<&str> {
        self.raw_value.as_deref()
    }
}

// ---------------------------------------------------------------------------------------------
// CommandNameCache

/// Cache mapping a command to the names of all commands it inherits from, including itself.
// Java: com/google/devtools/common/options/CommandNameCache.java:20 CommandNameCache
pub trait CommandNameCache: Send + Sync {
    /// Returns the names of all commands `command_name` inherits from, including itself.
    // Java: com/google/devtools/common/options/CommandNameCache.java:42 CommandNameCache.get
    fn get(&self, command_name: &str) -> Vec<String>;
}

/// `CommandNameCache.CommandNameCacheInstance.INSTANCE`: the process-wide cache, set by the
/// runtime (and by tests).
// Java: com/google/devtools/common/options/CommandNameCache.java:22 CommandNameCache.CommandNameCacheInstance
pub struct CommandNameCacheInstance;

static COMMAND_NAME_CACHE: RwLock<Option<Arc<dyn CommandNameCache>>> = RwLock::new(None);

impl CommandNameCacheInstance {
    /// Only for use by `BlazeRuntime` (and tests). Can be set multiple times.
    // Java: com/google/devtools/common/options/CommandNameCache.java:29 CommandNameCacheInstance.setCommandNameCache
    pub fn set_command_name_cache(cache: Arc<dyn CommandNameCache>) {
        *COMMAND_NAME_CACHE.write().unwrap() = Some(cache);
    }

    // Java: com/google/devtools/common/options/CommandNameCache.java:35 CommandNameCacheInstance.get
    pub fn get(command_name: &str) -> Vec<String> {
        let delegate = COMMAND_NAME_CACHE
            .read()
            .unwrap()
            .clone()
            .unwrap_or_else(|| panic!("NullPointerException: CommandNameCache not set"));
        delegate.get(command_name)
    }
}

// ---------------------------------------------------------------------------------------------
// InvocationPolicyParser

/// Parses the given InvocationPolicy string, which may be a base64-encoded binary-serialized
/// InvocationPolicy message, or a text formatted InvocationPolicy message.
// Java: com/google/devtools/common/options/InvocationPolicyParser.java:28 InvocationPolicyParser
pub struct InvocationPolicyParser;

impl InvocationPolicyParser {
    /// Parses InvocationPolicy in either of the accepted formats. Returns an empty policy if no
    /// policy is provided (`None` or empty).
    ///
    /// The policy is an internal string (the option value). The text form is parsed char by char
    /// as Java's `TextFormat.parse(CharSequence)` does; string values of the binary form are
    /// decoded as protobuf-java decodes them (`new String(bytes, UTF_8)`).
    // Java: com/google/devtools/common/options/InvocationPolicyParser.java:36 InvocationPolicyParser.parsePolicy
    pub fn parse_policy(policy: Option<&str>) -> Result<InvocationPolicy, OptionsParsingException> {
        let Some(policy) = policy.filter(|p| !p.is_empty()) else {
            return Ok(InvocationPolicy::default());
        };

        let malformed = || {
            OptionsParsingException::new(format!(
                "Malformed value of --invocation_policy: {policy}"
            ))
        };
        // First try decoding the policy as a base64 encoded binary proto.
        let without_whitespace: String = policy
            .chars()
            .filter(|&c| !char_matcher_whitespace(c))
            .collect();
        match base64_decode(&without_whitespace) {
            Ok(bytes) => {
                // `InvocationPolicy.parseFrom(bytes)`: the descriptor-driven parser is the port
                // of protobuf-java's (CodedInputStream, string decoding, error cases).
                let descriptor = pool()
                    .find_message_type_by_name(InvocationPolicy::FULL_NAME)
                    .expect("InvocationPolicy descriptor");
                let message =
                    DynamicMessage::parse_from(descriptor, &bytes).map_err(|_| malformed())?;
                // PORT: Java keeps unknown fields (a field number this version does not know, or
                // an unknown value of a closed enum) in the message's UnknownFieldSet; the
                // policy's `toString()` (`--canonicalize_policy`, the error texts of the
                // enforcer) prints them, and `toBuilder` carries them into the expanded
                // policies. The generated type has no unknown field set, so a policy with one
                // fails loudly instead of losing it.
                if has_unknown_fields(&message) {
                    blaze_util::unported!(
                        "InvocationPolicyParser.parsePolicy",
                        "unknown fields in a binary --invocation_policy"
                    );
                }
                Ok(message.into_message::<InvocationPolicy>())
            }
            // If the flag value can't be decoded from base64, try decoding the policy as a text
            // formatted proto.
            Err(Base64DecodingException) => {
                blaze_protos::text_format::parse::<InvocationPolicy>(policy)
                    .map_err(|_| malformed())
            }
        }
    }
}

/// Whether `message` or a message nested in it has a non-empty `getUnknownFields()`.
fn has_unknown_fields(message: &DynamicMessage) -> bool {
    use blaze_protos::dynamic_message::Value as FieldValue;
    !message.get_unknown_fields().is_empty()
        || message.get_all_fields().into_iter().any(|(_, values)| {
            values.iter().any(|value| match value {
                FieldValue::Message(nested) => has_unknown_fields(nested),
                _ => false,
            })
        })
}

/// `CharMatcher.whitespace().matches(c)` (Guava): the Unicode White_Space characters, without
/// U+0085's neighbours U+001C..U+001F.
fn char_matcher_whitespace(c: char) -> bool {
    // Java (Guava 33): com/google/common/base/CharMatcher.java:1217 CharMatcher.Whitespace.TABLE
    matches!(
        c,
        '\u{2002}'
            | '\u{3000}'
            | '\r'
            | '\u{0085}'
            | '\u{200A}'
            | '\u{2005}'
            | '\u{2000}'
            | '\u{2029}'
            | '\u{000B}'
            | '\u{2008}'
            | '\u{2003}'
            | '\u{205F}'
            | '\u{1680}'
            | '\u{0009}'
            | '\u{0020}'
            | '\u{2006}'
            | '\u{2001}'
            | '\u{202F}'
            | '\u{00A0}'
            | '\u{000C}'
            | '\u{2009}'
            | '\u{2004}'
            | '\u{2028}'
            | '\n'
            | '\u{2007}'
    )
}

/// Guava's `DecodingException` (wrapped in an `IllegalArgumentException` by `decode`).
#[derive(Debug)]
struct Base64DecodingException;

/// `BaseEncoding.base64().decode(chars)` (Guava): the standard alphabet; any number of trailing
/// `=` is removed first, an input length of 1 mod 4 is invalid, any other character outside the
/// alphabet is invalid, and the unused low bits of the last character are ignored.
// Java (Guava 33): com/google/common/io/BaseEncoding.java:1026 BaseEncoding.Base64Encoding.decodeTo
fn base64_decode(chars: &str) -> Result<Vec<u8>, Base64DecodingException> {
    fn decode(ch: char) -> Result<u32, Base64DecodingException> {
        match ch {
            'A'..='Z' => Ok(ch as u32 - 'A' as u32),
            'a'..='z' => Ok(ch as u32 - 'a' as u32 + 26),
            '0'..='9' => Ok(ch as u32 - '0' as u32 + 52),
            '+' => Ok(62),
            '/' => Ok(63),
            _ => Err(Base64DecodingException),
        }
    }
    // trimTrailingPadding. Java counts UTF-16 units; every char that is not ASCII is an error
    // below, so counting chars gives the same answer.
    let chars: Vec<char> = chars.trim_end_matches('=').chars().collect();
    // isValidPaddingStartPosition: lengths 0, 2 and 3 mod 4.
    if chars.len() % 4 == 1 {
        return Err(Base64DecodingException);
    }
    let mut target = Vec::with_capacity(chars.len() * 3 / 4);
    let mut i = 0;
    while i < chars.len() {
        let mut chunk = decode(chars[i])? << 18;
        i += 1;
        chunk |= decode(chars[i])? << 12;
        i += 1;
        target.push((chunk >> 16) as u8);
        if i < chars.len() {
            chunk |= decode(chars[i])? << 6;
            i += 1;
            target.push(((chunk >> 8) & 0xFF) as u8);
            if i < chars.len() {
                chunk |= decode(chars[i])?;
                i += 1;
                target.push((chunk & 0xFF) as u8);
            }
        }
    }
    Ok(target)
}

// ---------------------------------------------------------------------------------------------
// InvocationPolicyEnforcer

// Java: com/google/devtools/common/options/InvocationPolicyEnforcer.java:55 InvocationPolicyEnforcer.INVOCATION_POLICY_SOURCE
const INVOCATION_POLICY_SOURCE: &str = "Invocation policy";

/// Enforces the `FlagPolicy`s (from an `InvocationPolicy` proto) on an [`OptionsParser`] by
/// validating and changing the flag values in the given parser.
// Java: com/google/devtools/common/options/InvocationPolicyEnforcer.java:51 InvocationPolicyEnforcer
pub struct InvocationPolicyEnforcer {
    invocation_policy: InvocationPolicy,
    conversion_context: Option<Arc<ConversionContext>>,
}

// Java: com/google/devtools/common/options/InvocationPolicyEnforcer.java:75 InvocationPolicyEnforcer.FlagPolicyWithContext
#[derive(Clone)]
struct FlagPolicyWithContext {
    policy: FlagPolicy,
    description: OptionDescription,
    origin: OptionInstanceOrigin,
}

/// `FlagPolicy.getOperationCase()`.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
enum OperationCase {
    SetValue,
    UseDefault,
    DisallowValues,
    AllowValues,
    OperationNotSet,
}

fn operation_case(policy: &FlagPolicy) -> OperationCase {
    match &policy.operation {
        Some(Operation::SetValue(_)) => OperationCase::SetValue,
        Some(Operation::UseDefault(_)) => OperationCase::UseDefault,
        Some(Operation::DisallowValues(_)) => OperationCase::DisallowValues,
        Some(Operation::AllowValues(_)) => OperationCase::AllowValues,
        None => OperationCase::OperationNotSet,
    }
}

/// `policy.getSetValue()` (the default instance when another operation is set).
fn get_set_value(policy: &FlagPolicy) -> SetValue {
    match &policy.operation {
        Some(Operation::SetValue(s)) => s.clone(),
        _ => SetValue::default(),
    }
}

/// `setValue.getBehavior()`: an unset or unknown value is `UNDEFINED`.
fn get_behavior(set_value: &SetValue) -> Behavior {
    set_value
        .behavior
        .and_then(|b| Behavior::try_from(b).ok())
        .unwrap_or(Behavior::Undefined)
}

/// `FlagPolicy.toString()` (`AbstractMessage.toString`).
fn policy_to_string(policy: &FlagPolicy) -> String {
    blaze_protos::text_format::message_to_string(policy)
}

/// `PolicyOperationNotSetException`.
// Java: com/google/devtools/common/options/InvocationPolicyEnforcer.java:199 InvocationPolicyEnforcer.PolicyOperationNotSetException
fn policy_operation_not_set_exception(flag_name: &str) -> OptionsParsingException {
    OptionsParsingException::new(format!(
        "Flag policy for flag '{flag_name}' does not have an operation"
    ))
}

impl InvocationPolicyEnforcer {
    /// Creates an InvocationPolicyEnforcer that enforces the given policy.
    // Java: com/google/devtools/common/options/InvocationPolicyEnforcer.java:68 InvocationPolicyEnforcer.<init>
    pub fn new(
        invocation_policy: InvocationPolicy,
        conversion_context: Option<Arc<ConversionContext>>,
    ) -> InvocationPolicyEnforcer {
        InvocationPolicyEnforcer {
            invocation_policy,
            conversion_context,
        }
    }

    /// Applies this instance's policy to the provided options parser.
    ///
    /// Flag policies that apply to specific commands are enforced only if they contain this
    /// command or a command it inherits from. `invocation_policy_flag_list_builder` receives the
    /// unparsed flags that invocation policy applies to the command.
    // Java: com/google/devtools/common/options/InvocationPolicyEnforcer.java:101 InvocationPolicyEnforcer.enforce
    pub fn enforce(
        &self,
        parser: &mut OptionsParser,
        command: &str,
        invocation_policy_flag_list_builder: &mut Vec<OptionAndRawValue>,
    ) -> Result<(), OptionsParsingException> {
        if self.invocation_policy.flag_policies.is_empty() {
            return Ok(());
        }

        // The effective policy returned is expanded, filtered for applicable commands, and
        // cleaned of redundancies and conflicts.
        let effective_policies = get_effective_policies(&self.invocation_policy, parser, command)?;

        let conversion_context = self.conversion_context.as_deref();
        for flag_policy in &effective_policies {
            let flag_name = flag_policy.policy.flag_name();

            // `parser.getOptionValueDescription(flagName)` throws IllegalArgumentException for a
            // flag that does not exist. We are deliberately lenient if the flag policy has a
            // flag we don't know about. This is for better future proofing so that as new flags
            // are added, new policies can use the new flags without worrying about older
            // versions of Bazel.
            if parser
                .options_data()
                .get_option_definition_from_name(flag_name)
                .is_none()
            {
                continue;
            }
            let value_description = parser.get_option_value_description(flag_name);

            match operation_case(&flag_policy.policy) {
                OperationCase::SetValue => apply_set_value_operation(
                    parser,
                    flag_policy,
                    value_description.as_ref(),
                    conversion_context,
                    invocation_policy_flag_list_builder,
                )?,
                OperationCase::UseDefault => apply_use_default_operation(
                    parser,
                    "UseDefault",
                    flag_policy.description.get_option_definition(),
                    conversion_context,
                    invocation_policy_flag_list_builder,
                )?,
                OperationCase::AllowValues => {
                    let Some(Operation::AllowValues(allow_values)) = &flag_policy.policy.operation
                    else {
                        unreachable!()
                    };
                    FilterValueOperation::allow(conversion_context).apply(
                        parser,
                        &flag_policy.origin,
                        &allow_values.allowed_values,
                        match &allow_values.replacement_value {
                            Some(allow_values::ReplacementValue::NewValue(v)) => Some(v.as_str()),
                            _ => None,
                        },
                        matches!(
                            allow_values.replacement_value,
                            Some(allow_values::ReplacementValue::UseDefault(_))
                        ),
                        value_description.as_ref(),
                        &flag_policy.description,
                        invocation_policy_flag_list_builder,
                    )?;
                }
                OperationCase::DisallowValues => {
                    let Some(Operation::DisallowValues(disallow_values)) =
                        &flag_policy.policy.operation
                    else {
                        unreachable!()
                    };
                    FilterValueOperation::disallow(conversion_context).apply(
                        parser,
                        &flag_policy.origin,
                        &disallow_values.disallowed_values,
                        match &disallow_values.replacement_value {
                            Some(disallow_values::ReplacementValue::NewValue(v)) => {
                                Some(v.as_str())
                            }
                            _ => None,
                        },
                        matches!(
                            disallow_values.replacement_value,
                            Some(disallow_values::ReplacementValue::UseDefault(_))
                        ),
                        value_description.as_ref(),
                        &flag_policy.description,
                        invocation_policy_flag_list_builder,
                    )?;
                }
                OperationCase::OperationNotSet => {
                    return Err(policy_operation_not_set_exception(flag_name));
                }
            }
        }
        Ok(())
    }

    /// Returns the expanded and filtered policy that would be enforced for the given command.
    // Java: com/google/devtools/common/options/InvocationPolicyEnforcer.java:214 InvocationPolicyEnforcer.getEffectiveInvocationPolicy
    pub fn get_effective_invocation_policy(
        invocation_policy: &InvocationPolicy,
        parser: &mut OptionsParser,
        command: &str,
    ) -> Result<InvocationPolicy, OptionsParsingException> {
        let effective_policies = get_effective_policies(invocation_policy, parser, command)?;
        Ok(InvocationPolicy {
            flag_policies: effective_policies.into_iter().map(|p| p.policy).collect(),
            strategy_policy: None,
        })
    }
}

// Java: com/google/devtools/common/options/InvocationPolicyEnforcer.java:205 InvocationPolicyEnforcer.policyApplies
fn policy_applies(policy: &FlagPolicy, applicable_commands: &[String]) -> bool {
    // If the commands list is empty, then the policy applies to all commands.
    if policy.commands.is_empty() {
        return true;
    }
    policy
        .commands
        .iter()
        .any(|c| applicable_commands.contains(c))
}

/// Takes the provided policy and processes it to the form that can be used on the user options.
/// Expands any policies on expansion flags.
// Java: com/google/devtools/common/options/InvocationPolicyEnforcer.java:233 InvocationPolicyEnforcer.getEffectivePolicies
fn get_effective_policies(
    invocation_policy: &InvocationPolicy,
    parser: &mut OptionsParser,
    command: &str,
) -> Result<Vec<FlagPolicyWithContext>, OptionsParsingException> {
    let command_and_parent_commands = CommandNameCacheInstance::get(command);

    // Expand all policies to transfer policies on expansion flags to policies on the child flags.
    let mut expanded_policies: Vec<FlagPolicyWithContext> = Vec::new();
    let mut next_priority =
        OptionPriority::lowest_option_priority_at_category(PriorityCategory::INVOCATION_POLICY);
    for policy in &invocation_policy.flag_policies {
        // Explicitly disallow --config in invocation policy.
        if policy.flag_name() == "config" {
            return Err(OptionsParsingException::new(format!(
                "Invocation policy is applied after --config expansion, changing config values \
                 now would have no effect and is disallowed to prevent confusion. Please remove \
                 the following policy : {}",
                policy_to_string(policy)
            )));
        }

        // These policies are high-level, before expansion, and so are not the implicitDependents
        // or expansions of any other flag, other than in an obtuse sense from
        // --invocation_policy.
        let current_priority = next_priority.clone();
        let origin = OptionInstanceOrigin::new(
            current_priority.clone(),
            Some(INVOCATION_POLICY_SOURCE.to_string()),
            None,
            None,
        );
        next_priority = OptionPriority::next_option_priority(&current_priority);
        if !policy_applies(policy, &command_and_parent_commands) {
            // Only keep and expand policies that are applicable to the current command.
            continue;
        }

        let Some(option_description) = parser.get_option_description(policy.flag_name()) else {
            // InvocationPolicy ignores policy on non-existing flags by design, for version
            // compatibility.
            continue;
        };
        let policy_with_context = FlagPolicyWithContext {
            policy: policy.clone(),
            description: option_description,
            origin,
        };
        let policies = expand_policy(policy_with_context, parser)?;
        expanded_policies.extend(policies);
    }

    // Only keep that last policy for each flag (`ImmutableMap.Builder.buildKeepingLast`: a key
    // keeps the position of its first occurrence).
    let mut effective_policy: IndexMap<String, FlagPolicyWithContext> = IndexMap::new();
    for expanded_policy in expanded_policies {
        let flag_name = expanded_policy.policy.flag_name().to_string();
        effective_policy.insert(flag_name, expanded_policy);
    }
    Ok(effective_policy.into_values().collect())
}

// Java: com/google/devtools/common/options/InvocationPolicyEnforcer.java:302 InvocationPolicyEnforcer.throwAllowValuesOnExpansionFlagException
fn throw_allow_values_on_expansion_flag_exception(flag_name: &str) -> OptionsParsingException {
    OptionsParsingException::new(format!(
        "Allow_Values on expansion flags like {flag_name} is not allowed."
    ))
}

// Java: com/google/devtools/common/options/InvocationPolicyEnforcer.java:308 InvocationPolicyEnforcer.throwDisallowValuesOnExpansionFlagException
fn throw_disallow_values_on_expansion_flag_exception(flag_name: &str) -> OptionsParsingException {
    OptionsParsingException::new(format!(
        "Disallow_Values on expansion flags like {flag_name} is not allowed."
    ))
}

// Java: com/google/devtools/common/options/InvocationPolicyEnforcer.java:314 InvocationPolicyEnforcer.throwUndefinedBehaviorException
fn throw_undefined_behavior_exception(policy: &FlagPolicy) -> OptionsParsingException {
    OptionsParsingException::new(format!(
        "SetValue operation from invocation policy for has an undefined behavior: {}",
        policy_to_string(policy)
    ))
}

/// Expand a single policy. If the policy is not about an expansion flag, this will simply return
/// a list with a single element, oneself. If the policy is for an expansion flag, the policy
/// will get split into multiple policies applying to each flag the original flag expands to.
///
/// None of the flagPolicies returned should be on expansion flags.
// Java: com/google/devtools/common/options/InvocationPolicyEnforcer.java:330 InvocationPolicyEnforcer.expandPolicy
fn expand_policy(
    original_policy: FlagPolicyWithContext,
    parser: &mut OptionsParser,
) -> Result<Vec<FlagPolicyWithContext>, OptionsParsingException> {
    let mut expanded_policies: Vec<FlagPolicyWithContext> = Vec::new();

    let is_expansion = original_policy.description.is_expansion();
    let subflags: Vec<ParsedOptionRef> = parser.get_expansion_value_descriptions(
        original_policy.description.get_option_definition(),
        &original_policy.origin,
    )?;

    // If we have nothing to expand to, no need to do any further work.
    if subflags.is_empty() {
        return Ok(vec![original_policy]);
    }

    // Repeated flags are special, and could set multiple times in an expansion, with the user
    // expecting both values to be valid. Collect these separately.
    // PORT: java-hash-order. Java's ArrayListMultimap iterates its keys in the order of a
    // `HashMap` keyed by OptionDescription, whose hashCode is deterministic
    // (`optionDefinition.hashCode() + evaluatedExpansion.hashCode()`, the first being
    // `Field.hashCode()`: the hash of the declaring class's binary name XOR the hash of the Java
    // field name). Blaze's option definitions do not carry the Java class and field names, so
    // that order cannot be computed: with one key it does not matter, with more than one the
    // loop below fails loudly. No expansion or implicit requirement at the pin names two
    // different repeatable flags.
    // Java: com/google/devtools/common/options/OptionsParser.java:417 OptionsParser.OptionDescription.hashCode
    // Java: com/google/devtools/common/options/FieldOptionDefinition.java:376 FieldOptionDefinition.hashCode
    let mut repeatable_subflags_in_set_values: IndexMap<
        OptionDefinitionRef,
        (OptionDescription, Vec<ParsedOptionRef>),
    > = IndexMap::new();

    // Create a flag policy for the child that looks like the parent's policy "transferred" to
    // its child. Note that this only makes sense for SetValue, when setting an expansion flag,
    // or UseDefault, when preventing it from being set.
    for current_subflag in &subflags {
        let subflag_option_description = parser
            .get_option_description(current_subflag.get_option_definition().get_option_name())
            .expect("subflag is a known option");

        if current_subflag.get_option_definition().allows_multiple()
            && operation_case(&original_policy.policy) == OperationCase::SetValue
        {
            repeatable_subflags_in_set_values
                .entry(current_subflag.get_option_definition())
                .or_insert_with(|| (subflag_option_description, Vec::new()))
                .1
                .push(current_subflag.clone());
        } else {
            let subflag_as_policy = get_single_value_subflag_as_policy(
                subflag_option_description,
                current_subflag,
                &original_policy,
                is_expansion,
            )?;
            // In case any of the expanded flags are themselves expansions, recurse.
            // `getSingleValueSubflagAsPolicy` returns null for AllowValues and DisallowValues on
            // a flag with implicit requirements; Java passes it on to `expandPolicy`, which
            // dereferences it.
            let Some(subflag_as_policy) = subflag_as_policy else {
                panic!("NullPointerException: expandPolicy(null)");
            };
            expanded_policies.extend(expand_policy(subflag_as_policy, parser)?);
        }
    }

    // If there are any repeatable flag SetValues, deal with them together now.
    // Note that expansion flags have no value, and so cannot have multiple values either.
    // Skipping the recursion above is fine.
    if repeatable_subflags_in_set_values.len() > 1 {
        blaze_util::unported!(
            "InvocationPolicyEnforcer.expandPolicy",
            "HashMap order of {} repeatable subflags of a SetValue policy on --{}",
            repeatable_subflags_in_set_values.len(),
            original_policy
                .description
                .get_option_definition()
                .get_option_name()
        );
    }
    for (repeatable_flag, set_values) in repeatable_subflags_in_set_values.values() {
        let mut new_values: Vec<String> = Vec::with_capacity(set_values.len());
        let mut origins: Vec<&OptionInstanceOrigin> = Vec::with_capacity(set_values.len());
        for set_value in set_values {
            // `newValues.add(setValue.getUnconvertedValue())`: a null value is kept in the list
            // and rejected by `SetValue.Builder.addAllFlagValue`.
            new_values.push(
                set_value
                    .get_unconverted_value()
                    .unwrap_or_else(|| panic!("NullPointerException: null flag value"))
                    .to_string(),
            );
            origins.push(set_value.get_origin());
        }
        // These options come from expanding a single policy, so they have effectively the same
        // priority. They could have come from different expansions or implicit requirements in
        // the recursive resolving of the option list, so just pick the first one. Do collapse
        // the source strings though, in case there are different sources.
        let arbitrary_first_option_origin = origins[0];
        let mut sources: Vec<String> = Vec::new();
        for origin in &origins {
            // `OptionInstanceOrigin::getSource` mapped through `Stream.distinct()` and joined:
            // `String.valueOf(null)` is "null".
            let source = origin.get_source().unwrap_or("null").to_string();
            if !sources.contains(&source) {
                sources.push(source);
            }
        }
        let origin_of_subflags = OptionInstanceOrigin::new(
            arbitrary_first_option_origin.get_priority().clone(),
            Some(sources.join(", ")),
            arbitrary_first_option_origin
                .get_implicit_dependent()
                .cloned(),
            arbitrary_first_option_origin.get_expanded_from().cloned(),
        );
        expanded_policies.push(get_set_value_subflag_as_policy(
            repeatable_flag.clone(),
            new_values,
            origin_of_subflags,
            &original_policy,
        )?);
    }

    // Don't add the original policy if it was an expansion flag, which have no value, but do add
    // it if there was either no expansion or if it was a valued flag with implicit
    // requirements.
    if !is_expansion {
        expanded_policies.push(original_policy);
    }

    Ok(expanded_policies)
}

/// Expand a SetValue flag policy on a repeatable flag. SetValue operations are the only flag
/// policies that set the flag, and so interact with repeatable flags, flags that can be set
/// multiple times, in subtle ways.
// Java: com/google/devtools/common/options/InvocationPolicyEnforcer.java:428 InvocationPolicyEnforcer.getSetValueSubflagAsPolicy
fn get_set_value_subflag_as_policy(
    subflag_desc: OptionDescription,
    subflag_value: Vec<String>,
    subflag_origin: OptionInstanceOrigin,
    original_policy: &FlagPolicyWithContext,
) -> Result<FlagPolicyWithContext, OptionsParsingException> {
    // Some checks.
    let subflag = subflag_desc.get_option_definition();
    assert!(
        operation_case(&original_policy.policy) == OperationCase::SetValue,
        "VerifyException"
    );
    if !subflag.allows_multiple() {
        assert!(subflag_value.len() <= 1, "VerifyException");
    }

    // Flag value from the expansion, overridability from the original policy, unless the flag
    // is repeatable, in which case we care about appendability, not overridability.
    let behavior = match get_behavior(&get_set_value(&original_policy.policy)) {
        Behavior::Undefined => {
            return Err(throw_undefined_behavior_exception(&original_policy.policy));
        }
        Behavior::FinalValueIgnoreOverrides | Behavior::Append => {
            Behavior::FinalValueIgnoreOverrides
        }
        Behavior::AllowOverrides => {
            if subflag.allows_multiple() {
                Behavior::Append
            } else {
                Behavior::AllowOverrides
            }
        }
        Behavior::FinalValueThrowOnOverride => Behavior::FinalValueThrowOnOverride,
    };
    let set_value_expansion = SetValue {
        flag_value: subflag_value,
        behavior: Some(behavior as i32),
    };

    // Commands from the original policy, flag name of the expansion
    Ok(FlagPolicyWithContext {
        policy: FlagPolicy {
            commands: original_policy.policy.commands.clone(),
            flag_name: Some(subflag.get_option_name().to_string()),
            operation: Some(Operation::SetValue(set_value_expansion)),
            custom_error_message: None,
        },
        description: subflag_desc,
        origin: subflag_origin,
    })
}

/// For an expansion flag in an invocation policy, each flag it expands to must be given a
/// corresponding policy.
// Java: com/google/devtools/common/options/InvocationPolicyEnforcer.java:477 InvocationPolicyEnforcer.getSingleValueSubflagAsPolicy
fn get_single_value_subflag_as_policy(
    subflag_context: OptionDescription,
    current_subflag: &ParsedOptionRef,
    original_policy: &FlagPolicyWithContext,
    is_expansion: bool,
) -> Result<Option<FlagPolicyWithContext>, OptionsParsingException> {
    let mut subflag_as_policy = None;
    match operation_case(&original_policy.policy) {
        OperationCase::SetValue => {
            if current_subflag.get_option_definition().allows_multiple() {
                panic!(
                    "AssertionError: SetValue subflags with allowMultiple should have been dealt \
                     with separately and accumulated into a single FlagPolicy."
                );
            }
            // Accept null originalValueStrings, they are expected when the subflag is also an
            // expansion flag.
            let subflag_value: Vec<String> = match current_subflag.get_unconverted_value() {
                None => Vec::new(),
                Some(v) => vec![v.to_string()],
            };
            subflag_as_policy = Some(get_set_value_subflag_as_policy(
                subflag_context,
                subflag_value,
                current_subflag.get_origin().clone(),
                original_policy,
            )?);
        }
        OperationCase::UseDefault => {
            // Commands from the original policy, flag name of the expansion
            subflag_as_policy = Some(FlagPolicyWithContext {
                policy: FlagPolicy {
                    commands: original_policy.policy.commands.clone(),
                    flag_name: Some(
                        current_subflag
                            .get_option_definition()
                            .get_option_name()
                            .to_string(),
                    ),
                    operation: Some(Operation::UseDefault(UseDefault::default())),
                    custom_error_message: None,
                },
                description: subflag_context,
                origin: current_subflag.get_origin().clone(),
            });
        }
        OperationCase::AllowValues => {
            if is_expansion {
                return Err(throw_allow_values_on_expansion_flag_exception(
                    original_policy.policy.flag_name(),
                ));
            }
            // If this flag is an implicitRequirement, and some values for the parent flag are
            // allowed, nothing needs to happen on the implicitRequirement that is set for all
            // values of the flag.
        }
        OperationCase::DisallowValues => {
            if is_expansion {
                return Err(throw_disallow_values_on_expansion_flag_exception(
                    original_policy.policy.flag_name(),
                ));
            }
            // If this flag is an implicitRequirement, and some values for the parent flag are
            // disallowed, that implies that all others are allowed, so nothing needs to happen
            // on the implicitRequirement that is set for all values of the parent flag.
        }
        OperationCase::OperationNotSet => {
            return Err(policy_operation_not_set_exception(
                original_policy.policy.flag_name(),
            ));
        }
    }
    Ok(subflag_as_policy)
}

// Java: com/google/devtools/common/options/InvocationPolicyEnforcer.java:544 InvocationPolicyEnforcer.applySetValueOperation
fn apply_set_value_operation(
    parser: &mut OptionsParser,
    flag_policy: &FlagPolicyWithContext,
    value_description: Option<&OptionValueDescription>,
    _conversion_context: Option<&ConversionContext>,
    invocation_policy_flag_list_builder: &mut Vec<OptionAndRawValue>,
) -> Result<(), OptionsParsingException> {
    let set_value = get_set_value(&flag_policy.policy);
    let option_definition = flag_policy.description.get_option_definition();

    // SetValue.flag_value must have at least 1 value.
    if set_value.flag_value.is_empty() {
        return Err(OptionsParsingException::new(format!(
            "SetValue operation from invocation policy for {option_definition} does not have a \
             value"
        )));
    }

    // Flag must allow multiple values if multiple values are specified by the policy.
    if set_value.flag_value.len() > 1
        && !flag_policy
            .description
            .get_option_definition()
            .allows_multiple()
    {
        return Err(OptionsParsingException::new(format!(
            "SetValue operation from invocation policy sets multiple values for \
             {option_definition} which does not allow multiple values"
        )));
    }

    match get_behavior(&set_value) {
        Behavior::Undefined => {
            return Err(throw_undefined_behavior_exception(&flag_policy.policy));
        }
        Behavior::AllowOverrides => {
            if value_description.is_some() {
                // The user set the value for the flag but the flag policy is overridable, so
                // keep the user's value.
                // Nothing to do -- the value already has an override.
                return Ok(());
            }
        }
        Behavior::FinalValueIgnoreOverrides => {
            // Clear the value in case the flag is a repeated flag so that values don't
            // accumulate.
            parser.clear_value(flag_policy.description.get_option_definition());
        }
        Behavior::Append => {}
        Behavior::FinalValueThrowOnOverride => {
            if value_description.is_some() {
                return Err(OptionsParsingException::new(format!(
                    "User set a value for {} which is not permitted by the invocation policy. \
                     This flag value will always be overridden to {}. {}",
                    option_definition,
                    java_list_to_string(get_set_value(&flag_policy.policy).flag_value.into_iter()),
                    flag_policy.policy.custom_error_message()
                )));
            }
        }
    }

    // Set all the flag values from the policy.
    for flag_value in &set_value.flag_value {
        invocation_policy_flag_list_builder.push(OptionAndRawValue::create(
            option_definition.get_option_name(),
            Some(flag_value.clone()),
        ));

        parser.set_option_value_at_specific_priority_without_expansion(
            flag_policy.origin.clone(),
            option_definition,
            flag_value,
        )?;
    }
    Ok(())
}

// Java: com/google/devtools/common/options/InvocationPolicyEnforcer.java:637 InvocationPolicyEnforcer.applyUseDefaultOperation
fn apply_use_default_operation(
    parser: &mut OptionsParser,
    _policy_type: &str,
    option: OptionDefinitionRef,
    conversion_context: Option<&ConversionContext>,
    invocation_policy_flag_list_builder: &mut Vec<OptionAndRawValue>,
) -> Result<(), OptionsParsingException> {
    let cleared_value_description = parser.clear_value(option);
    if let Some(cleared_value_description) = cleared_value_description {
        // Log the removed value.
        let cleared_flag_name = cleared_value_description
            .get_option_definition()
            .get_option_name();
        let cleared_flag_default_value = cleared_value_description
            .get_option_definition()
            .get_default_value(conversion_context);
        invocation_policy_flag_list_builder.push(OptionAndRawValue::create(
            cleared_flag_name,
            Some(match &cleared_flag_default_value {
                Some(v) => v.java_to_string(),
                None => String::new(),
            }),
        ));
    }
    Ok(())
}

/// Checks the user's flag values against a filtering function.
// Java: com/google/devtools/common/options/InvocationPolicyEnforcer.java:664 InvocationPolicyEnforcer.FilterValueOperation
struct FilterValueOperation<'a> {
    /// "Allow" (`AllowValueOperation`) or "Disallow" (`DisallowValueOperation`).
    policy_type: &'static str,
    conversion_context: Option<&'a ConversionContext>,
}

impl<'a> FilterValueOperation<'a> {
    // Java: com/google/devtools/common/options/InvocationPolicyEnforcer.java:667 FilterValueOperation.AllowValueOperation
    fn allow(conversion_context: Option<&'a ConversionContext>) -> Self {
        FilterValueOperation {
            policy_type: "Allow",
            conversion_context,
        }
    }

    // Java: com/google/devtools/common/options/InvocationPolicyEnforcer.java:678 FilterValueOperation.DisallowValueOperation
    fn disallow(conversion_context: Option<&'a ConversionContext>) -> Self {
        FilterValueOperation {
            policy_type: "Disallow",
            conversion_context,
        }
    }

    /// Determines if the given value is allowed. `converted_policy_values` is Java's
    /// `HashSet<Object>` (null allowed; only `contains` is used).
    // Java: com/google/devtools/common/options/InvocationPolicyEnforcer.java:672 AllowValueOperation.isFlagValueAllowed
    // Java: com/google/devtools/common/options/InvocationPolicyEnforcer.java:683 DisallowValueOperation.isFlagValueAllowed
    fn is_flag_value_allowed(
        &self,
        converted_policy_values: &[Option<Value>],
        value: Option<&Value>,
    ) -> bool {
        let contains = converted_policy_values.iter().any(|v| v.as_ref() == value);
        if self.policy_type == "Allow" {
            contains
        } else {
            // In a disallow operation, the values that the flag policy specifies are not
            // allowed, so the value is allowed if the set of policy values does not contain the
            // current flag value.
            !contains
        }
    }

    // Java: com/google/devtools/common/options/InvocationPolicyEnforcer.java:709 FilterValueOperation.apply
    fn apply(
        &self,
        parser: &mut OptionsParser,
        origin: &OptionInstanceOrigin,
        policy_values: &[String],
        new_value: Option<&str>,
        use_default: bool,
        value_description: Option<&OptionValueDescription>,
        option_description: &OptionDescription,
        invocation_policy_flag_list_builder: &mut Vec<OptionAndRawValue>,
    ) -> Result<(), OptionsParsingException> {
        let option_definition = option_description.get_option_definition();
        // Convert all the allowed values from strings to real objects using the options'
        // converters so that they can be checked for equality using real .equals() instead of
        // string comparison. For example, "--foo=0", "--foo=false", "--nofoo", and "-f-" (if the
        // option has an abbreviation) are all equal for boolean flags. Plus converters can be
        // arbitrarily complex.
        let mut converted_policy_values: Vec<Option<Value>> = Vec::new();
        let mut add = |v: Option<Value>| {
            if !converted_policy_values.contains(&v) {
                converted_policy_values.push(v);
            }
        };
        for value in policy_values {
            let converted_value = option_definition
                .get_converter()
                .convert(Some(value), self.conversion_context)?;
            // Some converters return lists, and if the flag is a repeatable flag, the items in
            // the list from the converter should be added, and not the list itself. Otherwise
            // the items from invocation policy will be compared to lists, which will never work.
            // See OptionsParserImpl.ParsedOptionEntry.addValue.
            match &converted_value {
                Some(v) if option_definition.allows_multiple() && v.as_list().is_some() => {
                    for item in v.as_list().unwrap() {
                        add(Some(item.clone()));
                    }
                }
                _ => add(converted_value),
            }
        }

        // Check that if the default value of the flag is disallowed by the policy, that the
        // policy does not also set use_default. Otherwise the default value would still be set
        // if the user uses a disallowed value. This doesn't apply to repeatable flags since the
        // default value for repeatable flags is always the empty list. It also doesn't apply to
        // flags that are null by default, since these flags' default value is not parsed by the
        // converter, so there is no guarantee that there exists an accepted user-input value
        // that would also set the value to NULL. In these cases, we assume that "unset" is a
        // distinct value that is always allowed.
        if !option_definition.allows_multiple() && !option_definition.is_special_null_default() {
            let default_value_allowed = self.is_flag_value_allowed(
                &converted_policy_values,
                option_definition
                    .get_default_value(self.conversion_context)
                    .as_ref(),
            );
            if !default_value_allowed && use_default {
                return Err(OptionsParsingException::new(format!(
                    "{}Values policy disallows the default value '{}' for {} but also specifies \
                     to use the default value",
                    self.policy_type,
                    java_string_of(
                        option_definition
                            .get_default_value(self.conversion_context)
                            .as_ref()
                    ),
                    option_definition
                )));
            }
        }

        match value_description {
            None => {
                // Nothing has set the value yet, so check that the default value from the
                // flag's definition is allowed. The else case below (i.e. valueDescription is not
                // null) checks for the flag allowing multiple values, however, flags that allow
                // multiple values cannot have default values, and their value is always the
                // empty list if they haven't been specified, which is why new_default_value is
                // not a repeated field.
                self.check_default_value(
                    parser,
                    origin,
                    option_description,
                    policy_values,
                    new_value,
                    &converted_policy_values,
                    invocation_policy_flag_list_builder,
                )
            }
            Some(value_description) => self.check_user_value(
                parser,
                origin,
                option_description,
                value_description,
                policy_values,
                new_value,
                use_default,
                &converted_policy_values,
                invocation_policy_flag_list_builder,
            ),
        }
    }

    // Java: com/google/devtools/common/options/InvocationPolicyEnforcer.java:786 FilterValueOperation.checkDefaultValue
    fn check_default_value(
        &self,
        parser: &mut OptionsParser,
        origin: &OptionInstanceOrigin,
        option_description: &OptionDescription,
        policy_values: &[String],
        new_value: Option<&str>,
        converted_policy_values: &[Option<Value>],
        invocation_policy_flag_list_builder: &mut Vec<OptionAndRawValue>,
    ) -> Result<(), OptionsParsingException> {
        let option_definition = option_description.get_option_definition();
        if option_definition.is_special_null_default() {
            // Do nothing, the unset value by definition cannot be set. In option filtering
            // operations, the value is being filtered, but the value that is `no value` passes
            // any filter. Otherwise, there is no way to "usedefault" on one of these options that
            // has no value by default.
        } else if !self.is_flag_value_allowed(
            converted_policy_values,
            option_definition
                .get_default_value(self.conversion_context)
                .as_ref(),
        ) {
            if let Some(new_value) = new_value {
                // Use the default value from the policy, since the original default is not
                // allowed
                parser.clear_value(option_definition);
                parser.set_option_value_at_specific_priority_without_expansion(
                    origin.clone(),
                    option_definition,
                    new_value,
                )?;
                invocation_policy_flag_list_builder.push(OptionAndRawValue::create(
                    option_definition.get_option_name(),
                    Some(new_value.to_string()),
                ));
            } else {
                // The operation disallows the default value, but doesn't supply a new value.
                return Err(OptionsParsingException::new(format!(
                    "Default flag value '{}' for {} is not allowed by invocation policy, but the \
                     policy does not provide a new value. {}ed values are: {}",
                    java_string_of(
                        option_description
                            .get_option_definition()
                            .get_default_value(self.conversion_context)
                            .as_ref()
                    ),
                    option_definition,
                    self.policy_type,
                    java_list_to_string(policy_values.iter().cloned())
                )));
            }
        }
        Ok(())
    }

    // Java: com/google/devtools/common/options/InvocationPolicyEnforcer.java:830 FilterValueOperation.checkUserValue
    fn check_user_value(
        &self,
        parser: &mut OptionsParser,
        origin: &OptionInstanceOrigin,
        option_description: &OptionDescription,
        value_description: &OptionValueDescription,
        policy_values: &[String],
        new_value: Option<&str>,
        use_default: bool,
        converted_policy_values: &[Option<Value>],
        invocation_policy_flag_list_builder: &mut Vec<OptionAndRawValue>,
    ) -> Result<(), OptionsParsingException> {
        let option = option_description.get_option_definition();
        if option_description.get_option_definition().allows_multiple() {
            // allowMultiple requires that the type of the option be List<T>, so cast from
            // Object to List<?>.
            let option_values = value_description
                .get_value()
                .unwrap_or_else(|| panic!("NullPointerException: repeatable option value"));
            let option_values = option_values
                .as_list()
                .unwrap_or_else(|| panic!("ClassCastException: not a List"));
            for value in option_values {
                if !self.is_flag_value_allowed(converted_policy_values, Some(value)) {
                    if use_default {
                        apply_use_default_operation(
                            parser,
                            &format!("{}Values", self.policy_type),
                            option,
                            self.conversion_context,
                            invocation_policy_flag_list_builder,
                        )?;
                    } else {
                        return Err(OptionsParsingException::new(format!(
                            "Flag value '{}' for {} is not allowed by invocation policy. {}ed \
                             values are: {}",
                            value.java_to_string(),
                            option,
                            self.policy_type,
                            java_list_to_string(policy_values.iter().cloned())
                        )));
                    }
                }
            }
        } else {
            let value = value_description.get_value();
            if !self.is_flag_value_allowed(converted_policy_values, value.as_ref()) {
                if let Some(new_value) = new_value {
                    parser.clear_value(option);
                    parser.set_option_value_at_specific_priority_without_expansion(
                        origin.clone(),
                        option,
                        new_value,
                    )?;
                    invocation_policy_flag_list_builder.push(OptionAndRawValue::create(
                        option.get_option_name(),
                        Some(new_value.to_string()),
                    ));
                } else if use_default {
                    apply_use_default_operation(
                        parser,
                        &format!("{}Values", self.policy_type),
                        option,
                        self.conversion_context,
                        invocation_policy_flag_list_builder,
                    )?;
                } else {
                    return Err(OptionsParsingException::new(format!(
                        "Flag value '{}' for {} is not allowed by invocation policy and the \
                         policy does not specify a new value. {}ed values are: {}",
                        java_string_of(value.as_ref()),
                        option,
                        self.policy_type,
                        java_list_to_string(policy_values.iter().cloned())
                    )));
                }
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Guava `BaseEncoding.base64().decode` accepts missing and extra trailing padding and
    /// ignores the unused bits of the last character.
    #[test]
    fn base64_decode_follows_guava() {
        assert_eq!(base64_decode("").unwrap(), b"");
        assert_eq!(base64_decode("QQ==").unwrap(), b"A");
        assert_eq!(base64_decode("QQ").unwrap(), b"A");
        assert_eq!(base64_decode("QQ=====").unwrap(), b"A");
        assert_eq!(base64_decode("QR").unwrap(), b"A");
        assert_eq!(base64_decode("QUI").unwrap(), b"AB");
        assert_eq!(base64_decode("QUJD").unwrap(), b"ABC");
        assert!(base64_decode("Q").is_err());
        assert!(base64_decode("QQ=A").is_err());
        assert!(base64_decode("Q-==").is_err());
        assert!(base64_decode("flag_policies").is_err());
    }

    /// A binary policy without unknown fields parses (`flag_policies { flag_name: "x" }`).
    #[test]
    fn binary_policy_without_unknown_fields_parses() {
        let policy = InvocationPolicyParser::parse_policy(Some("CgMKAXg=")).unwrap();
        assert_eq!(policy.flag_policies.len(), 1);
        assert_eq!(policy.flag_policies[0].flag_name.as_deref(), Some("x"));
    }

    /// Review finding F5: Java keeps an unknown field (here field 99 of `InvocationPolicy`,
    /// varint 1) and prints it; Blaze fails loudly instead of dropping it.
    #[test]
    #[should_panic(expected = "InvocationPolicyParser.parsePolicy")]
    fn binary_policy_with_unknown_field_fails_loudly() {
        let _ = InvocationPolicyParser::parse_policy(Some("mAYB"));
    }

    /// The same for an unknown field (50) of a nested `FlagPolicy`.
    #[test]
    #[should_panic(expected = "InvocationPolicyParser.parsePolicy")]
    fn binary_policy_with_nested_unknown_field_fails_loudly() {
        let _ = InvocationPolicyParser::parse_policy(Some("CgOQAwA="));
    }
}
