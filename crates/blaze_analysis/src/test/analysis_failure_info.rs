//! Ports of `analysis/test/AnalysisFailure.java` and `analysis/test/AnalysisFailureInfo.java`
//! (with their Starlark APIs, `starlarkbuildapi/test/AnalysisFailureApi.java` and
//! `AnalysisFailureInfoApi.java`): the analysis errors a target would have caused, propagated
//! as a provider under `--allow_analysis_failures` (analysis-failure testing).

use std::fmt;

use allocative::Allocative;
use blaze_cmdline::Label;
use blaze_collect::{NestedSet, NestedSetBuilder};
use blaze_packages::starlark_defs::builtin_providers::ANALYSIS_FAILURE_INFO;
use blaze_starlark::label::LabelValue;
use blaze_starlark::structs::DeeplyImmutable;
use starlark::any::ProvidesStaticType;
use starlark::starlark_simple_value;
use starlark::values::{Demand, Heap, NoSerialize, StarlarkValue, Value, starlark_value};

use crate::info::InfoProviderKey;
use crate::rule::depsets::{depset_value, nested_set_to_depset};

/// Encapsulates information about an analysis-phase error which would have occurred during a
/// build.
// Java: com/google/devtools/build/lib/analysis/test/AnalysisFailure.java:25 AnalysisFailure
#[derive(Clone, Debug, PartialEq, Eq, Hash, ProvidesStaticType, NoSerialize, Allocative)]
pub struct AnalysisFailure {
    #[allocative(skip)]
    label: Label,
    message: String,
}

starlark_simple_value!(AnalysisFailure);

impl AnalysisFailure {
    // Java: com/google/devtools/build/lib/analysis/test/AnalysisFailure.java:26 AnalysisFailure.create
    pub fn create(label: Label, message: String) -> AnalysisFailure {
        AnalysisFailure { label, message }
    }

    // Java: com/google/devtools/build/lib/starlarkbuildapi/test/AnalysisFailureApi.java:44 AnalysisFailureApi.getLabel
    pub fn get_label(&self) -> &Label {
        &self.label
    }

    // Java: com/google/devtools/build/lib/starlarkbuildapi/test/AnalysisFailureApi.java:51 AnalysisFailureApi.getMessage
    pub fn get_message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for AnalysisFailure {
    // Java: com/google/devtools/build/lib/analysis/test/AnalysisFailure.java:31 AnalysisFailure.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<AnalyisFailure object>")
    }
}

const FAILURE_FIELDS: [&str; 2] = ["label", "message"];

// Java: com/google/devtools/build/lib/starlarkbuildapi/test/AnalysisFailureApi.java:35 AnalysisFailureApi
#[starlark_value(type = "AnalysisFailure")]
impl<'v> StarlarkValue<'v> for AnalysisFailure {
    fn get_attr(&self, attribute: &str, heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            "label" => Some(heap.alloc(LabelValue(self.label.clone()))),
            "message" => Some(heap.alloc(self.message.as_str())),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        FAILURE_FIELDS.contains(&attribute)
    }

    fn dir_attr(&self) -> Vec<String> {
        FAILURE_FIELDS.iter().map(|s| (*s).to_owned()).collect()
    }

    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(DeeplyImmutable);
    }
}

/// Implementation for the `AnalysisFailureInfo` provider.
// Java: com/google/devtools/build/lib/analysis/test/AnalysisFailureInfo.java:28 AnalysisFailureInfo
#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct AnalysisFailureInfo {
    #[allocative(skip)]
    causes: NestedSet<AnalysisFailure>,
}

starlark_simple_value!(AnalysisFailureInfo);

impl AnalysisFailureInfo {
    /// Constructs an `AnalysisFailureInfo` object representing the given failures.
    // Java: com/google/devtools/build/lib/analysis/test/AnalysisFailureInfo.java:48 AnalysisFailureInfo.forAnalysisFailures
    pub fn for_analysis_failures(
        failures: impl IntoIterator<Item = AnalysisFailure>,
    ) -> AnalysisFailureInfo {
        let mut builder = NestedSetBuilder::stable_order();
        builder.add_all(failures);
        AnalysisFailureInfo {
            causes: builder.build(),
        }
    }

    /// Constructs an `AnalysisFailureInfo` representing the given sets of failures, without
    /// flattening them.
    // Java: com/google/devtools/build/lib/analysis/test/AnalysisFailureInfo.java:58 AnalysisFailureInfo.forAnalysisFailureSets
    pub fn for_analysis_failure_sets(
        failures: impl IntoIterator<Item = NestedSet<AnalysisFailure>>,
    ) -> AnalysisFailureInfo {
        let mut full_set_builder = NestedSetBuilder::stable_order();
        for failure in failures {
            full_set_builder.add_transitive(&failure);
        }
        AnalysisFailureInfo {
            causes: full_set_builder.build(),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/test/AnalysisFailureInfo.java:73 AnalysisFailureInfo.getCausesNestedSet
    pub fn get_causes_nested_set(&self) -> &NestedSet<AnalysisFailure> {
        &self.causes
    }
}

impl fmt::Display for AnalysisFailureInfo {
    // Java: NativeInfo/StructImpl.repr with no struct fields.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "AnalysisFailureInfo()")
    }
}

const INFO_FIELDS: [&str; 1] = ["causes"];

// Java: com/google/devtools/build/lib/starlarkbuildapi/test/AnalysisFailureInfoApi.java:46 AnalysisFailureInfoApi
#[starlark_value(type = "AnalysisFailureInfo")]
impl<'v> StarlarkValue<'v> for AnalysisFailureInfo {
    fn get_attr(&self, attribute: &str, _heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            // Java: com/google/devtools/build/lib/analysis/test/AnalysisFailureInfo.java:69 AnalysisFailureInfo.getCauses
            "causes" => Some(depset_value(&nested_set_to_depset(
                &self.causes,
                "AnalysisFailure",
                &|failure, heap| heap.alloc(failure.clone()),
            ))),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        INFO_FIELDS.contains(&attribute)
    }

    fn dir_attr(&self) -> Vec<String> {
        INFO_FIELDS.iter().map(|s| (*s).to_owned()).collect()
    }

    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(InfoProviderKey(ANALYSIS_FAILURE_INFO.key()));
        demand.provide_value(DeeplyImmutable);
    }
}
