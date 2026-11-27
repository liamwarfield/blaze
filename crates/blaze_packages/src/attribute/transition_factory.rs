//! Placeholder data for configuration transitions on attributes and rules (Java
//! `analysis/config/transitions/TransitionFactory<AttributeTransitionData>` /
//! `TransitionFactory<RuleTransitionData>`), and a port of `packages/AttributeTransitionData`.
//!
//! The loading phase only needs to know whether there is a transition, whether it is a tool
//! (exec) or split transition, and which Starlark transition object it is; creating transitions
//! belongs to analysis (W4).

use blaze_cmdline::Label;

use crate::attribute::AttributeMap;
use crate::rule_class::StarlarkCallable;

/// Java `TransitionFactory.TransitionType`.
// Java: com/google/devtools/build/lib/analysis/config/transitions/TransitionFactory.java:42 TransitionFactory.TransitionType
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransitionType {
    Any,
    Rule,
    Attribute,
}

/// A transition factory (Java `TransitionFactory<?>` implementations relevant to loading).
// Java: com/google/devtools/build/lib/analysis/config/transitions/TransitionFactory.java:32 TransitionFactory
#[derive(Debug, Clone, PartialEq)]
pub enum TransitionFactory {
    /// Java `NoTransition.getFactory()`.
    // Java: com/google/devtools/build/lib/analysis/config/transitions/NoTransition.java:37 NoTransition.getFactory
    // Java: com/google/devtools/build/lib/analysis/config/transitions/NoTransition.java:69 NoTransition.Factory.transitionType
    NoTransition,
    /// Java `ExecutionTransitionFactory.createFactory(execGroup)` (`cfg = "exec"`).
    Exec { exec_group: Option<String> },
    /// Java `TargetTransition` (`cfg = "target"`), identical to no transition for loading.
    Target,
    /// A Starlark-defined transition (Java `StarlarkAttributeTransitionProvider` /
    /// `StarlarkRuleTransitionProvider`) over the frozen `transition()` object.
    Starlark {
        transition: StarlarkCallable,
        is_split: bool,
    },
    /// Java `ComposingTransitionFactory`: `first`, then `second`.
    // Java: com/google/devtools/build/lib/analysis/config/transitions/ComposingTransitionFactory.java:118 ComposingTransitionFactory.transitionFactory1
    // Java: com/google/devtools/build/lib/analysis/config/transitions/ComposingTransitionFactory.java:120 ComposingTransitionFactory.transitionFactory2
    // PORT: skipped, the factories' transition types are not modelled (loading composes rule transitions with rule transitions only; see compose): ComposingTransitionFactory.transitionType
    // Java: com/google/devtools/build/lib/analysis/config/transitions/ComposingTransitionFactory.java:103 ComposingTransitionFactory.create
    // Java: com/google/devtools/build/lib/analysis/config/transitions/ComposingTransitionFactory.java:103 ComposingTransitionFactory.create
    Composing {
        first: Box<TransitionFactory>,
        second: Box<TransitionFactory>,
    },
    /// Java `ConfigFeatureFlagTransitionFactory(attributeName)`
    /// (`config_common.config_feature_flag_transition(attribute)`): a rule transition that sets
    /// exactly the feature flag values of the rule's `attribute_name` attribute.
    // Java: com/google/devtools/build/lib/rules/config/ConfigFeatureFlagTransitionFactory.java:45 ConfigFeatureFlagTransitionFactory
    // Java: com/google/devtools/build/lib/rules/config/ConfigFeatureFlagTransitionFactory.java:156 ConfigFeatureFlagTransitionFactory.getAttributeName
    // Java: com/google/devtools/build/lib/rules/config/ConfigFeatureFlagTransitionFactory.java:161 ConfigFeatureFlagTransitionFactory.equals
    // PORT: skipped, TransitionFactory is never hashed here and no output prints a factory at the pin: ConfigFeatureFlagTransitionFactory.hashCode, ConfigFeatureFlagTransitionFactory.toString
    ConfigFeatureFlag { attribute_name: String },
    /// A native transition factory, named by its Java class simple name.
    Native {
        name: String,
        is_tool: bool,
        is_split: bool,
    },
}

impl TransitionFactory {
    /// Java `NoConfigTransition.getFactory()` (`cfg = config.none()`): a tool transition
    /// (`Factory.isTool()` is true), not a split.
    // Java: com/google/devtools/build/lib/analysis/config/transitions/NoConfigTransition.java:97 NoConfigTransition.Factory.isTool
    // Java: com/google/devtools/build/lib/analysis/config/transitions/NoConfigTransition.java:77 NoConfigTransition.getFactory
    // Java: com/google/devtools/build/lib/analysis/config/transitions/NoConfigTransition.java:92 NoConfigTransition.Factory.transitionType
    pub fn no_config_transition() -> TransitionFactory {
        TransitionFactory::Native {
            name: "NoConfigTransition".to_owned(),
            is_tool: true,
            is_split: false,
        }
    }

    /// Java `factory instanceof NoConfigTransition.Factory`.
    // Java: com/google/devtools/build/lib/analysis/config/transitions/NoConfigTransition.java:59 NoConfigTransition.isInstance
    pub fn is_no_config_transition(&self) -> bool {
        matches!(self, TransitionFactory::Native { name, .. } if name == "NoConfigTransition")
    }

    /// Java `ComposingTransitionFactory.of`: `factory1` then `factory2`, dropping a no-op side;
    /// `Err` with Java's `IncompatibleTransitionsException` message.
    // PORT: TransitionFactory.transitionType() compatibility is not modelled (loading only
    // composes rule transitions with rule transitions).
    // Java: com/google/devtools/build/lib/analysis/config/transitions/ComposingTransitionFactory.java:50 ComposingTransitionFactory.of
    pub fn compose(
        transition_factory1: TransitionFactory,
        transition_factory2: TransitionFactory,
    ) -> Result<TransitionFactory, String> {
        if transition_factory1.is_tool() && transition_factory2.is_tool() {
            // Java: com/google/devtools/build/lib/analysis/config/transitions/ComposingTransitionFactory.java:92 ComposingTransitionFactory.IncompatibleTransitionsException.<init>
            return Err("can't compose two exec transitions".to_owned());
        }
        if transition_factory1.is_no_transition() {
            // Since transitionFactory1 causes no changes, use transitionFactory2 directly.
            return Ok(transition_factory2);
        } else if transition_factory2.is_no_transition() {
            // Since transitionFactory2 causes no changes, use transitionFactory1 directly.
            return Ok(transition_factory1);
        }
        Ok(TransitionFactory::Composing {
            first: Box::new(transition_factory1),
            second: Box::new(transition_factory2),
        })
    }

    /// Java `ComposingTransitionFactory.ofUnchecked`: [`TransitionFactory::compose`], panicking
    /// (IllegalArgumentException) when incompatible.
    // Java: com/google/devtools/build/lib/analysis/config/transitions/ComposingTransitionFactory.java:81 ComposingTransitionFactory.ofUnchecked
    pub fn compose_unchecked(
        transition_factory1: TransitionFactory,
        transition_factory2: TransitionFactory,
    ) -> TransitionFactory {
        match TransitionFactory::compose(transition_factory1, transition_factory2) {
            Ok(f) => f,
            Err(e) => panic!(
                "IllegalArgumentException: com.google.devtools.build.lib.analysis.config.transitions.ComposingTransitionFactory$IncompatibleTransitionsException: {e}"
            ),
        }
    }

    /// Java `NoTransition.isInstance(factory)`.
    // Java: com/google/devtools/build/lib/analysis/config/transitions/NoTransition.java:47 NoTransition.isInstance
    pub fn is_no_transition(&self) -> bool {
        matches!(self, TransitionFactory::NoTransition)
    }

    // Java: com/google/devtools/build/lib/analysis/config/transitions/TransitionFactory.java:80 TransitionFactory.isTool
    pub fn is_tool(&self) -> bool {
        match self {
            TransitionFactory::Exec { .. } => true,
            // Java: com/google/devtools/build/lib/analysis/config/transitions/ComposingTransitionFactory.java:123 ComposingTransitionFactory.isTool
            TransitionFactory::Composing { first, second } => first.is_tool() || second.is_tool(),
            TransitionFactory::Native { is_tool, .. } => *is_tool,
            _ => false,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/config/transitions/TransitionFactory.java:85 TransitionFactory.isSplit
    pub fn is_split(&self) -> bool {
        match self {
            TransitionFactory::Starlark { is_split, .. } => *is_split,
            // Java: com/google/devtools/build/lib/analysis/config/transitions/ComposingTransitionFactory.java:128 ComposingTransitionFactory.isSplit
            TransitionFactory::Composing { first, second } => first.is_split() || second.is_split(),
            TransitionFactory::Native { is_split, .. } => *is_split,
            _ => false,
        }
    }
}

/// Data a transition factory uses to create an attribute transition (Java
/// `AttributeTransitionData`).
// Java: com/google/devtools/build/lib/packages/AttributeTransitionData.java:33 AttributeTransitionData
pub struct AttributeTransitionData<'a> {
    pub attributes: Option<&'a dyn AttributeMap>,
    pub execution_platform: Option<Label>,
    // PORT: `Object analysisData` belongs to analysis; omitted until W4.
}
