//! The Skyframe parts of `BazelDepGraphFunction` (BAZEL_DEP_GRAPH) and
//! `BazelModuleInspectorFunction` (BAZEL_MODULE_INSPECTION).

use blaze_bzlmod_model::{
    BazelDepGraphFunction, BazelDepGraphValue, ExternalDepsException, ModuleExtensionId, ModuleKey,
};
use blaze_events::Event;
use blaze_skyframe::{ComputeError, Environment, SkyKey, Transience};
use blaze_util::profiler::{Profiler, ProfilerTask};
use indexmap::{IndexMap, IndexSet};

use super::keys::{
    BazelDepGraphKey, BazelModuleResolutionKey, ModuleFileKey, SingleExtensionKey,
    SingleExtensionValue,
};
use super::throw;
use crate::inspector::{BazelModuleInspectorFunction as InspectorLogic, BazelModuleInspectorValue};

// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelDepGraphFunction.java:60 BazelDepGraphFunction.compute
pub fn compute_dep_graph(
    env: &mut Environment<'_>,
) -> Result<Option<BazelDepGraphValue>, ComputeError> {
    let selection_result = env.get_value(BazelModuleResolutionKey)?;
    if env.values_missing() {
        return Ok(None);
    }
    let selection_result = selection_result.expect("values are not missing");
    let dep_graph = &selection_result.resolved_dep_graph;
    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelDepGraphFunction.java:69 BazelDepGraphFunction.compute
    let _c = Profiler::instance().profile(ProfilerTask::BZLMOD, "finalize dep graph");
    BazelDepGraphFunction::compute(dep_graph)
        .map(Some)
        .map_err(|e| throw(e, Transience::Persistent))
}

/// `ExtensionRepos`: the repos each extension generates, and the errors of failed extensions.
// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleInspectorFunction.java:178 BazelModuleInspectorFunction.ExtensionRepos
struct ExtensionRepos {
    extension_to_repo_internal_names: IndexMap<ModuleExtensionId, IndexSet<String>>,
    errors: Vec<ExternalDepsException>,
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleInspectorFunction.java:48 BazelModuleInspectorFunction.compute
pub fn compute_inspector(
    env: &mut Environment<'_>,
) -> Result<Option<BazelModuleInspectorValue>, ComputeError> {
    let Some(root) = env.get_value(ModuleFileKey::key_for_root_module())? else {
        return Ok(None);
    };
    let Some(dep_graph_value) = env.get_value(BazelDepGraphKey)? else {
        return Ok(None);
    };
    let Some(resolution_value) = env.get_value(BazelModuleResolutionKey)? else {
        return Ok(None);
    };
    let overrides = &root.as_root().overrides;
    let unpruned_dep_graph = &resolution_value.unpruned_dep_graph;
    let resolved_dep_graph = &resolution_value.resolved_dep_graph;

    let used_modules: IndexSet<ModuleKey> = resolved_dep_graph.keys().cloned().collect();
    let dep_graph =
        InspectorLogic::compute_augmented_graph(unpruned_dep_graph, &used_modules, overrides);

    let Some(extension_repos) = comput_extension_repos(&dep_graph_value, env)? else {
        return Ok(None);
    };

    // Group all ModuleKeys seen by their module name for easy lookup
    let modules_index = InspectorLogic::compute_modules_index(&dep_graph);

    Ok(Some(BazelModuleInspectorValue::create(
        dep_graph,
        modules_index,
        extension_repos.extension_to_repo_internal_names,
        dep_graph_value.module_key_to_repository_names(),
        extension_repos.errors,
    )))
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleInspectorFunction.java:183 BazelModuleInspectorFunction.computExtensionRepos
fn comput_extension_repos(
    dep_graph_value: &BazelDepGraphValue,
    env: &mut Environment<'_>,
) -> Result<Option<ExtensionRepos>, ComputeError> {
    let single_extension_keys: Vec<SingleExtensionKey> = dep_graph_value
        .extension_usages_table
        .row_key_set()
        .cloned()
        .map(SingleExtensionValue::key)
        .collect();
    let sky_keys: Vec<SkyKey> = single_extension_keys
        .iter()
        .cloned()
        .map(SkyKey::new)
        .collect();
    let single_extension_values = env.get_values_and_exceptions(&sky_keys)?;

    // ImmutableSetMultimap.Builder
    let mut extension_to_repo_internal_names: IndexMap<ModuleExtensionId, IndexSet<String>> =
        IndexMap::new();
    let mut errors: Vec<ExternalDepsException> = Vec::new();
    for single_extension_key in &single_extension_keys {
        let single_extension_value = match single_extension_values
            .get_or_throw_typed::<SingleExtensionKey, ExternalDepsException>(single_extension_key)
        {
            Ok(v) => v,
            Err(e) => {
                // The extension failed, so we can't report its generated repos. We can still
                // report the imported repos in keep going mode, so don't fail and just skip this
                // extension.
                env.listener().handle(&Event::error(e.get_message()));
                errors.push(e);
                continue;
            }
        };
        let Some(single_extension_value) = single_extension_value else {
            return Ok(None);
        };
        // A multimap has no entry for a key without values.
        if !single_extension_value.generated_repo_specs.is_empty() {
            extension_to_repo_internal_names
                .entry(single_extension_key.argument().clone())
                .or_default()
                .extend(single_extension_value.generated_repo_specs.keys().cloned());
        }
    }
    Ok(Some(ExtensionRepos {
        extension_to_repo_internal_names,
        errors,
    }))
}
