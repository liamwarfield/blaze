//! Port of `analysis/config/StarlarkDefinedConfigTransition.java` (the loading-phase parts:
//! construction, setting canonicalization, identity, repr). `evaluate` belongs to analysis (W4).

use std::collections::BTreeMap;
use std::fmt;
use std::hash::Hash;
use std::sync::Arc;

use allocative::Allocative;
use blaze_cmdline::Label;
use blaze_cmdline::LabelPackageContext;
use blaze_cmdline::RepositoryMapping;
use blaze_cmdline::label_constants::COMMAND_LINE_OPTION_PACKAGE_IDENTIFIER;
use blaze_cmdline::label_constants::COMMAND_LINE_OPTION_PREFIX;
use blaze_starlark::Location;
use starlark::any::ProvidesStaticType;
use starlark::coerce::Coerce;
use starlark::collections::StarlarkHasher;
use starlark::starlark_complex_value;
use starlark::values::Freeze;
use starlark::values::FreezeResult;
use starlark::values::Freezer;
use starlark::values::FrozenValue;
use starlark::values::NoSerialize;
use starlark::values::StarlarkValue;
use starlark::values::Trace;
use starlark::values::Value;
use starlark::values::ValueLifetimeless;
use starlark::values::ValueLike;
use starlark::values::starlark_value;

/// Java `StarlarkDefinedConfigTransition.Settings`.
// Java: com/google/devtools/build/lib/analysis/config/StarlarkDefinedConfigTransition.java:81 StarlarkDefinedConfigTransition.Settings
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Settings {
    /// Build settings that are read by a transition.
    Inputs,
    /// Build settings that are written by a transition.
    Outputs,
    /// Build settings that are read and/or written by a transition.
    InputsAndOutputs,
}

impl fmt::Display for Settings {
    // Java enum toString: the constant name.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Settings::Inputs => "INPUTS",
            Settings::Outputs => "OUTPUTS",
            Settings::InputsAndOutputs => "INPUTS_AND_OUTPUTS",
        })
    }
}

/// Which subclass of `StarlarkDefinedConfigTransition` this is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Allocative)]
pub enum TransitionKind {
    /// Java `RegularTransition` (`transition()`).
    Regular,
    /// Java `ExecTransition` (`exec_transition()`, builtins only).
    Exec,
    /// Java `AnalysisTestTransition` (`analysis_test_transition()`).
    AnalysisTest,
}

/// The immutable, non-Starlark part of a transition.
#[derive(Debug, PartialEq, Eq)]
pub struct TransitionData {
    pub kind: TransitionKind,
    /// Canonicalized setting -> setting as given (Java `ImmutableSortedMap`, by label).
    pub inputs_canonicalized_to_given: BTreeMap<Label, String>,
    pub outputs_canonicalized_to_given: BTreeMap<Label, String>,
    pub parent_label: Label,
    pub location: Location,
    pub package_context: LabelPackageContext,
}

/// Errors of transition construction (Java `EvalException` from the constructor).
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct TransitionError(pub String);

// Java: com/google/devtools/build/lib/analysis/config/StarlarkDefinedConfigTransition.java:133 StarlarkDefinedConfigTransition.canonicalizeSetting
fn canonicalize_setting(
    setting: &str,
    repo_mapping: &RepositoryMapping,
    parent_label: &Label,
) -> Result<Label, blaze_cmdline::LabelSyntaxException> {
    // native options
    if setting.starts_with(COMMAND_LINE_OPTION_PREFIX) {
        return Label::parse_canonical(setting);
    }
    Label::parse_with_package_context(
        setting,
        &LabelPackageContext::of(parent_label.get_package_identifier(), repo_mapping.clone()),
    )
}

// Java: com/google/devtools/build/lib/analysis/config/StarlarkDefinedConfigTransition.java:153 StarlarkDefinedConfigTransition.getCanonicalizedSettings
fn get_canonicalized_settings(
    repo_mapping: &RepositoryMapping,
    parent_label: &Label,
    settings: &[String],
    disallowed_options: &[String],
    inputs_or_outputs: Settings,
) -> Result<BTreeMap<Label, String>, TransitionError> {
    // PORT: Java fills a HashMap and copies it to an ImmutableSortedMap; only the sorted map is
    // observable.
    let mut canonicalized_to_given: BTreeMap<Label, String> = BTreeMap::new();
    for setting in settings {
        let canonicalized_setting = canonicalize_setting(setting, repo_mapping, parent_label)
            .map_err(|_| {
                TransitionError(format!(
                    "Malformed label in transition {inputs_or_outputs} parameter: '{setting}'"
                ))
            })?;
        if *canonicalized_setting.get_package_identifier()
            == *COMMAND_LINE_OPTION_PACKAGE_IDENTIFIER
        {
            let option_name = canonicalized_setting.get_name();
            if disallowed_options.iter().any(|o| o == option_name) {
                return Err(TransitionError(format!(
                    "Option '{option_name}' is not allowed in transitions {inputs_or_outputs}."
                )));
            }
        }
        if let Some(previous_setting) =
            canonicalized_to_given.insert(canonicalized_setting.clone(), setting.clone())
        {
            return Err(TransitionError(format!(
                "Transition declares duplicate build setting '{canonicalized_setting}' in \
                 {inputs_or_outputs} (specified as '{setting}' and '{previous_setting}')"
            )));
        }
    }
    Ok(canonicalized_to_given)
}

impl TransitionData {
    // Java: com/google/devtools/build/lib/analysis/config/StarlarkDefinedConfigTransition.java:101 StarlarkDefinedConfigTransition.<init>
    fn new(
        kind: TransitionKind,
        inputs: &[String],
        outputs: &[String],
        repo_mapping: &RepositoryMapping,
        parent_label: Label,
        location: Location,
        disallowed_options: &[String],
    ) -> Result<TransitionData, TransitionError> {
        let package_context =
            LabelPackageContext::of(parent_label.get_package_identifier(), repo_mapping.clone());
        let outputs_canonicalized_to_given = get_canonicalized_settings(
            repo_mapping,
            &parent_label,
            outputs,
            disallowed_options,
            Settings::Outputs,
        )?;
        let inputs_canonicalized_to_given = get_canonicalized_settings(
            repo_mapping,
            &parent_label,
            inputs,
            disallowed_options,
            Settings::Inputs,
        )?;
        Ok(TransitionData {
            kind,
            inputs_canonicalized_to_given,
            outputs_canonicalized_to_given,
            parent_label,
            location,
            package_context,
        })
    }

    /// Java `getInputs()`: the given forms, in canonical order.
    // Java: com/google/devtools/build/lib/analysis/config/StarlarkDefinedConfigTransition.java:198 StarlarkDefinedConfigTransition.getInputs
    pub fn get_inputs(&self) -> Vec<String> {
        self.inputs_canonicalized_to_given
            .values()
            .cloned()
            .collect()
    }

    // Java: com/google/devtools/build/lib/analysis/config/StarlarkDefinedConfigTransition.java:210 StarlarkDefinedConfigTransition.getOutputs
    pub fn get_outputs(&self) -> Vec<String> {
        self.outputs_canonicalized_to_given
            .values()
            .cloned()
            .collect()
    }
}

/// A configuration transition defined in Starlark (Java `StarlarkDefinedConfigTransition`), the
/// value of `transition()`, `exec_transition()` and `analysis_test_transition()`.
// Java: com/google/devtools/build/lib/analysis/config/StarlarkDefinedConfigTransition.java:75 StarlarkDefinedConfigTransition
#[derive(Debug, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[repr(C)]
pub struct StarlarkDefinedConfigTransitionGen<V: ValueLifetimeless> {
    /// Java `RegularTransition.impl` (None for analysis-test transitions).
    impl_: Option<V>,
    /// Java `AnalysisTestTransition.changedSettings` (given setting -> value).
    changed_settings: Vec<(String, V)>,
    #[trace(unsafe_ignore)]
    #[allocative(skip)]
    data: Arc<TransitionData>,
}

starlark_complex_value!(pub StarlarkDefinedConfigTransition);

// SAFETY: `repr(C)` and FrozenValue/Value have the same layout.
unsafe impl<'v> Coerce<StarlarkDefinedConfigTransitionGen<Value<'v>>>
    for StarlarkDefinedConfigTransitionGen<FrozenValue>
{
}

impl<'v> Freeze for StarlarkDefinedConfigTransition<'v> {
    type Frozen = FrozenStarlarkDefinedConfigTransition;
    fn freeze(self, freezer: &Freezer) -> FreezeResult<FrozenStarlarkDefinedConfigTransition> {
        Ok(StarlarkDefinedConfigTransitionGen {
            impl_: match self.impl_ {
                Some(v) => Some(v.freeze(freezer)?),
                None => None,
            },
            changed_settings: self
                .changed_settings
                .into_iter()
                .map(|(k, v)| Ok((k, v.freeze(freezer)?)))
                .collect::<FreezeResult<Vec<_>>>()?,
            data: self.data,
        })
    }
}

impl<'v> StarlarkDefinedConfigTransition<'v> {
    // Java: com/google/devtools/build/lib/analysis/config/StarlarkDefinedConfigTransition.java:272 StarlarkDefinedConfigTransition.newRegularTransition
    // Java: com/google/devtools/build/lib/analysis/config/StarlarkDefinedConfigTransition.java:385 RegularTransition.<init>
    pub fn new_regular_transition(
        impl_: Value<'v>,
        inputs: &[String],
        outputs: &[String],
        parent_label: Label,
        location: Location,
        repo_mapping: &RepositoryMapping,
        disallowed_options: &[String],
    ) -> Result<Self, TransitionError> {
        Ok(StarlarkDefinedConfigTransitionGen {
            impl_: Some(impl_),
            changed_settings: Vec::new(),
            data: Arc::new(TransitionData::new(
                TransitionKind::Regular,
                inputs,
                outputs,
                repo_mapping,
                parent_label,
                location,
                disallowed_options,
            )?),
        })
    }

    // Java: com/google/devtools/build/lib/analysis/config/StarlarkDefinedConfigTransition.java:286 StarlarkDefinedConfigTransition.newExecTransition
    // Java: com/google/devtools/build/lib/analysis/config/StarlarkDefinedConfigTransition.java:758 ExecTransition.<init>
    pub fn new_exec_transition(
        impl_: Value<'v>,
        inputs: &[String],
        outputs: &[String],
        parent_label: Label,
        location: Location,
        repo_mapping: &RepositoryMapping,
        disallowed_options: &[String],
    ) -> Result<Self, TransitionError> {
        Ok(StarlarkDefinedConfigTransitionGen {
            impl_: Some(impl_),
            changed_settings: Vec::new(),
            data: Arc::new(TransitionData::new(
                TransitionKind::Exec,
                inputs,
                outputs,
                repo_mapping,
                parent_label,
                location,
                disallowed_options,
            )?),
        })
    }

    // Java: com/google/devtools/build/lib/analysis/config/StarlarkDefinedConfigTransition.java:300 StarlarkDefinedConfigTransition.newAnalysisTestTransition
    // Java: com/google/devtools/build/lib/analysis/config/StarlarkDefinedConfigTransition.java:315 AnalysisTestTransition.<init>
    pub fn new_analysis_test_transition(
        changed_settings: Vec<(String, Value<'v>)>,
        repo_mapping: &RepositoryMapping,
        parent_label: Label,
        location: Location,
        disallowed_options: &[String],
    ) -> Result<Self, TransitionError> {
        let outputs: Vec<String> = changed_settings.iter().map(|(k, _)| k.clone()).collect();
        Ok(StarlarkDefinedConfigTransitionGen {
            impl_: None,
            changed_settings,
            data: Arc::new(TransitionData::new(
                TransitionKind::AnalysisTest,
                &[],
                &outputs,
                repo_mapping,
                parent_label,
                location,
                disallowed_options,
            )?),
        })
    }
}

impl<'v, V: ValueLike<'v>> StarlarkDefinedConfigTransitionGen<V> {
    pub fn data(&self) -> &Arc<TransitionData> {
        &self.data
    }

    /// Java `isForAnalysisTesting()`.
    // Java: com/google/devtools/build/lib/analysis/config/StarlarkDefinedConfigTransition.java:192 StarlarkDefinedConfigTransition.isForAnalysisTesting
    pub fn is_for_analysis_testing(&self) -> bool {
        self.data.kind == TransitionKind::AnalysisTest
    }

    /// Java `isExecTransition()`.
    // Java: com/google/devtools/build/lib/analysis/config/StarlarkDefinedConfigTransition.java:126 StarlarkDefinedConfigTransition.isExecTransition
    pub fn is_exec_transition(&self) -> bool {
        self.data.kind == TransitionKind::Exec
    }

    /// Java `RegularTransition.impl`.
    pub fn get_impl(&self) -> Option<Value<'v>> {
        self.impl_.map(|v| v.to_value())
    }

    /// Java `AnalysisTestTransition.changedSettings`.
    pub fn changed_settings(&self) -> impl Iterator<Item = (&str, Value<'v>)> {
        self.changed_settings
            .iter()
            .map(|(k, v)| (k.as_str(), v.to_value()))
    }

    /// Java `getLocation()`.
    pub fn get_location(&self) -> &Location {
        &self.data.location
    }
}

impl<'v, V: ValueLike<'v>> fmt::Display for StarlarkDefinedConfigTransitionGen<V> {
    // Java: com/google/devtools/build/lib/analysis/config/StarlarkDefinedConfigTransition.java:355 AnalysisTestTransition.repr
    // Java: com/google/devtools/build/lib/analysis/config/StarlarkDefinedConfigTransition.java:733 RegularTransition.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.data.kind {
            TransitionKind::AnalysisTest => f.write_str("<analysis_test_transition object>"),
            TransitionKind::Regular | TransitionKind::Exec => f.write_str("<transition object>"),
        }
    }
}

#[starlark_value(type = "transition")]
impl<'v, V: ValueLike<'v>> StarlarkValue<'v> for StarlarkDefinedConfigTransitionGen<V>
where
    Self: ProvidesStaticType<'v>,
{
    // Java: com/google/devtools/build/lib/analysis/config/StarlarkDefinedConfigTransition.java:738 RegularTransition.equals
    // Java: com/google/devtools/build/lib/analysis/config/StarlarkDefinedConfigTransition.java:360 AnalysisTestTransition.equals
    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        let Some(other) = StarlarkDefinedConfigTransition::from_value(other) else {
            return Ok(false);
        };
        let (a, b) = (&self.data, &other.data);
        if (a.kind == TransitionKind::AnalysisTest) != (b.kind == TransitionKind::AnalysisTest) {
            return Ok(false);
        }
        if a.get_inputs() != b.get_inputs() || a.get_outputs() != b.get_outputs() {
            return Ok(false);
        }
        match (self.impl_, other.get_impl()) {
            (Some(x), Some(y)) => Ok(x.to_value().ptr_eq(y)),
            (None, None) => {
                if self.changed_settings.len() != other.changed_settings.len() {
                    return Ok(false);
                }
                for ((ka, va), (kb, vb)) in
                    self.changed_settings.iter().zip(other.changed_settings())
                {
                    if ka != kb || !va.to_value().equals(vb)? {
                        return Ok(false);
                    }
                }
                Ok(true)
            }
            _ => Ok(false),
        }
    }

    /// Hashes what `equals` compares; Java's cached `hashCode` is only consistent with `equals`
    /// too (a hash code value is never printed).
    // Java: com/google/devtools/build/lib/analysis/config/StarlarkDefinedConfigTransition.java:373 AnalysisTestTransition.hashCode
    // Java: com/google/devtools/build/lib/analysis/config/StarlarkDefinedConfigTransition.java:751 RegularTransition.hashCode
    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        self.data.get_inputs().hash(hasher);
        self.data.get_outputs().hash(hasher);
        Ok(())
    }
}
