//! The Starlark-independent bodies of the `cc_internal` methods
//! (`com.google.devtools.build.lib.rules.cpp.CcStarlarkInternal`) that the toolchain and variable
//! machinery needs. The Starlark wrapper crate does the argument casting (`Dict.cast`,
//! `Sequence.cast`, `CcToolchainProvider.wrap`) and calls these.

use std::sync::Arc;

use blaze_vfs::PathFragment;
use indexmap::IndexMap;

use crate::actions::{ParamFileInfo, ParameterFileType};
use crate::artifact_category::ArtifactCategory;
use crate::cc_common::CoptsFilter;
use crate::cc_toolchain_config_info::CcToolchainConfigInfo;
use crate::cc_toolchain_features::CcToolchainFeatures;
use crate::cc_toolchain_variables::{
    CcToolchainVariables, CcToolchainVariablesRef, VariableObject,
};
use crate::errors::EvalException;
use crate::feature_configuration_for_starlark::FeatureConfigurationForStarlark;
use crate::link_command_line::{LinkCommandLine, LinkCommandLineBuilder};
use crate::starlark_info::StarlarkInfo;

/// `cc_internal.cc_toolchain_variables(vars = ...)`: wraps a dictionary of build variables.
// Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:140 CcStarlarkInternal.getCcToolchainVariables
pub fn cc_toolchain_variables(
    build_variables: &IndexMap<String, VariableObject>,
) -> CcToolchainVariablesRef {
    CcToolchainVariables::new_map_variables(None, build_variables)
}

/// `cc_internal.combine_cc_toolchain_variables(parent, *variables)`.
// Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:159 CcStarlarkInternal.combineCcToolchainVariables
pub fn combine_cc_toolchain_variables(
    parent: &CcToolchainVariablesRef,
    variables_sequence: &[CcToolchainVariablesRef],
) -> CcToolchainVariablesRef {
    let mut builder = CcToolchainVariables::builder_with_parent(Some(parent.clone()));
    for variables in variables_sequence {
        builder.add_all_non_transitive(variables);
    }
    builder.build()
}

/// `cc_internal.intern_string_sequence_variable_value(string_sequence)`.
// Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:176 CcStarlarkInternal.internStringSequenceVariableValue
// PORT: interning is a memory optimization; the value is returned as an immutable list.
pub fn intern_string_sequence_variable_value(string_sequence: &[String]) -> VariableObject {
    VariableObject::string_list(string_sequence)
}

/// `cc_internal.cc_toolchain_features(toolchain_config_info, tools_directory)`.
// Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:263 CcStarlarkInternal.ccToolchainFeatures
pub fn cc_toolchain_features(
    cc_toolchain_config_info: Arc<StarlarkInfo>,
    tools_directory_path_string: &str,
) -> Result<CcToolchainFeatures, EvalException> {
    CcToolchainFeatures::new(
        &CcToolchainConfigInfo::new(cc_toolchain_config_info),
        PathFragment::create(tools_directory_path_string),
    )
}

/// Java: `ArtifactCategory.valueOf(category)`; unknown names throw `IllegalArgumentException`.
fn artifact_category_value_of(category: &str) -> ArtifactCategory {
    ArtifactCategory::value_of(category).unwrap_or_else(|| {
        panic!(
            "IllegalArgumentException: No enum constant \
             com.google.devtools.build.lib.rules.cpp.ArtifactCategory.{category}"
        )
    })
}

/// `cc_internal.get_artifact_name_for_category(cc_toolchain, category, output_name)`, given the
/// toolchain's features (`CcToolchainProvider.getFeatures()`).
// Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:353 CcStarlarkInternal.getArtifactNameForCategory
pub fn get_artifact_name_for_category(
    features: &CcToolchainFeatures,
    category: &str,
    output_name: &str,
) -> String {
    features.get_artifact_name_for_category(artifact_category_value_of(category), output_name)
}

/// `cc_internal.get_artifact_name_extension_for_category(cc_toolchain, category)`.
// Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:368 CcStarlarkInternal.getArtifactNameExtensionForCategory
pub fn get_artifact_name_extension_for_category(
    features: &CcToolchainFeatures,
    category: &str,
) -> String {
    features.get_artifact_name_extension_for_category(artifact_category_value_of(category))
}

/// `cc_internal.intern_seq(seq)`.
// Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:403 CcStarlarkInternal.internList
// PORT: interning is a memory optimization; the elements are returned unchanged (as a tuple, which
// the Starlark wrapper constructs).
pub fn intern_seq<T: Clone>(seq: &[T]) -> Vec<T> {
    seq.to_vec()
}

/// `cc_internal.get_link_args(action_name, feature_configuration, build_variables,
/// parameter_file_type)`: the [`LinkCommandLine`] and its param file info, which the Starlark
/// wrapper turns into `Args.forRegisteredAction(...)`.
// Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:420 CcStarlarkInternal.getArgs
pub fn get_link_args(
    action_name: &str,
    feature_configuration: &FeatureConfigurationForStarlark,
    build_variables: CcToolchainVariablesRef,
    param_file_type: Option<&str>,
) -> Result<(LinkCommandLine, Option<ParamFileInfo>), EvalException> {
    let mut link_command_line_builder = LinkCommandLineBuilder::new();
    link_command_line_builder
        .set_action_name(action_name)
        .set_build_variables(build_variables)
        .set_feature_configuration(feature_configuration.get_feature_configuration().clone());
    if let Some(param_file_type) = param_file_type {
        let parsed = ParameterFileType::value_of(param_file_type).unwrap_or_else(|| {
            panic!(
                "IllegalArgumentException: No enum constant \
                 com.google.devtools.build.lib.actions.ParameterFile.ParameterFileType.{param_file_type}"
            )
        });
        link_command_line_builder
            .set_parameter_file_type(parsed)
            .set_split_command_line(true);
    }
    let link_command_line = link_command_line_builder.build();
    let param_file_info = link_command_line.get_param_file_info()?;
    Ok((link_command_line, param_file_info))
}

/// The copts filter of `create_cc_compile_action` / `create_cc_compile_action_template`.
// Java: com/google/devtools/build/lib/rules/cpp/CcStarlarkInternal.java:759 CcStarlarkInternal.createCoptsFilter
pub fn create_copts_filter(
    copts_filter_string: Option<&str>,
) -> Result<CoptsFilter, EvalException> {
    match copts_filter_string {
        None | Some("") => Ok(CoptsFilter::always_passes()),
        Some(s) => CoptsFilter::from_regex(s)
            .map_err(|e| EvalException::new(format!("invalid regular expression '{s}': {e}"))),
    }
}

// `cc_internal.expand_and_tokenize` (CcStarlarkInternal.expandAndTokenize) is ported in
// blaze_rules `cpp::cc_starlark_internal`, which has the rule context's expander.
