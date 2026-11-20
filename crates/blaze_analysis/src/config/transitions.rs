//! Ports of `analysis/config/transitions/*` (`ConfigurationTransition`, `PatchTransition`,
//! `SplitTransition`, `NoTransition`, `NoConfigTransition`, `ComposingTransitionFactory`,
//! `TransitionFactory`, `TransitionUtil`), `analysis/config/ExecutionTransitionFactory`,
//! `analysis/starlark/StarlarkAttributeTransitionProvider`,
//! `analysis/starlark/StarlarkRuleTransitionProvider`,
//! `analysis/test/TestTrimmingTransitionFactory` and
//! `rules/config/ConfigFeatureFlagTaggedTrimmingTransitionFactory`.
//!
//! PORT: Java's open `ConfigurationTransition` / `TransitionFactory` hierarchies are closed here.
//! A loading-phase `blaze_packages::attribute::TransitionFactory` is turned into a
//! [`ConfigurationTransition`] by [`create_attribute_transition`] /
//! [`create_rule_transition`] (Java `TransitionFactory.create(data)`).

use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::sync::{Arc, LazyLock};

use blaze_build_config::build_options::{BuildOptions, BuildOptionsView};
use blaze_build_config::feature_flag_value::FeatureFlagValue;
use blaze_build_config::fragment_options_class::{
    CONFIG_FEATURE_FLAG_OPTIONS, CORE_OPTIONS, FragmentOptionsClass,
};
use blaze_build_config::function_transition_util::{
    TransitionAttrObject, apply_and_validate_with_attrs, exec_transition_patch,
    exec_transition_required_fragments, get_affected_by_starlark_transition_via_diff,
    update_affected_by_starlark_transition,
};
use blaze_build_config::starlark_transition::{
    StarlarkDefinedConfigTransition, TransitionData, TransitionKind,
};
use blaze_build_config::transitions::{
    EMPTY_OPTIONS, test_trimming_required_fragments, test_trimming_trim_view,
};
use blaze_cmdline::{Label, LabelRepoContext};
use blaze_config::transitions::{
    StarlarkDefinedConfigTransition as LoadingTransition, TransitionKind as LoadingTransitionKind,
};
use blaze_events::{Event, EventHandler};
use blaze_optdefs::fragment::config_feature_flag_options::ConfigFeatureFlagOptions;
use blaze_optdefs::fragment::core_options::{CoreOptions, CoreOptionsOutputDirectoryNamingScheme};
use blaze_optdefs::fragment::test_configuration::TestConfigurationTestOptions;
use blaze_packages::attribute::{
    Attribute, AttributeMap, NonconfigurableAttributeMapper, NonconfiguredAttributeMapper,
    RawAttributeMapper, TransitionFactory,
};
use blaze_packages::native_rules::base_rule_classes::TAGGED_TRIMMING_ATTR;
use blaze_packages::rule_class::StarlarkCallable;
use blaze_packages::target::{Rule, RuleOrMacroInstance};
use blaze_packages::types::{AttrValue, Type};
use blaze_starlark::StarlarkSemantics;
use blaze_starlark::structs::{StarlarkInfo, StructProvider};
use indexmap::IndexMap;
use parking_lot::Mutex;
use starlark::values::{FrozenHeapRef, Heap, OwnedFrozenValue, Value};

use crate::config::config_matching_provider::ConfigMatchingProvider;
use crate::config::configured_attribute_mapper::{
    AttributeResolutionResult, ConfiguredAttributeMapper,
};

pub use blaze_build_config::starlark_transition::PATCH_TRANSITION_KEY;

/// Java `TransitionFactory.TransitionCreationException` (a `RuntimeException`).
// Java: com/google/devtools/build/lib/analysis/config/transitions/TransitionFactory.java:33 TransitionFactory.TransitionCreationException
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct TransitionCreationException(pub String);

// -------------------------------------------------------------------------------------------
// Transition data.

/// Data an attribute transition factory reads (Java `packages/AttributeTransitionData`).
// Java: com/google/devtools/build/lib/packages/AttributeTransitionData.java:34 AttributeTransitionData
#[derive(Clone, Default)]
pub struct AttributeTransitionData<'a> {
    /// Java `attributes()`: the configured attributes of the target the attribute belongs to.
    pub attributes: Option<&'a ConfiguredAttributeMapper>,
    /// Java `executionPlatform()`.
    pub execution_platform: Option<Label>,
    /// Java `analysisData()`: the Starlark exec transition provider (`null` when the build
    /// uses native exec transition logic).
    pub analysis_data: Option<Arc<StarlarkAttributeTransitionProvider>>,
}

/// Data a rule transition factory reads (Java `packages/RuleTransitionData`).
///
/// PORT: `blaze_packages::rule_class::RuleTransitionData` omits the config conditions; this is
/// the full Java record.
// Java: com/google/devtools/build/lib/packages/RuleTransitionData.java:28 RuleTransitionData
#[derive(Clone)]
pub struct RuleTransitionData {
    pub rule: Arc<Rule>,
    pub config_conditions: Arc<IndexMap<blaze_cmdline::Label, ConfigMatchingProvider>>,
    pub config_hash: String,
}

impl RuleTransitionData {
    // Java: com/google/devtools/build/lib/packages/RuleTransitionData.java:38 RuleTransitionData.create
    pub fn create(
        rule: Arc<Rule>,
        config_conditions: Arc<IndexMap<blaze_cmdline::Label, ConfigMatchingProvider>>,
        config_hash: String,
    ) -> RuleTransitionData {
        RuleTransitionData {
            rule,
            config_conditions,
            config_hash,
        }
    }
}

// -------------------------------------------------------------------------------------------
// Configuration transitions.

/// The `attr` struct a Starlark transition implementation receives (Java `StructImpl
/// attrObject`): Starlark attribute names to their (resolved) values.
#[derive(Clone, Debug)]
pub struct TransitionAttributes {
    pub entries: Vec<(String, Option<AttrValue>)>,
    /// Java's `errorMessageForUnknownField` of the struct.
    pub no_attr_error: &'static str,
}

/// Java `StarlarkAttributesCollection.ERROR_MESSAGE_FOR_NO_ATTR`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttributesCollection.java:67 StarlarkAttributesCollection.ERROR_MESSAGE_FOR_NO_ATTR
const ERROR_MESSAGE_FOR_NO_ATTR: &str =
    "No attribute '%s' in attr. Make sure you declared a rule attribute with this name.";

const RULE_TRANSITION_NO_ATTR: &str = "No attribute '%s'. Either this attribute does not exist for this rule or the attribute was not resolved because it is set by a select that reads flags the transition may set.";

impl TransitionAttrObject for TransitionAttributes {
    fn alloc<'v>(&self, heap: Heap<'v>) -> Value<'v> {
        // Java: StructProvider.STRUCT.create(attributes, errorMessageForUnknownField).
        let fields: Vec<(String, Value<'v>)> = self
            .entries
            .iter()
            .map(|(k, v)| {
                (
                    k.clone(),
                    transition_attr_value_to_starlark(v.as_ref(), heap),
                )
            })
            .collect();
        heap.alloc(StarlarkInfo::with_message(
            heap.alloc(StructProvider),
            fields,
            self.no_attr_error,
        ))
    }
}

/// Java `Attribute.valueToStarlark` for the `attr` object of a Starlark transition.
///
/// PORT: Java's `ComputedDefault` and `LateBoundDefault` are `StarlarkValue`s, so
/// `Starlark.fromJava` returns them unchanged and the transition sees an opaque object.
/// `blaze_packages`' `Attribute::value_to_starlark` leaves these unported; until it gains them,
/// they are wrapped here in [`OpaqueAttributeDefault`] and every other value goes through it.
// Java: com/google/devtools/build/lib/packages/Attribute.java:2442 Attribute.valueToStarlark
fn transition_attr_value_to_starlark<'v>(v: Option<&AttrValue>, heap: Heap<'v>) -> Value<'v> {
    use blaze_packages::attribute::computed_default::{ComputedDefaultKind, LateBoundDefaultKind};
    match v {
        Some(AttrValue::ComputedDefault(c)) => match c.kind() {
            ComputedDefaultKind::Starlark(_) => heap.alloc(StarlarkComputedDefaultValue),
            // Java: a native rule's own `ComputedDefault` subclass (often anonymous).
            ComputedDefaultKind::Native(_) => heap.alloc(OpaqueAttributeDefault),
        },
        Some(AttrValue::LateBound(l)) => match l.kind() {
            LateBoundDefaultKind::Label => heap.alloc(LabelLateBoundDefaultValue),
            LateBoundDefaultKind::LabelList => heap.alloc(LabelListLateBoundDefaultValue),
            LateBoundDefaultKind::AlwaysNull => heap.alloc(AlwaysNullLateBoundDefaultValue),
            LateBoundDefaultKind::Other => heap.alloc(OpaqueAttributeDefault),
        },
        _ => Attribute::value_to_starlark(v, heap),
    }
}

/// A Java `ComputedDefault` / `LateBoundDefault` subclass seen as a Starlark value: it declares
/// no Starlark method or field; `type()` is the class's simple name (`Starlark.classType`) and
/// `repr` the default `<unknown object <class name>>` (analysis review S2-15).
// Java: net/starlark/java/eval/Starlark.java:337 Starlark.classType
// Java: net/starlark/java/eval/StarlarkValue.java:41 StarlarkValue.repr
macro_rules! attribute_default_value {
    ($name:ident, $type_name:literal, $java_class:literal) => {
        #[derive(
            Debug,
            starlark::any::ProvidesStaticType,
            starlark::values::NoSerialize,
            allocative::Allocative,
        )]
        struct $name;

        starlark::starlark_simple_value!($name);

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(concat!("<unknown object ", $java_class, ">"))
            }
        }

        #[starlark::values::starlark_value(type = $type_name)]
        impl<'v> starlark::values::StarlarkValue<'v> for $name {}
    };
}

// Java: com/google/devtools/build/lib/packages/Attribute.java:1523 Attribute.StarlarkComputedDefault
attribute_default_value!(
    StarlarkComputedDefaultValue,
    "StarlarkComputedDefault",
    "com.google.devtools.build.lib.packages.Attribute$StarlarkComputedDefault"
);
// Java: com/google/devtools/build/lib/packages/Attribute.java:1720 Attribute.LabelLateBoundDefault
attribute_default_value!(
    LabelLateBoundDefaultValue,
    "LabelLateBoundDefault",
    "com.google.devtools.build.lib.packages.Attribute$LabelLateBoundDefault"
);
// Java: com/google/devtools/build/lib/packages/Attribute.java:1789 Attribute.LabelListLateBoundDefault
attribute_default_value!(
    LabelListLateBoundDefaultValue,
    "LabelListLateBoundDefault",
    "com.google.devtools.build.lib.packages.Attribute$LabelListLateBoundDefault"
);
// Java: com/google/devtools/build/lib/packages/Attribute.java:1710 Attribute.AlwaysNullLateBoundDefault
attribute_default_value!(
    AlwaysNullLateBoundDefaultValue,
    "AlwaysNullLateBoundDefault",
    "com.google.devtools.build.lib.packages.Attribute$AlwaysNullLateBoundDefault"
);

/// A native rule's own `ComputedDefault` subclass or another `SimpleLateBoundDefault`: the Java
/// class is not recorded by blaze_packages, so its `repr` fails loudly.
/// PORT: `type()` of this value still answers "unknown" (starlark-rust's type name is static);
/// Java answers the subclass's simple name.
// Java: com/google/devtools/build/lib/packages/Attribute.java:1275 Attribute.ComputedDefault
// Java: com/google/devtools/build/lib/packages/Attribute.java:1580 Attribute.SimpleLateBoundDefault
#[derive(
    Debug, starlark::any::ProvidesStaticType, starlark::values::NoSerialize, allocative::Allocative,
)]
struct OpaqueAttributeDefault;

starlark::starlark_simple_value!(OpaqueAttributeDefault);

impl fmt::Display for OpaqueAttributeDefault {
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        blaze_util::unported!(
            "StarlarkValue.repr",
            "the Java class of a native ComputedDefault / LateBoundDefault"
        )
    }
}

#[starlark::values::starlark_value(type = "unknown")]
impl<'v> starlark::values::StarlarkValue<'v> for OpaqueAttributeDefault {}

/// A Starlark-defined transition bound to its `attr` object (Java
/// `StarlarkAttributeTransitionProvider.FunctionSplitTransition` and
/// `StarlarkRuleTransitionProvider.FunctionPatchTransition`, both `StarlarkTransition`s).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkTransition.java:51 StarlarkTransition
#[derive(Clone)]
pub struct StarlarkTransition {
    pub definition: Arc<StarlarkDefinedConfigTransition>,
    pub kind: StarlarkTransitionKind,
    pub attr_object: Arc<TransitionAttributes>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum StarlarkTransitionKind {
    /// Java `FunctionSplitTransition` of a `StarlarkAttributeTransitionProvider`.
    FunctionSplit {
        allow_immutable_flag_changes: bool,
        is_exec_transition_provider: bool,
    },
    /// Java `FunctionPatchTransition` of a `StarlarkRuleTransitionProvider`.
    FunctionPatch,
}

impl StarlarkTransition {
    /// Returns true if the transition is an exec transition.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttributeTransitionProvider.java:148 FunctionSplitTransition.isExecTransition
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleTransitionProvider.java:262 FunctionPatchTransition.isExecTransition
    pub fn is_exec_transition(&self) -> bool {
        match self.kind {
            StarlarkTransitionKind::FunctionSplit {
                is_exec_transition_provider,
                ..
            } => is_exec_transition_provider,
            StarlarkTransitionKind::FunctionPatch => false,
        }
    }

    /// The inputs of the starlark transition as a set of canonicalized labels.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkTransition.java:69 StarlarkTransition.getInputs
    pub fn get_inputs(&self) -> Vec<Label> {
        self.definition
            .get_inputs_canonicalized_to_given()
            .iter()
            .map(|(l, _)| l.clone())
            .collect()
    }

    /// The outputs of the starlark transition as a set of canonicalized labels.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkTransition.java:74 StarlarkTransition.getOutputs
    pub fn get_outputs(&self) -> Vec<Label> {
        self.definition
            .get_outputs_canonicalized_to_given()
            .iter()
            .map(|(l, _)| l.clone())
            .collect()
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkTransition.java:59 StarlarkTransition.getName
    pub fn get_name(&self) -> String {
        format!(
            "Starlark transition:{}",
            match self.definition.get_location() {
                Some(l) => l.to_string(),
                None => "null".to_owned(),
            }
        )
    }

    /// Identity for caching: the definition (a Starlark transition keeps the same instance for
    /// the life of the build arena, see [`starlark_defined_config_transition_of`]; the
    /// `StarlarkTransitionCache` that uses the key lives in the same arena) and the attribute
    /// values it sees.
    fn cache_key(&self) -> String {
        format!(
            "{:p}/{:?}/{:?}",
            Arc::as_ptr(&self.definition),
            self.kind,
            self.attr_object.entries
        )
    }
}

impl fmt::Debug for StarlarkTransition {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.get_name())
    }
}

/// A configuration transition (Java `ConfigurationTransition` and its implementations).
// Java: com/google/devtools/build/lib/analysis/config/transitions/ConfigurationTransition.java:38 ConfigurationTransition
#[derive(Clone, Debug)]
pub enum ConfigurationTransition {
    /// Java `NoTransition.INSTANCE`.
    NoTransition,
    /// Java `NoConfigTransition.INSTANCE`.
    NoConfigTransition,
    /// Java `ComposingTransitionFactory.ComposingTransition`.
    Composing(Box<ConfigurationTransition>, Box<ConfigurationTransition>),
    /// Java `ExecutionTransitionFactory.ExecTransitionFinalizer`.
    ExecTransitionFinalizer {
        execution_platform: Option<Label>,
        main_transition: StarlarkTransition,
    },
    /// Java `TestTrimmingTransitionFactory.TestTrimmingTransition`.
    TestTrimming { testonly: bool },
    /// Java `ConfigFeatureFlagTaggedTrimmingTransitionFactory.ConfigFeatureFlagTaggedTrimmingTransition`.
    ConfigFeatureFlagTaggedTrimming { flags: BTreeSet<Label> },
    /// Java `ConfigFeatureFlagTransitionFactory.ConfigFeatureFlagValuesTransition`: the flag
    /// values (an `ImmutableSortedMap`, natural label order).
    ConfigFeatureFlagValues {
        flag_values: BTreeMap<Label, String>,
    },
    /// Java `StarlarkTransition` subclasses.
    Starlark(StarlarkTransition),
}

impl ConfigurationTransition {
    /// Returns the option classes this transition requires to apply.
    // Java: com/google/devtools/build/lib/analysis/config/transitions/ConfigurationTransition.java:56 ConfigurationTransition.requiresOptionFragments
    pub fn requires_option_fragments(&self) -> Vec<FragmentOptionsClass> {
        match self {
            // Java: com/google/devtools/build/lib/analysis/config/transitions/NoConfigTransition.java:57 NoConfigTransition.requiresOptionFragments
            ConfigurationTransition::NoConfigTransition => vec![&CORE_OPTIONS],
            // Java: com/google/devtools/build/lib/analysis/config/ExecutionTransitionFactory.java:173 ExecTransitionFinalizer.requiresOptionFragments
            ConfigurationTransition::ExecTransitionFinalizer { .. } => {
                exec_transition_required_fragments()
            }
            // Java: com/google/devtools/build/lib/analysis/test/TestTrimmingTransitionFactory.java:63 TestTrimmingTransition.requiresOptionFragments
            ConfigurationTransition::TestTrimming { .. } => test_trimming_required_fragments(),
            ConfigurationTransition::ConfigFeatureFlagTaggedTrimming { .. } => {
                vec![&CONFIG_FEATURE_FLAG_OPTIONS, &CORE_OPTIONS]
            }
            // Java: com/google/devtools/build/lib/rules/config/ConfigFeatureFlagTransitionFactory.java:84 ConfigFeatureFlagValuesTransition.requiresOptionFragments
            ConfigurationTransition::ConfigFeatureFlagValues { .. } => {
                vec![&CONFIG_FEATURE_FLAG_OPTIONS]
            }
            ConfigurationTransition::NoTransition
            | ConfigurationTransition::Composing(..)
            | ConfigurationTransition::Starlark(_) => Vec::new(),
        }
    }

    /// Java `NoTransition.isInstance(transition)`.
    pub fn is_no_transition(&self) -> bool {
        matches!(self, ConfigurationTransition::NoTransition)
    }

    // Java: com/google/devtools/build/lib/analysis/config/transitions/ConfigurationTransition.java:95 ConfigurationTransition.getName
    pub fn get_name(&self) -> String {
        match self {
            ConfigurationTransition::NoTransition => "NoTransition".to_owned(),
            ConfigurationTransition::NoConfigTransition => "NoConfigTransition".to_owned(),
            // Java: com/google/devtools/build/lib/analysis/config/transitions/ComposingTransitionFactory.java:186 ComposingTransition.getName
            ConfigurationTransition::Composing(t1, t2) => {
                format!("({} + {})", t1.get_name(), t2.get_name())
            }
            // Java: com/google/devtools/build/lib/analysis/config/ExecutionTransitionFactory.java:158 ExecTransitionFinalizer.getName
            ConfigurationTransition::ExecTransitionFinalizer { .. } => "exec".to_owned(),
            ConfigurationTransition::TestTrimming { .. } => "TestTrimmingTransition".to_owned(),
            ConfigurationTransition::ConfigFeatureFlagTaggedTrimming { .. } => {
                "ConfigFeatureFlagTaggedTrimmingTransition".to_owned()
            }
            // ConfigurationTransition.getName: getClass().getSimpleName().
            ConfigurationTransition::ConfigFeatureFlagValues { .. } => {
                "ConfigFeatureFlagValuesTransition".to_owned()
            }
            ConfigurationTransition::Starlark(t) => t.get_name(),
        }
    }

    /// Visits this transition and its components (Java `visit(Visitor)`); `visitor` receives
    /// every leaf transition.
    // Java: com/google/devtools/build/lib/analysis/config/transitions/ConfigurationTransition.java:103 ConfigurationTransition.visit
    pub fn visit(&self, visitor: &mut dyn FnMut(&ConfigurationTransition)) {
        match self {
            // Java: com/google/devtools/build/lib/analysis/config/transitions/ComposingTransitionFactory.java:192 ComposingTransition.visit
            ConfigurationTransition::Composing(t1, t2) => {
                t1.visit(visitor);
                t2.visit(visitor);
            }
            // Java: com/google/devtools/build/lib/analysis/config/ExecutionTransitionFactory.java:167 ExecTransitionFinalizer.visit
            ConfigurationTransition::ExecTransitionFinalizer {
                main_transition, ..
            } => visitor(&ConfigurationTransition::Starlark(main_transition.clone())),
            other => visitor(other),
        }
    }

    /// Visits the Starlark transitions of this transition (Java `StarlarkTransitionVisitor`).
    pub fn visit_starlark(&self, visitor: &mut dyn FnMut(&StarlarkTransition)) {
        self.visit(&mut |t| {
            if let ConfigurationTransition::Starlark(s) = t {
                visitor(s);
            }
        });
    }

    /// Identity of this transition for `StarlarkTransitionCache` (Java `equals`/`hashCode`).
    pub fn cache_key(&self) -> String {
        match self {
            ConfigurationTransition::NoTransition => "NoTransition".to_owned(),
            ConfigurationTransition::NoConfigTransition => "NoConfigTransition".to_owned(),
            ConfigurationTransition::Composing(t1, t2) => {
                format!("({} + {})", t1.cache_key(), t2.cache_key())
            }
            ConfigurationTransition::ExecTransitionFinalizer {
                execution_platform,
                main_transition,
            } => format!(
                "exec({}, {})",
                Label::print(execution_platform.as_ref()),
                main_transition.cache_key()
            ),
            ConfigurationTransition::TestTrimming { testonly } => {
                format!("TestTrimming({testonly})")
            }
            ConfigurationTransition::ConfigFeatureFlagTaggedTrimming { flags } => {
                format!("ConfigFeatureFlagTaggedTrimmingTransition{{flags={flags:?}}}")
            }
            // Java: com/google/devtools/build/lib/rules/config/ConfigFeatureFlagTransitionFactory.java:120 ConfigFeatureFlagValuesTransition.toString
            ConfigurationTransition::ConfigFeatureFlagValues { flag_values } => format!(
                "ConfigFeatureFlagValuesTransition{{flagValues={{{}}}}}",
                flag_values
                    .iter()
                    .map(|(k, v)| format!("{}={}", k, v))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            ConfigurationTransition::Starlark(t) => t.cache_key(),
        }
    }

    /// Applies the transition to the options (Java `apply(BuildOptionsView, EventHandler)`).
    /// Errors are reported to `event_handler`.
    // Java: com/google/devtools/build/lib/analysis/config/transitions/ConfigurationTransition.java:74 ConfigurationTransition.apply
    pub fn apply(
        &self,
        build_options: &BuildOptionsView,
        event_handler: &dyn EventHandler,
        semantics: &Arc<StarlarkSemantics>,
    ) -> IndexMap<String, BuildOptions> {
        match self {
            ConfigurationTransition::Composing(t1, t2) => {
                // Java: com/google/devtools/build/lib/analysis/config/transitions/ComposingTransitionFactory.java:166 ComposingTransition.apply
                let mut to_options: IndexMap<String, BuildOptions> = IndexMap::new();
                let transition1_output = t1.apply(
                    &restrict(t1, build_options.underlying()),
                    event_handler,
                    semantics,
                );
                for (key1, value1) in &transition1_output {
                    let transition2_output =
                        t2.apply(&restrict(t2, value1), event_handler, semantics);
                    for (key2, value2) in transition2_output {
                        let key = compose_keys(key1, &key2);
                        if to_options.contains_key(&key) {
                            panic!(
                                "IllegalArgumentException: Multiple entries with same key: {key}"
                            );
                        }
                        to_options.insert(key, value2);
                    }
                }
                to_options
            }
            ConfigurationTransition::Starlark(t) => match t.kind {
                StarlarkTransitionKind::FunctionSplit {
                    allow_immutable_flag_changes,
                    is_exec_transition_provider,
                } => {
                    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttributeTransitionProvider.java:128 FunctionSplitTransition.split
                    // Starlark transitions already have logic to enforce they only access
                    // declared inputs and outputs. Rather than complicate BuildOptionsView with
                    // more access points to BuildOptions, we just use the original BuildOptions
                    // and trust the transition's enforcement logic.
                    let options = build_options.underlying();
                    let res = apply_and_validate_with_attrs(
                        options,
                        &t.definition,
                        allow_immutable_flag_changes,
                        is_exec_transition_provider,
                        &*t.attr_object,
                        semantics,
                        event_handler,
                    );
                    let res = match res {
                        Some(r) => r,
                        None => {
                            let mut m = IndexMap::new();
                            m.insert("error".to_owned(), options.clone());
                            m
                        }
                    };
                    // Java: com/google/devtools/build/lib/analysis/config/transitions/SplitTransition.java:62 SplitTransition.apply
                    blaze_util::check_state!(
                        !res.is_empty(),
                        "Split transition output may not be empty"
                    );
                    res
                }
                StarlarkTransitionKind::FunctionPatch => {
                    singleton(self.patch(build_options, event_handler, semantics))
                }
            },
            _ => singleton(self.patch(build_options, event_handler, semantics)),
        }
    }

    /// Java `PatchTransition.patch` of the patch transitions.
    fn patch(
        &self,
        options: &BuildOptionsView,
        event_handler: &dyn EventHandler,
        semantics: &Arc<StarlarkSemantics>,
    ) -> BuildOptions {
        match self {
            // Java: com/google/devtools/build/lib/analysis/config/transitions/NoTransition.java:44 NoTransition.patch
            ConfigurationTransition::NoTransition => options.underlying().clone(),
            // Java: com/google/devtools/build/lib/analysis/config/transitions/NoConfigTransition.java:62 NoConfigTransition.patch
            ConfigurationTransition::NoConfigTransition => EMPTY_OPTIONS.clone(),
            // Java: com/google/devtools/build/lib/analysis/config/ExecutionTransitionFactory.java:181 ExecTransitionFinalizer.patch
            ConfigurationTransition::ExecTransitionFinalizer {
                execution_platform,
                main_transition,
            } => exec_transition_patch(
                execution_platform.as_ref(),
                &main_transition.definition,
                options,
                semantics,
                event_handler,
            ),
            // Java: com/google/devtools/build/lib/analysis/test/TestTrimmingTransitionFactory.java:68 TestTrimmingTransition.patch
            ConfigurationTransition::TestTrimming { testonly } => {
                let Some(original_test_options) = options.get::<TestConfigurationTestOptions>()
                else {
                    return options.underlying().clone();
                };
                if !original_test_options.trim_test_configuration
                    || (original_test_options
                        .experimental_retain_test_configuration_across_testonly
                        && *testonly)
                {
                    return options.underlying().clone();
                }
                test_trimming_trim_view(options)
            }
            // Java: com/google/devtools/build/lib/rules/config/ConfigFeatureFlagTransitionFactory.java:89 ConfigFeatureFlagValuesTransition.patch
            ConfigurationTransition::ConfigFeatureFlagValues { flag_values } => {
                if !options.contains(&CONFIG_FEATURE_FLAG_OPTIONS) {
                    return options.underlying().clone();
                }
                let values: IndexMap<Label, String> = flag_values
                    .iter()
                    .map(|(k, v)| (k.clone(), v.clone()))
                    .collect();
                let mut to_options =
                    FeatureFlagValue::replace_flag_values(options.underlying(), &values);
                // In legacy mode, need to update `affected by Starlark transition` to include
                // changed flags.
                if to_options
                    .get::<CoreOptions>()
                    .expect("CoreOptions")
                    .output_directory_naming_scheme
                    == CoreOptionsOutputDirectoryNamingScheme::LEGACY
                {
                    let changed = get_affected_by_starlark_transition_via_diff(
                        &to_options,
                        options.underlying(),
                    );
                    update_affected_by_starlark_transition(
                        to_options.get_mut::<CoreOptions>().expect("CoreOptions"),
                        &changed,
                    );
                }
                to_options
            }
            // Java: com/google/devtools/build/lib/rules/config/ConfigFeatureFlagTaggedTrimmingTransitionFactory.java:62 ConfigFeatureFlagTaggedTrimmingTransition.patch
            ConfigurationTransition::ConfigFeatureFlagTaggedTrimming { flags } => {
                let Some(config_feature_flag_options) = options.get::<ConfigFeatureFlagOptions>()
                else {
                    return options.underlying().clone();
                };
                if !config_feature_flag_options.enforce_transitive_configs_for_config_feature_flag {
                    return options.underlying().clone();
                }
                let mut to_options =
                    FeatureFlagValue::trim_flag_values(options.underlying(), flags);
                // In legacy mode, need to update `affected by Starlark transition` to include
                // changed flags.
                if to_options
                    .get::<CoreOptions>()
                    .expect("CoreOptions")
                    .output_directory_naming_scheme
                    == CoreOptionsOutputDirectoryNamingScheme::LEGACY
                {
                    let changed = get_affected_by_starlark_transition_via_diff(
                        &to_options,
                        options.underlying(),
                    );
                    update_affected_by_starlark_transition(
                        to_options.get_mut::<CoreOptions>().expect("CoreOptions"),
                        &changed,
                    );
                }
                to_options
            }
            // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleTransitionProvider.java:238 FunctionPatchTransition.patch
            ConfigurationTransition::Starlark(t) => {
                // Starlark transitions already have logic to enforce they only access declared
                // inputs and outputs. Rather than complicate BuildOptionsView with more access
                // points to BuildOptions, we just use the original BuildOptions and trust the
                // transition's enforcement logic.
                let build_options = options.underlying();
                let result = apply_and_validate_with_attrs(
                    build_options,
                    &t.definition,
                    /* allow_non_configurable_flag_changes= */ false,
                    t.is_exec_transition(),
                    &*t.attr_object,
                    semantics,
                    event_handler,
                );
                let Some(result) = result else {
                    return build_options.clone();
                };
                if result.len() != 1 {
                    event_handler.handle(&Event::error_at(
                        t.definition.get_location().map(to_event_location),
                        "Rule transition only allowed to return a single transitioned configuration.",
                    ));
                    return build_options.clone();
                }
                result.into_values().next().unwrap()
            }
            ConfigurationTransition::Composing(..) => {
                unreachable!("ComposingTransition is not a PatchTransition")
            }
        }
    }
}

fn singleton(options: BuildOptions) -> IndexMap<String, BuildOptions> {
    let mut m = IndexMap::new();
    m.insert(PATCH_TRANSITION_KEY.to_owned(), options);
    m
}

/// Composes a new key out of two given keys. If either transition is a patch (1:1) its
/// placeholder `PATCH_TRANSITION_KEY` is absorbed; if both transitions split, the keys are
/// joined by a comma so the composition produces the cross product of their splits.
// Java: com/google/devtools/build/lib/analysis/config/transitions/ComposingTransitionFactory.java:214 ComposingTransition.composeKeys
fn compose_keys(key1: &str, key2: &str) -> String {
    if key1 == PATCH_TRANSITION_KEY {
        return key2.to_owned();
    }
    if key2 == PATCH_TRANSITION_KEY {
        return key1.to_owned();
    }
    format!("{key1},{key2}")
}

/// Returns a `BuildOptionsView` restricted to the transition's required fragments.
// Java: com/google/devtools/build/lib/analysis/config/transitions/TransitionUtil.java:27 TransitionUtil.restrict
pub fn restrict<'a>(
    transition: &ConfigurationTransition,
    options: &'a BuildOptions,
) -> BuildOptionsView<'a> {
    BuildOptionsView::new(options, transition.requires_option_fragments())
}

/// `blaze_starlark::Location` → event location.
pub fn to_event_location(location: &blaze_starlark::Location) -> blaze_events::Location {
    blaze_events::Location::new(
        location.file.clone(),
        location.line as i32,
        location.column as i32,
    )
}

// -------------------------------------------------------------------------------------------
// Starlark transition providers.

/// Java `StarlarkAttributeTransitionProvider` (and `StarlarkExecTransitionProvider`, the
/// provider of the Starlark exec transition: immutable flag changes allowed, an exec transition
/// provider).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttributeTransitionProvider.java:47 StarlarkAttributeTransitionProvider
#[derive(Clone, Debug)]
pub struct StarlarkAttributeTransitionProvider {
    pub starlark_defined_config_transition: Arc<StarlarkDefinedConfigTransition>,
    pub allow_immutable_flag_changes: bool,
    pub is_exec_transition_provider: bool,
}

impl StarlarkAttributeTransitionProvider {
    pub fn new(definition: Arc<StarlarkDefinedConfigTransition>) -> Self {
        StarlarkAttributeTransitionProvider {
            starlark_defined_config_transition: definition,
            allow_immutable_flag_changes: false,
            is_exec_transition_provider: false,
        }
    }

    /// Java `StarlarkExecTransitionLoader.StarlarkExecTransitionProvider`.
    // Java: com/google/devtools/build/lib/analysis/config/StarlarkExecTransitionLoader.java:58 StarlarkExecTransitionLoader.StarlarkExecTransitionProvider
    pub fn new_exec(definition: Arc<StarlarkDefinedConfigTransition>) -> Self {
        StarlarkAttributeTransitionProvider {
            starlark_defined_config_transition: definition,
            allow_immutable_flag_changes: true,
            is_exec_transition_provider: true,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttributeTransitionProvider.java:73 StarlarkAttributeTransitionProvider.create
    pub fn create(&self, attribute_map: Option<&ConfiguredAttributeMapper>) -> StarlarkTransition {
        // TODO(bazel-team): consider caching transition instances to save CPU time, similar to
        // what's done in StarlarkRuleTransitionProvider. This could benefit builds that apply
        // transitions over many build graph edges.
        // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkAttributeTransitionProvider.java:111 FunctionSplitTransition.<init>
        let mut attributes: Vec<(String, Option<AttrValue>)> = Vec::new();
        if let Some(attribute_map) = attribute_map {
            for attribute in attribute_map.get_attribute_names() {
                let type_ = attribute_map
                    .get_attribute_type(&attribute)
                    .expect("attribute type");
                let val = attribute_map.get(&attribute, type_);
                // PORT: Java swallows `InvalidStarlarkValueException` for values that have no
                // Starlark form (native targets only); every `AttrValue` has one here.
                let name = Attribute::get_starlark_name(&attribute);
                if let Some(e) = attributes.iter_mut().find(|(k, _)| *k == name) {
                    e.1 = val;
                } else {
                    attributes.push((name, val));
                }
            }
        }
        StarlarkTransition {
            definition: self.starlark_defined_config_transition.clone(),
            kind: StarlarkTransitionKind::FunctionSplit {
                allow_immutable_flag_changes: self.allow_immutable_flag_changes,
                is_exec_transition_provider: self.is_exec_transition_provider,
            },
            attr_object: Arc::new(TransitionAttributes {
                entries: attributes,
                no_attr_error: ERROR_MESSAGE_FOR_NO_ATTR,
            }),
        }
    }
}

/// Java `StarlarkRuleTransitionProvider`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleTransitionProvider.java:52 StarlarkRuleTransitionProvider
#[derive(Clone, Debug)]
pub struct StarlarkRuleTransitionProvider {
    pub starlark_defined_config_transition: Arc<StarlarkDefinedConfigTransition>,
}

impl StarlarkRuleTransitionProvider {
    /// Java `create(ruleData)`.
    ///
    /// PORT: `StarlarkDefinedConfigTransition.createRuleTransition` caches the transition per
    /// `RuleTransitionData`; recreating it yields an equal transition.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleTransitionProvider.java:66 StarlarkRuleTransitionProvider.create
    pub fn create(&self, rule_data: &RuleTransitionData) -> StarlarkTransition {
        self.create_transition(rule_data)
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleTransitionProvider.java:87 StarlarkRuleTransitionProvider.createTransition
    fn create_transition(&self, rule_data: &RuleTransitionData) -> StarlarkTransition {
        let rule = &rule_data.rule;
        let config_conditions = &rule_data.config_conditions;
        let config_hash = &rule_data.config_hash;
        let mut attributes: Vec<(String, Option<AttrValue>)> = Vec::new();
        let attribute_mapper = RawAttributeMapper::of(&**rule as &dyn RuleOrMacroInstance);
        let configured_attribute_mapper = ConfiguredAttributeMapper::of_hash(
            rule.clone(),
            config_conditions.clone(),
            config_hash.clone(),
            false,
        );
        let transition_outputs = self.starlark_defined_config_transition.get_outputs();

        for attribute in rule.get_attribute_provider().get_attributes() {
            // If the value is present, even if it is null, add to the attribute map.
            let mut val = attribute_mapper.get_raw_attribute_value_of(rule, attribute);
            if let Some(AttrValue::SelectorList(sl)) = &val {
                let sl = sl.clone();
                match self.handle_configured_attribute(
                    config_conditions,
                    &configured_attribute_mapper,
                    &transition_outputs,
                    attribute,
                    &sl,
                ) {
                    // Skip this attribute.
                    None => continue,
                    Some(resolved) => val = resolved,
                }
            }
            let name = Attribute::get_starlark_name(&attribute.get_public_name());
            if let Some(e) = attributes.iter_mut().find(|(k, _)| *k == name) {
                e.1 = val;
            } else {
                attributes.push((name, val));
            }
        }

        StarlarkTransition {
            definition: self.starlark_defined_config_transition.clone(),
            kind: StarlarkTransitionKind::FunctionPatch,
            attr_object: Arc::new(TransitionAttributes {
                entries: attributes,
                no_attr_error: RULE_TRANSITION_NO_ATTR,
            }),
        }
    }

    /// `None` is Java's `Result.failure()`; `Some(resolved)` is `Result.success(resolved)`.
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleTransitionProvider.java:146 StarlarkRuleTransitionProvider.handleConfiguredAttribute
    fn handle_configured_attribute(
        &self,
        config_conditions: &IndexMap<Label, ConfigMatchingProvider>,
        configured_attribute_mapper: &ConfiguredAttributeMapper,
        transition_outputs: &[String],
        attribute: &Attribute,
        val: &blaze_packages::types::BuildTypeSelectorList,
    ) -> Option<Option<AttrValue>> {
        // If there are no configConditions then nothing is resolvable.
        if config_conditions.is_empty() {
            return None;
        }

        // If any of the select keys reference the outputs, this isn't resolvable.
        if Self::select_branches_reference_outputs(config_conditions, transition_outputs, val) {
            return None;
        }

        // Resolve the attribute, ignoring any failures. They will be reported (and fail
        // analysis) later in the rule analysis.
        match configured_attribute_mapper.get_resolved_attribute_of(attribute) {
            AttributeResolutionResult::Failure(_) => None,
            AttributeResolutionResult::Success(v) => Some(v),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleTransitionProvider.java:172 StarlarkRuleTransitionProvider.selectBranchesReferenceOutputs
    fn select_branches_reference_outputs(
        config_conditions: &IndexMap<Label, ConfigMatchingProvider>,
        transition_outputs: &[String],
        val: &blaze_packages::types::BuildTypeSelectorList,
    ) -> bool {
        for label in val.get_key_labels() {
            let config_matching_provider = config_conditions
                .get(&label)
                .expect("NullPointerException: config condition of a select key");
            if Self::check_if_attribute_select_on_a_flag_transition_changes(
                config_matching_provider,
                transition_outputs,
            ) {
                return true;
            }
        }
        false
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleTransitionProvider.java:187 StarlarkRuleTransitionProvider.checkIfAttributeSelectOnAFlagTransitionChanges
    fn check_if_attribute_select_on_a_flag_transition_changes(
        config_matching_provider: &ConfigMatchingProvider,
        transition_outputs: &[String],
    ) -> bool {
        // check settingMap
        let native_flag_labels: std::collections::HashSet<String> = config_matching_provider
            .settings_map()
            .iter()
            .map(|(key, _)| format!("//command_line_option:{key}"))
            .collect();
        // check flags values
        let flag_labels: std::collections::HashSet<String> = config_matching_provider
            .flag_settings_map()
            .keys()
            .map(|flag| flag.get_canonical_form())
            .collect();

        for output in transition_outputs {
            if native_flag_labels.contains(output) || flag_labels.contains(output) {
                return true;
            }
        }
        false
    }
}

// -------------------------------------------------------------------------------------------
// Converting loading-phase transition objects.

/// The analysis form of each loading-phase `transition()` object, memoized by the frozen
/// value's identity so the same `transition()` keeps one `StarlarkDefinedConfigTransition` (Java
/// keeps one instance per object). In the build arena (`blaze_util::build_arena`, decision D7),
/// released before the .bzl heaps the identities are addresses in.
#[derive(Default)]
struct ConvertedTransitions(Mutex<Vec<(String, Arc<StarlarkDefinedConfigTransition>)>>);

impl blaze_util::build_arena::ArenaSlot for ConvertedTransitions {
    const RELEASE_ORDER: blaze_util::build_arena::ReleaseOrder =
        blaze_util::build_arena::ReleaseOrder::Memo;
}

/// The `StarlarkDefinedConfigTransition` of a loading-phase transition object held in a
/// `TransitionFactory::Starlark` (W3's `blaze_config::transitions` value), converted with
/// `StarlarkDefinedConfigTransition::from_parts`.
pub fn starlark_defined_config_transition_of(
    transition: &StarlarkCallable,
) -> Arc<StarlarkDefinedConfigTransition> {
    let frozen = transition.value();
    let identity = format!("{:?}", frozen.to_value().identity());
    let converted_transitions = blaze_util::build_arena::slot::<ConvertedTransitions>();
    if let Some((_, t)) = converted_transitions
        .0
        .lock()
        .iter()
        .find(|(id, _)| *id == identity)
    {
        return t.clone();
    }
    let value = frozen.to_value();
    let loading = LoadingTransition::from_value(value).unwrap_or_else(|| {
        panic!(
            "IllegalStateException: transition factory holds a non-transition value {}",
            value
        )
    });
    let data = loading.data();
    let kind = match data.kind {
        LoadingTransitionKind::Regular => TransitionKind::Regular,
        LoadingTransitionKind::Exec => TransitionKind::Exec,
        LoadingTransitionKind::AnalysisTest => TransitionKind::AnalysisTest,
    };
    // An AnalysisTestTransition has no implementation function; its object stands in (never
    // called).
    let implementation = match kind {
        TransitionKind::AnalysisTest => frozen.to_value().unpack_frozen().expect("frozen"),
        _ => loading
            .get_impl()
            .and_then(|v| v.unpack_frozen())
            .expect("a regular transition has a frozen implementation"),
    };
    // SAFETY: both values live on frozen .bzl heaps that BZL_LOAD keeps in the build arena
    // (PORT: keep-alive), as `StarlarkCallable::to_owned_frozen_value` relies on. The
    // transition is held by this memo (same arena, released before the heaps) and by values of
    // the command's evaluators; the arena is released only once no evaluator is left.
    let implementation = unsafe { OwnedFrozenValue::new(FrozenHeapRef::default(), implementation) };
    // AnalysisTestTransition.changedSettings: the dict values are Starlark objects.
    let changed_settings: Vec<(String, blaze_build_config::starlark_transition::JavaObject)> =
        loading
            .changed_settings()
            .map(|(k, v)| {
                (
                    k.to_owned(),
                    blaze_build_config::starlark_transition::to_java_object(v).unwrap_or_else(
                        |t| {
                            blaze_util::unported!(
                                "StarlarkDefinedConfigTransition.AnalysisTestTransition",
                                "changed setting value of type {}",
                                t
                            )
                        },
                    ),
                )
            })
            .collect();
    let converted = Arc::new(StarlarkDefinedConfigTransition::from_parts(
        transition.to_owned_frozen_value(),
        implementation,
        TransitionData {
            kind,
            inputs_canonicalized_to_given: data
                .inputs_canonicalized_to_given
                .iter()
                .map(|(l, g)| (l.clone(), g.clone()))
                .collect(),
            outputs_canonicalized_to_given: data
                .outputs_canonicalized_to_given
                .iter()
                .map(|(l, g)| (l.clone(), g.clone()))
                .collect(),
            parent_label: data.parent_label.clone(),
            location: Some(data.location.clone()),
            package_context: data.package_context.clone(),
            repo_mapping: data.package_context.repo_mapping().clone(),
            changed_settings,
        },
    ));
    let mut converted_list = converted_transitions.0.lock();
    if let Some((_, t)) = converted_list.iter().find(|(id, _)| *id == identity) {
        return t.clone();
    }
    converted_list.push((identity, converted.clone()));
    converted
}

// -------------------------------------------------------------------------------------------
// TransitionFactory.create.

/// Java `TransitionFactory<AttributeTransitionData>.create(data)` for the loading-phase factory
/// of an attribute.
// Java: com/google/devtools/build/lib/analysis/config/transitions/TransitionFactory.java:68 TransitionFactory.create
pub fn create_attribute_transition(
    factory: &TransitionFactory,
    data: &AttributeTransitionData<'_>,
) -> Result<ConfigurationTransition, TransitionCreationException> {
    Ok(match factory {
        // Java: com/google/devtools/build/lib/analysis/config/transitions/NoTransition.java:70 NoTransition.Factory.create
        TransitionFactory::NoTransition | TransitionFactory::Target => {
            ConfigurationTransition::NoTransition
        }
        TransitionFactory::Exec { .. } => execution_transition_factory_create(data)?,
        TransitionFactory::Starlark { transition, .. } => ConfigurationTransition::Starlark(
            StarlarkAttributeTransitionProvider::new(starlark_defined_config_transition_of(
                transition,
            ))
            .create(data.attributes),
        ),
        // Java: com/google/devtools/build/lib/analysis/config/transitions/ComposingTransitionFactory.java:107 ComposingTransitionFactory.create
        TransitionFactory::Composing { first, second } => {
            let transition1 = create_attribute_transition(first, data)?;
            let transition2 = create_attribute_transition(second, data)?;
            ConfigurationTransition::Composing(Box::new(transition1), Box::new(transition2))
        }
        TransitionFactory::Native { name, .. } => create_native(name),
        TransitionFactory::ConfigFeatureFlag { .. } => {
            panic!("IllegalStateException: ConfigFeatureFlagTransitionFactory is a rule transition")
        }
    })
}

/// Java `TransitionFactory<RuleTransitionData>.create(data)` for the loading-phase factory of a
/// rule class.
// Java: com/google/devtools/build/lib/analysis/config/transitions/TransitionFactory.java:68 TransitionFactory.create
pub fn create_rule_transition(
    factory: &TransitionFactory,
    data: &RuleTransitionData,
) -> ConfigurationTransition {
    match factory {
        TransitionFactory::NoTransition | TransitionFactory::Target => {
            ConfigurationTransition::NoTransition
        }
        TransitionFactory::Starlark { transition, .. } => ConfigurationTransition::Starlark(
            StarlarkRuleTransitionProvider {
                starlark_defined_config_transition: starlark_defined_config_transition_of(
                    transition,
                ),
            }
            .create(data),
        ),
        TransitionFactory::Composing { first, second } => ConfigurationTransition::Composing(
            Box::new(create_rule_transition(first, data)),
            Box::new(create_rule_transition(second, data)),
        ),
        TransitionFactory::Native { name, .. } => create_native(name),
        TransitionFactory::ConfigFeatureFlag { attribute_name } => {
            config_feature_flag_transition_factory_create(attribute_name, data)
        }
        TransitionFactory::Exec { .. } => {
            panic!("IllegalStateException: ExecutionTransitionFactory is not a rule transition")
        }
    }
}

// Java: com/google/devtools/build/lib/rules/config/ConfigFeatureFlagTransitionFactory.java:137 ConfigFeatureFlagTransitionFactory.create
fn config_feature_flag_transition_factory_create(
    attribute_name: &str,
    rule_data: &RuleTransitionData,
) -> ConfigurationTransition {
    let attrs = NonconfiguredAttributeMapper::of(&rule_data.rule);
    if attrs.is_attribute_value_explicitly_specified(attribute_name) {
        let flag_values = match attrs.get(attribute_name, Type::LabelKeyedStringDict) {
            Some(AttrValue::LabelKeyedStringDict(m)) => m.into_iter().collect(),
            other => panic!(
                "ClassCastException: {} of {} is {:?}, not a label-keyed string dict",
                attribute_name,
                rule_data.rule.get_label(),
                other
            ),
        };
        ConfigurationTransition::ConfigFeatureFlagValues { flag_values }
    } else {
        ConfigurationTransition::NoTransition
    }
}

fn create_native(name: &str) -> ConfigurationTransition {
    match name {
        // Java: com/google/devtools/build/lib/analysis/config/transitions/NoConfigTransition.java:87 NoConfigTransition.Factory.create
        "NoConfigTransition" => ConfigurationTransition::NoConfigTransition,
        other => blaze_util::unported!("TransitionFactory.create", "native factory {}", other),
    }
}

/// Java `ExecutionTransitionFactory.create(dataWithTargetAttributes)`.
// Java: com/google/devtools/build/lib/analysis/config/ExecutionTransitionFactory.java:73 ExecutionTransitionFactory.create
fn execution_transition_factory_create(
    data_with_target_attributes: &AttributeTransitionData<'_>,
) -> Result<ConfigurationTransition, TransitionCreationException> {
    // Delete AttributeTransitionData.attributes() so the exec transition doesn't try to read the
    // attributes of the target it's attached to. This is for two reasons:
    //
    //   1) While per-target exec transitions may be interesting, we're not ready to expose that
    //      level of API flexibility
    //   2) No need for StarlarkTransitionCache misses due to different StarlarkTransition
    //      instances bound to different attributes that shouldn't affect output.
    let data = AttributeTransitionData {
        attributes: None,
        execution_platform: data_with_target_attributes.execution_platform.clone(),
        analysis_data: data_with_target_attributes.analysis_data.clone(),
    };

    let Some(starlark_exec_transition_provider) = &data.analysis_data else {
        // TODO: https://github.com/bazelbuild/rules_license/issues/148 - This is a temporary hack
        // to prevent the license() rule's license_kinds attribute from failing.
        //
        // license() rules are already configured in NoConfigTransition. So when they traverse
        // 'cfg = "exec"' on their license_kind attribute, without this special case they'd
        // throw the below TransitionCreationException.
        let has_license_kinds = data_with_target_attributes
            .attributes
            .expect("NullPointerException: attributes")
            .has("license_kinds");
        if has_license_kinds {
            return Ok(ConfigurationTransition::NoConfigTransition);
        }
        return Err(TransitionCreationException(
            "expected a Starlark exec transition definition, but was null".to_owned(),
        ));
    };

    // PORT: Java's `transitionInstanceCache` only shares instances; equal transitions are
    // created here instead.
    Ok(ConfigurationTransition::ExecTransitionFinalizer {
        execution_platform: data.execution_platform.clone(),
        main_transition: starlark_exec_transition_provider.create(data.attributes),
    })
}

/// Returns the exec group of the `ExecutionTransitionFactory` within the given (possibly
/// composed) transition factory, or `None` if there is none.
// Java: com/google/devtools/build/lib/analysis/DependencyResolutionHelpers.java:252 DependencyResolutionHelpers.findExecutionTransitionFactory
pub fn find_execution_transition_factory(factory: &TransitionFactory) -> Option<String> {
    match factory {
        TransitionFactory::Exec { exec_group } => Some(exec_group.clone().unwrap_or_else(|| {
            blaze_config::transition_factories::DEFAULT_EXEC_GROUP_NAME.to_owned()
        })),
        TransitionFactory::Composing { first, second } => {
            // A composition contains at most one native transition, so there is at most one
            // match; Java keeps the last one visited.
            let a = find_execution_transition_factory(first);
            let b = find_execution_transition_factory(second);
            b.or(a)
        }
        _ => None,
    }
}

// -------------------------------------------------------------------------------------------
// Trimming transition factories (Java `ConfiguredRuleClassProvider.getTrimmingTransitionFactory`).

/// The rule-trimming transition factories Bazel's rule sets register, in registration order
/// (`CoreRules`: `TestTrimmingTransitionFactory`; `ConfigRules`:
/// `ConfigFeatureFlagTaggedTrimmingTransitionFactory(TAGGED_TRIMMING_ATTR)`), composed with
/// `ComposingTransitionFactory.ofUnchecked`.
// Java: com/google/devtools/build/lib/analysis/ConfiguredRuleClassProvider.java:388 ConfiguredRuleClassProvider.Builder.addTrimmingTransitionFactory
pub fn create_trimming_transition(rule_data: &RuleTransitionData) -> ConfigurationTransition {
    // ComposingTransitionFactory.create(t1, t2): both are rule factories, neither is
    // NoTransition.getFactory().
    ConfigurationTransition::Composing(
        Box::new(test_trimming_transition_factory_create(rule_data)),
        Box::new(
            config_feature_flag_tagged_trimming_transition_factory_create(
                TAGGED_TRIMMING_ATTR,
                rule_data,
            ),
        ),
    )
}

/// Java `ComposingTransitionFactory.ofUnchecked(ruleClassFactory, trimmingTransitionFactory)
/// .create(ruleData)` as `RuleTransitionApplier.computeTransition` does it.
// Java: com/google/devtools/build/lib/analysis/producers/RuleTransitionApplier.java:229 RuleTransitionApplier.computeTransition
pub fn create_rule_transition_with_trimming(
    rule_class_factory: &TransitionFactory,
    rule_data: &RuleTransitionData,
) -> ConfigurationTransition {
    // Java: com/google/devtools/build/lib/analysis/config/transitions/ComposingTransitionFactory.java:50 ComposingTransitionFactory.of
    // The trimming factories are RULE transitions and not tools, so the compatibility checks
    // pass for every rule class factory.
    if rule_class_factory.is_no_transition() {
        // Since transitionFactory1 causes no changes, use transitionFactory2 directly.
        return create_trimming_transition(rule_data);
    }
    ConfigurationTransition::Composing(
        Box::new(create_rule_transition(rule_class_factory, rule_data)),
        Box::new(create_trimming_transition(rule_data)),
    )
}

/// The labels in `TestOptions` (Java `TEST_OPTIONS`: the option names of `TestOptions`).
// Java: com/google/devtools/build/lib/analysis/test/TestTrimmingTransitionFactory.java:40 TestTrimmingTransitionFactory.TEST_OPTIONS
fn test_options_names() -> Vec<&'static str> {
    blaze_build_config::fragment_options_class::TEST_OPTIONS
        .get_option_definitions()
        .map(|d| d.get_option_name())
        .collect()
}

// Java: com/google/devtools/build/lib/analysis/test/TestTrimmingTransitionFactory.java:43 TestTrimmingTransitionFactory.TRANSITIVE_CONFIG_TO_TRIGGER_SKIP
static TRANSITIVE_CONFIG_TO_TRIGGER_SKIP: LazyLock<Label> =
    LazyLock::new(|| Label::parse_canonical_unchecked("//command_line_option/fragment:test"));

/// Java `AliasProvider.mayBeAlias(rule)`: the rule class can have any provider.
// Java: com/google/devtools/build/lib/analysis/AliasProvider.java:103 AliasProvider.mayBeAlias
pub fn may_be_alias(rule: &Rule) -> bool {
    let provider_set = rule.get_rule_class_object().get_advertised_providers();
    provider_set.can_have_any_provider()
        || provider_set
            .get_builtin_providers()
            .contains("AliasProvider")
        || provider_set
            .get_builtin_providers()
            .contains("LateBoundAliasProvider")
}

// Java: com/google/devtools/build/lib/analysis/test/TestTrimmingTransitionFactory.java:89 TestTrimmingTransitionFactory.create
fn test_trimming_transition_factory_create(
    rule_data: &RuleTransitionData,
) -> ConfigurationTransition {
    let rule = &rule_data.rule;
    let rule_class = rule.get_rule_class_object();
    if rule_class
        .get_configuration_fragment_policy()
        .is_legal_configuration_fragment("TestConfiguration", None)
        || may_be_alias(rule)
    {
        return ConfigurationTransition::NoTransition;
    }

    if let Some(option_reference_function) = rule_class.get_option_reference_function() {
        let test_options = test_options_names();
        for referenced_options in option_reference_function(rule) {
            if test_options.contains(&referenced_options.as_str()) {
                return ConfigurationTransition::NoTransition;
            }
        }
    }

    let attrs = NonconfigurableAttributeMapper::of(&**rule as &dyn RuleOrMacroInstance);

    if attrs.has_typed(TAGGED_TRIMMING_ATTR, Type::NodepLabelList) {
        if let Some(AttrValue::LabelList(entries)) =
            attrs.get(TAGGED_TRIMMING_ATTR, Type::NodepLabelList)
        {
            for entry in entries {
                if entry == *TRANSITIVE_CONFIG_TO_TRIGGER_SKIP {
                    return ConfigurationTransition::NoTransition;
                }
            }
        }
    }

    if attrs.has_typed("testonly", Type::Boolean)
        && matches!(
            attrs.get("testonly", Type::Boolean),
            Some(AttrValue::Bool(true))
        )
    {
        return ConfigurationTransition::TestTrimming { testonly: true };
    }
    ConfigurationTransition::TestTrimming { testonly: false }
}

// Java: com/google/devtools/build/lib/rules/config/ConfigFeatureFlagTaggedTrimmingTransitionFactory.java:111 ConfigFeatureFlagTaggedTrimmingTransitionFactory.create
fn config_feature_flag_tagged_trimming_transition_factory_create(
    attribute_name: &str,
    rule_data: &RuleTransitionData,
) -> ConfigurationTransition {
    let rule = &rule_data.rule;
    let attrs = NonconfiguredAttributeMapper::of(rule);
    let rule_class = rule.get_rule_class_object();

    if may_be_alias(rule) {
        // As a convenience, do not require transitive_config to be set for alias rule.
        return ConfigurationTransition::NoTransition;
    }

    // Java: ConfigRuleClasses.ConfigFeatureFlagRule.RULE_NAME
    if rule_class.get_name() == "config_feature_flag" {
        let mut flags = BTreeSet::new();
        flags.insert(rule.get_label().clone());
        return ConfigurationTransition::ConfigFeatureFlagTaggedTrimming { flags };
    }

    let mut required_labels: BTreeSet<Label> = BTreeSet::new();
    if attrs.is_attribute_value_explicitly_specified(attribute_name) {
        if let Some(AttrValue::LabelList(entries)) = attrs.get(attribute_name, Type::NodepLabelList)
        {
            // Entries starting with //command_line_option[:/] represent native options and are
            // not relevant for this transition. Non-existent flags already do not error so this
            // skipping is done out of an abundance of caution and as a statement of intent for
            // the future.
            for entry in entries {
                let package_name = entry.get_package_name();
                if package_name == "command_line_option"
                    || package_name.starts_with("command_line_option/")
                {
                    continue;
                }
                required_labels.insert(entry);
            }
        }
    }
    if let TransitionFactory::ConfigFeatureFlag { attribute_name } =
        rule_class.get_transition_factory()
    {
        let setting_attribute = attribute_name;
        // Because the process of setting a flag also creates a dependency on that flag, we need
        // to include all the set flags, even if they aren't actually declared as used by this
        // rule.
        if let Some(AttrValue::LabelKeyedStringDict(m)) =
            attrs.get(setting_attribute, Type::LabelKeyedStringDict)
        {
            required_labels.extend(m.into_keys());
        }
    }

    // Java returns `ConfigFeatureFlagTaggedTrimmingTransition.EMPTY` for an empty set.
    ConfigurationTransition::ConfigFeatureFlagTaggedTrimming {
        flags: required_labels,
    }
}

/// Java `ConfiguredRuleClassProvider.getToolchainTaggedTrimmingTransition()`
/// (`ConfigFeatureFlagTaggedTrimmingTransition.EMPTY`, set by `ConfigRules`).
// Java: com/google/devtools/build/lib/rules/config/ConfigRules.java:44 ConfigRules.init
pub fn toolchain_tagged_trimming_transition() -> ConfigurationTransition {
    ConfigurationTransition::ConfigFeatureFlagTaggedTrimming {
        flags: BTreeSet::new(),
    }
}

/// Java `toolchainTaggedTrimmingTransition.patch(new BuildOptionsView(options,
/// requiresOptionFragments()), listener)`.
pub fn patch_toolchain_tagged_trimming(
    options: &BuildOptions,
    event_handler: &dyn EventHandler,
    semantics: &Arc<StarlarkSemantics>,
) -> BuildOptions {
    let transition = toolchain_tagged_trimming_transition();
    transition.patch(&restrict(&transition, options), event_handler, semantics)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Java: com/google/devtools/build/lib/analysis/config/transitions/ComposingTransitionFactoryTest.java composeKeys
    #[test]
    fn compose_keys_absorbs_patch_keys() {
        assert_eq!(compose_keys("", "x"), "x");
        assert_eq!(compose_keys("a", ""), "a");
        assert_eq!(compose_keys("a", "x"), "a,x");
        assert_eq!(compose_keys("", ""), "");
    }

    #[test]
    fn requires_option_fragments_of_native_transitions() {
        assert!(
            ConfigurationTransition::NoTransition
                .requires_option_fragments()
                .is_empty()
        );
        assert_eq!(
            ConfigurationTransition::NoConfigTransition
                .requires_option_fragments()
                .len(),
            1
        );
    }

    #[test]
    #[ignore = "needs a BuildOptions fixture with TestOptions (ComposingTransitionFactoryTest)"]
    fn composing_transition_applies_both() {}
}

#[cfg(test)]
mod attribute_default_value_tests {
    use super::*;

    // Java Starlark.type / repr of an Attribute.StarlarkComputedDefault and of the late-bound
    // defaults seen by a transition's `attr` (analysis review S2-15).
    #[test]
    fn computed_and_late_bound_defaults_have_their_java_names() {
        starlark::values::Heap::temp(|heap| {
            let v = heap.alloc(StarlarkComputedDefaultValue);
            assert_eq!(v.get_type(), "StarlarkComputedDefault");
            assert_eq!(
                v.to_repr(),
                "<unknown object com.google.devtools.build.lib.packages.Attribute$StarlarkComputedDefault>"
            );
            assert_eq!(
                heap.alloc(LabelLateBoundDefaultValue).get_type(),
                "LabelLateBoundDefault"
            );
            assert_eq!(
                heap.alloc(LabelListLateBoundDefaultValue).get_type(),
                "LabelListLateBoundDefault"
            );
            assert_eq!(
                heap.alloc(AlwaysNullLateBoundDefaultValue).to_repr(),
                "<unknown object com.google.devtools.build.lib.packages.Attribute$AlwaysNullLateBoundDefault>"
            );
        });
    }
}
