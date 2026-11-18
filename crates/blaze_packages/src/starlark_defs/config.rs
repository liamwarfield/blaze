//! The `config` module (Java `analysis/starlark/StarlarkConfig.java`: build settings and the
//! exec/target/none transition factories), the configuration globals `transition`,
//! `exec_transition` and `analysis_test_transition` (Java `rules/config/ConfigGlobalLibrary.java`)
//! and the conversions of transition values to [`TransitionFactory`] used by `attr.*(cfg=)` and
//! `rule(cfg=)`.

use std::collections::HashSet;
use std::fmt;

use allocative::Allocative;
use blaze_cmdline::Label;
use blaze_cmdline::label_constants::COMMAND_LINE_OPTION_PREFIX;
use blaze_config::transition_factories::ExecutionTransitionFactory;
use blaze_config::transition_factories::NativeTransitionFactory;
use blaze_config::transition_factories::NativeTransitionKind;
use blaze_config::transition_factories::TransitionType;
use blaze_config::transitions::Settings;
use blaze_config::transitions::StarlarkDefinedConfigTransition;
use blaze_config::transitions::StarlarkDefinedConfigTransitionGen;
use blaze_starlark::ThreadContext;
use starlark::any::ProvidesStaticType;
use starlark::environment::GlobalsBuilder;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::FrozenValue;
use starlark::values::NoSerialize;
use starlark::values::StarlarkValue;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::none::NoneOr;
use starlark::values::starlark_value;

use super::util::caller_location;
use super::util::dict_cast;
use super::util::errorf;
use super::util::module_context_or_throw;
use super::util::sequence_cast_strings;
use super::value_cell::PendingValue;
use crate::attribute::TransitionFactory;
use crate::bzl_context::BazelModuleContext;
use crate::rule_class::BuildSetting;
use crate::rule_class::DEFAULT_EXEC_GROUP_NAME;
use crate::types::Type;

// -------------------------------------------------------------------------------------------
// BuildSetting value

/// The Starlark value of a [`BuildSetting`] (Java `BuildSetting` implements `BuildSettingApi`).
#[derive(Debug, Clone, PartialEq, Eq, ProvidesStaticType, NoSerialize, Allocative)]
pub struct BuildSettingValue(#[allocative(skip)] pub BuildSetting);

starlark_simple_value!(BuildSettingValue);

impl fmt::Display for BuildSettingValue {
    // Java: com/google/devtools/build/lib/packages/BuildSetting.java:69 BuildSetting.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

#[starlark_value(type = "BuildSetting")]
impl<'v> StarlarkValue<'v> for BuildSettingValue {
    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(other
            .downcast_ref::<BuildSettingValue>()
            .is_some_and(|o| o == self))
    }
}

// -------------------------------------------------------------------------------------------
// config module

/// The `config` module object (Java `StarlarkConfig`).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkConfig.java:34 StarlarkConfig
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct StarlarkConfig;

starlark_simple_value!(StarlarkConfig);

impl fmt::Display for StarlarkConfig {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkConfig.java:84 StarlarkConfig.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<config>")
    }
}

#[starlark_value(type = "config")]
impl<'v> StarlarkValue<'v> for StarlarkConfig {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("config", config_methods);
        Some(RES.methods())
    }
}

#[starlark_module]
fn config_methods(builder: &mut MethodsBuilder) {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkConfig.java:37 StarlarkConfig.intSetting
    fn int(
        #[starlark(this)] _this: Value,
        #[starlark(require = named, default = false)] flag: bool,
    ) -> starlark::Result<BuildSettingValue> {
        Ok(BuildSettingValue(BuildSetting::create(flag, Type::Integer)))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkConfig.java:42 StarlarkConfig.boolSetting
    fn bool(
        #[starlark(this)] _this: Value,
        #[starlark(require = named, default = false)] flag: bool,
    ) -> starlark::Result<BuildSettingValue> {
        Ok(BuildSettingValue(BuildSetting::create(flag, Type::Boolean)))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkConfig.java:47 StarlarkConfig.stringSetting
    fn string(
        #[starlark(this)] _this: Value,
        #[starlark(require = named, default = false)] flag: bool,
        #[starlark(require = named, default = false)] allow_multiple: bool,
    ) -> starlark::Result<BuildSettingValue> {
        Ok(BuildSettingValue(BuildSetting::create_full(
            flag,
            Type::String,
            allow_multiple,
            false,
        )))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkConfig.java:52 StarlarkConfig.stringListSetting
    fn string_list(
        #[starlark(this)] _this: Value,
        #[starlark(require = named, default = false)] flag: bool,
        #[starlark(require = named, default = false)] repeatable: bool,
    ) -> starlark::Result<BuildSettingValue> {
        if repeatable && !flag {
            return Err(errorf(
                "'repeatable' can only be set for a setting with 'flag = True'",
            ));
        }
        Ok(BuildSettingValue(BuildSetting::create_full(
            flag,
            Type::StringList,
            false,
            repeatable,
        )))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkConfig.java:60 StarlarkConfig.stringSetSetting
    fn string_set(
        #[starlark(this)] _this: Value,
        #[starlark(require = named, default = false)] flag: bool,
        #[starlark(require = named, default = false)] repeatable: bool,
    ) -> starlark::Result<BuildSettingValue> {
        if repeatable && !flag {
            return Err(errorf(
                "'repeatable' can only be set for a setting with 'flag = True'",
            ));
        }
        Ok(BuildSettingValue(BuildSetting::create_full(
            flag,
            Type::StringSet,
            false,
            repeatable,
        )))
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkConfig.java:68 StarlarkConfig.exec
    fn exec(
        #[starlark(this)] _this: Value,
        #[starlark(default = NoneOr::None)] exec_group: NoneOr<&str>,
    ) -> starlark::Result<ExecutionTransitionFactory> {
        Ok(match exec_group {
            NoneOr::None => ExecutionTransitionFactory::create_factory(),
            NoneOr::Other(g) => ExecutionTransitionFactory::create_factory_for(g),
        })
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkConfig.java:75 StarlarkConfig.target
    fn target(#[starlark(this)] _this: Value) -> starlark::Result<NativeTransitionFactory> {
        Ok(NativeTransitionFactory {
            kind: NativeTransitionKind::NoTransition,
        })
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkConfig.java:80 StarlarkConfig.none
    fn none(#[starlark(this)] _this: Value) -> starlark::Result<NativeTransitionFactory> {
        Ok(NativeTransitionFactory {
            kind: NativeTransitionKind::NoConfigTransition,
        })
    }
}

/// Registers `config` (Java `ConfigBootstrap`).
#[starlark_module]
pub fn register_config(builder: &mut GlobalsBuilder) {
    const config: StarlarkConfig = StarlarkConfig;
}

// -------------------------------------------------------------------------------------------
// ConfigGlobalLibrary: transition(), exec_transition(), analysis_test_transition()

// Java: com/google/devtools/build/lib/rules/config/ConfigGlobalLibrary.java:191 ConfigGlobalLibrary.validOptionName
fn valid_option_name(option_name: &str) -> bool {
    if option_name.starts_with("experimental_") {
        // Don't allow experimental flags.
        return false;
    }
    if option_name == "incompatible_enable_cc_toolchain_resolution"
        || option_name == "incompatible_enable_apple_toolchain_resolution"
    {
        // This is specifically allowed.
        return true;
    } else if option_name.starts_with("incompatible_") {
        // Don't allow other incompatible flags.
        return false;
    }
    true
}

// Java: com/google/devtools/build/lib/rules/config/ConfigGlobalLibrary.java:143 ConfigGlobalLibrary.validateBuildSettingKeys
fn validate_build_setting_keys(
    option_keys: &[String],
    key_error_descriptor: Settings,
    allow_incompatible_and_experimental_options: bool,
    module: &BazelModuleContext,
) -> starlark::Result<()> {
    let package_context = module.package_context();
    let mut processed_options = HashSet::new();
    let singular_error_descriptor = if key_error_descriptor == Settings::Inputs {
        "input"
    } else {
        "output"
    };
    for option_key in option_keys {
        if !option_key.starts_with(COMMAND_LINE_OPTION_PREFIX) {
            match Label::parse_with_repo_context(option_key, &package_context) {
                Ok(label) => {
                    if !label.get_repository().is_visible() {
                        return Err(errorf(format!(
                            "invalid transition {singular_error_descriptor} '{label}': no repo \
                             visible as @{} from {}",
                            label.get_repository().get_name(),
                            label.get_repository().get_context_repo_display_string()
                        )));
                    }
                }
                Err(e) => {
                    return Err(errorf(format!(
                        "invalid transition {singular_error_descriptor} '{option_key}'. If this \
                         is intended as a native option, it must begin with \
                         //command_line_option: {}",
                        e.get_message()
                    )));
                }
            }
        } else {
            let option_name = &option_key[COMMAND_LINE_OPTION_PREFIX.len()..];
            if !allow_incompatible_and_experimental_options && !valid_option_name(option_name) {
                return Err(errorf(format!(
                    "Invalid transition {singular_error_descriptor} '{option_key}'. Cannot \
                     transition on --experimental_* or --incompatible_* options"
                )));
            }
        }
        if !processed_options.insert(option_key.clone()) {
            return Err(errorf(format!(
                "duplicate transition {singular_error_descriptor} '{option_key}'"
            )));
        }
    }
    Ok(())
}

fn disallowed_options(eval: &Evaluator) -> starlark::Result<Vec<String>> {
    Ok(ThreadContext::get(eval)?
        .semantics
        .incompatible_disable_transitions_on
        .clone())
}

#[starlark_module]
pub fn register_config_global_library(builder: &mut GlobalsBuilder) {
    // Java: com/google/devtools/build/lib/rules/config/ConfigGlobalLibrary.java:48 ConfigGlobalLibrary.transition
    fn transition<'v>(
        #[starlark(require = named)] implementation: Value<'v>,
        #[starlark(require = named)] inputs: Value<'v>,
        #[starlark(require = named)] outputs: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<StarlarkDefinedConfigTransition<'v>> {
        let module = module_context_or_throw(eval)?;
        let inputs_list = sequence_cast_strings(inputs, "inputs")?;
        let outputs_list = sequence_cast_strings(outputs, "outputs")?;
        validate_build_setting_keys(&inputs_list, Settings::Inputs, false, &module)?;
        validate_build_setting_keys(&outputs_list, Settings::Outputs, false, &module)?;
        let location = caller_location(eval);
        let disallowed = disallowed_options(eval)?;
        StarlarkDefinedConfigTransition::new_regular_transition(
            implementation,
            &inputs_list,
            &outputs_list,
            module.label().clone(),
            location,
            &module.repo_mapping,
            &disallowed,
        )
        .map_err(errorf)
    }

    // Java: com/google/devtools/build/lib/rules/config/ConfigGlobalLibrary.java:78 ConfigGlobalLibrary.execTransition
    fn exec_transition<'v>(
        #[starlark(require = named)] implementation: Value<'v>,
        #[starlark(require = named)] inputs: Value<'v>,
        #[starlark(require = named)] outputs: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<StarlarkDefinedConfigTransition<'v>> {
        // TODO: blaze-configurability-team - When we relax this, add checks that regular usages
        // of transitions (ie, rule and attribute) cannot use exec_transition.
        super::util::fail_if_called_outside_builtins(eval)?;
        let module = module_context_or_throw(eval)?;
        let inputs_list = sequence_cast_strings(inputs, "inputs")?;
        let outputs_list = sequence_cast_strings(outputs, "outputs")?;
        validate_build_setting_keys(&inputs_list, Settings::Inputs, true, &module)?;
        validate_build_setting_keys(&outputs_list, Settings::Outputs, true, &module)?;
        let location = caller_location(eval);
        let disallowed = disallowed_options(eval)?;
        StarlarkDefinedConfigTransition::new_exec_transition(
            implementation,
            &inputs_list,
            &outputs_list,
            module.label().clone(),
            location,
            &module.repo_mapping,
            &disallowed,
        )
        .map_err(errorf)
    }

    // Java: com/google/devtools/build/lib/rules/config/ConfigGlobalLibrary.java:111 ConfigGlobalLibrary.analysisTestTransition
    fn analysis_test_transition<'v>(
        #[starlark(require = named)] settings: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<StarlarkDefinedConfigTransition<'v>> {
        let module = module_context_or_throw(eval)?;
        let changed_settings = dict_cast(
            settings,
            "string",
            |k| k.unpack_str().is_some(),
            "Object",
            |_| true,
            "changed_settings dict",
        )?;
        let changed_settings: Vec<(String, Value<'v>)> = changed_settings
            .into_iter()
            .map(|(k, v)| (k.unpack_str().unwrap().to_owned(), v))
            .collect();
        let keys: Vec<String> = changed_settings.iter().map(|(k, _)| k.clone()).collect();
        validate_build_setting_keys(&keys, Settings::Outputs, true, &module)?;
        let location = caller_location(eval);
        let disallowed = disallowed_options(eval)?;
        StarlarkDefinedConfigTransition::new_analysis_test_transition(
            changed_settings,
            &module.repo_mapping,
            module.label().clone(),
            location,
            &disallowed,
        )
        .map_err(errorf)
    }
}

// -------------------------------------------------------------------------------------------
// Conversions to TransitionFactory

/// A transition factory converted from a Starlark value, with the values its handles refer to.
pub struct ConvertedTransition<'v> {
    pub factory: TransitionFactory,
    pub pending: Vec<PendingValue<'v>>,
    /// Java `StarlarkAttributeTransitionProvider` / `StarlarkRuleTransitionProvider` of an
    /// analysis-test transition.
    pub is_for_analysis_testing: bool,
    /// The factory is (or contains) a Starlark-defined transition.
    pub is_starlark_defined: bool,
}

/// The Starlark-defined transition `v`, if it is one.
pub fn as_starlark_transition<'v>(
    v: Value<'v>,
) -> Option<&'v StarlarkDefinedConfigTransitionGen<Value<'v>>> {
    StarlarkDefinedConfigTransition::from_value(v)
}

fn frozen_transition_is_for_analysis_testing(v: FrozenValue) -> Option<bool> {
    v.downcast_ref::<StarlarkDefinedConfigTransitionGen<FrozenValue>>()
        .map(|t| t.is_for_analysis_testing())
}

/// Java `instanceof StarlarkDefinedConfigTransition` + `isForAnalysisTesting()`, for both
/// unfrozen and frozen transitions.
pub fn starlark_transition_info(v: Value) -> Option<bool> {
    if let Some(t) = StarlarkDefinedConfigTransition::from_value(v) {
        return Some(t.is_for_analysis_testing());
    }
    v.unpack_frozen()
        .and_then(frozen_transition_is_for_analysis_testing)
}

fn starlark_factory<'v>(
    v: Value<'v>,
    is_split: bool,
    eval: &Evaluator<'v, '_, '_>,
) -> Option<ConvertedTransition<'v>> {
    let for_testing = starlark_transition_info(v)?;
    let pending = PendingValue::new(v, eval);
    Some(ConvertedTransition {
        factory: TransitionFactory::Starlark {
            transition: pending.handle,
            is_split,
        },
        pending: vec![pending],
        is_for_analysis_testing: for_testing,
        is_starlark_defined: true,
    })
}

fn native_factory<'v>(factory: TransitionFactory) -> ConvertedTransition<'v> {
    ConvertedTransition {
        factory,
        pending: Vec::new(),
        is_for_analysis_testing: false,
        is_starlark_defined: false,
    }
}

/// `NoConfigTransition.getFactory()`.
fn no_config_transition_factory() -> TransitionFactory {
    TransitionFactory::no_config_transition()
}

/// Java `ExecutionTransitionFactory.createFactory(execGroup)`.
pub fn exec_factory(exec_group: &str) -> TransitionFactory {
    TransitionFactory::Exec {
        exec_group: Some(exec_group.to_owned()),
    }
}

/// Java `ExecutionTransitionFactory.createFactory()` (the default exec group).
// PORT: `exec_group: None` denotes DEFAULT_EXEC_GROUP_NAME, as in native_rules.
pub fn default_exec_factory() -> TransitionFactory {
    TransitionFactory::Exec { exec_group: None }
}

/// The native transition factories that are Starlark values (Java `ConfigurationTransitionApi`
/// instances that are `TransitionFactory`s), with their transition type.
fn native_transition_value(v: Value) -> Option<(TransitionFactory, TransitionType)> {
    if let Some(e) = v.downcast_ref::<ExecutionTransitionFactory>() {
        let factory = if e.get_exec_group() == DEFAULT_EXEC_GROUP_NAME {
            default_exec_factory()
        } else {
            exec_factory(e.get_exec_group())
        };
        return Some((factory, e.transition_type()));
    }
    if let Some(n) = v.downcast_ref::<NativeTransitionFactory>() {
        let f = match n.kind {
            NativeTransitionKind::NoTransition => TransitionFactory::NoTransition,
            NativeTransitionKind::NoConfigTransition => no_config_transition_factory(),
        };
        return Some((f, n.transition_type()));
    }
    None
}

/// Java `StarlarkAttrModule.convertCfg`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttrModule.java:514 StarlarkAttrModule.convertCfg
pub fn convert_cfg<'v>(
    eval: &Evaluator<'v, '_, '_>,
    trans: Value<'v>,
) -> starlark::Result<ConvertedTransition<'v>> {
    // The most common case is no transition.
    if trans.unpack_str() == Some("target") || trans.is_none() {
        return Ok(native_factory(TransitionFactory::NoTransition));
    }
    // TODO(b/203203933): remove after removing --incompatible_disable_starlark_host_transitions.
    if trans.unpack_str() == Some("host") {
        let disable_starlark_host_transitions = ThreadContext::get(eval)?
            .semantics
            .incompatible_disable_starlark_host_transitions;
        if disable_starlark_host_transitions {
            return Err(errorf(
                "'cfg = \"host\"' is deprecated and should no longer be used. Please use 'cfg = \
                 \"exec\"' instead.",
            ));
        }
        return Ok(native_factory(default_exec_factory()));
    }
    if trans.unpack_str() == Some("exec") {
        return Ok(native_factory(default_exec_factory()));
    }
    // StarlarkAttributeTransitionProvider.isSplit() is always true.
    if let Some(c) = starlark_factory(trans, true, eval) {
        return Ok(c);
    }
    // PORT: ComposedConfigurationTransition values are not created by any ported builtin.
    if let Some((factory, transition_type)) = native_transition_value(trans)
        && transition_type.is_compatible_with(TransitionType::Attribute)
    {
        return Ok(native_factory(factory));
    }
    // We don't actively advertise the hard-coded but exposed transitions like
    // android_split_transition because users of those transitions should already know about
    // them.
    Err(errorf(
        "cfg must be either 'target', 'exec' or a starlark defined transition defined by the \
         exec() or transition() functions.",
    ))
}

/// Java `StarlarkRuleClassFunctions.convertConfig`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:1246 StarlarkRuleClassFunctions.convertConfig
pub fn convert_config<'v>(
    eval: &Evaluator<'v, '_, '_>,
    cfg: Value<'v>,
) -> starlark::Result<ConvertedTransition<'v>> {
    if cfg.is_none() {
        return Ok(native_factory(TransitionFactory::NoTransition));
    }
    // defined in Starlark via, cfg = transition
    // StarlarkRuleTransitionProvider is a patch (non-split) transition.
    if let Some(c) = starlark_factory(cfg, false, eval) {
        return Ok(c);
    }
    if let Some((factory, transition_type)) = native_transition_value(cfg)
        && transition_type.is_compatible_with(TransitionType::Rule)
    {
        return Ok(native_factory(factory));
    }
    Err(errorf(
        "`cfg` must be set to a transition object initialized by the transition() function.",
    ))
}
