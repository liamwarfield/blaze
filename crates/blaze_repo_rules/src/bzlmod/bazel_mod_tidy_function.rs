//! BAZEL_MOD_TIDY: everything `bazel mod tidy` needs.
//!
//! Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModTidyFunction.java. The key and value
//! are in blaze_bzlmod (`skyframe::keys`); the function is here because it needs
//! `RepositoryUtils.getRootedPathFromLabel` and `RepositoryMappingValue`.

use blaze_bzlmod::skyframe::keys::{
    BazelDepGraphKey, BazelModTidyKey, BazelModTidyValue, ModuleFileKey, SingleExtensionEvalKey,
    SingleExtensionValue,
};
use blaze_bzlmod_model::{ExternalDepsException, ModuleKey};
use blaze_cmdline::Label;
use blaze_cmdline::label::LabelRepoContextImpl;
use blaze_cmdline::repository_name::BAZEL_TOOLS;
use blaze_events::Event;
use blaze_sky_loading::repos::RepositoryMappingKey;
use blaze_skyframe::{ComputeError, Environment, SkyKey};

use crate::repository::repository_utils::get_rooted_path_from_label;

/// Computes all information required for the `bazel mod tidy` command, which in particular
/// requires evaluating all module extensions used by the root module.
// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModTidyFunction.java:46 BazelModTidyFunction.compute
pub fn compute(
    _key: &BazelModTidyKey,
    env: &mut Environment<'_>,
) -> Result<Option<BazelModTidyValue>, ComputeError> {
    let Some(root_module_file_value) = env.get_value(ModuleFileKey::key_for_root_module())? else {
        return Ok(None);
    };
    let root_module_file_value = root_module_file_value.as_root();
    let Some(dep_graph_value) = env.get_value(BazelDepGraphKey)? else {
        return Ok(None);
    };
    let Some(bazel_tools_repo_mapping) =
        env.get_value(RepositoryMappingKey::key(BAZEL_TOOLS.clone()))?
    else {
        return Ok(None);
    };
    let Some(bazel_tools_mapping) = bazel_tools_repo_mapping.repository_mapping() else {
        panic!("java.lang.NullPointerException: no repository mapping for @bazel_tools");
    };
    let buildozer_label = match Label::parse_with_repo_context(
        // This label always has the ".exe" extension, even on Unix, to get a single static
        // label that works on all platforms.
        "@buildozer_binary//:buildozer.exe",
        &LabelRepoContextImpl::of(BAZEL_TOOLS.clone(), bazel_tools_mapping.clone()),
    ) {
        Ok(label) => label,
        Err(e) => panic!("IllegalStateException: {e}"),
    };
    let buildozer = match get_rooted_path_from_label(&buildozer_label, env)? {
        Ok(Some(path)) => path,
        Ok(None) => return Ok(None),
        Err(message) => panic!("IllegalStateException: {message}"),
    };

    // ImmutableSet of the eval keys, in the column's row order.
    let mut extensions_used_by_root_module: Vec<SingleExtensionEvalKey> = Vec::new();
    if let Some(column) = dep_graph_value
        .extension_usages_table
        .column(&ModuleKey::root())
    {
        for id in column.keys() {
            // Use the eval-only key to avoid errors caused by incorrect imports - we can fix them.
            let key = SingleExtensionValue::eval_key(id.clone());
            if !extensions_used_by_root_module.contains(&key) {
                extensions_used_by_root_module.push(key);
            }
        }
    }
    let sky_keys: Vec<SkyKey> = extensions_used_by_root_module
        .iter()
        .cloned()
        .map(SkyKey::new)
        .collect();
    let result = env.get_values_and_exceptions(&sky_keys)?;
    if env.values_missing() {
        return Ok(None);
    }
    let mut fixups = Vec::new();
    let mut errors: Vec<ExternalDepsException> = Vec::new();
    for extension in &extensions_used_by_root_module {
        let value = match result
            .get_or_throw_typed::<SingleExtensionEvalKey, ExternalDepsException>(extension)
        {
            Ok(value) => value,
            Err(e) => {
                // This extension failed, but we can still tidy up other extensions in keep going
                // mode.
                env.listener().handle(&Event::error(e.get_message()));
                errors.push(e);
                continue;
            }
        };
        let Some(eval_value) = value else {
            return Ok(None);
        };
        if let Some(fixup) = &eval_value.fixup {
            fixups.push(fixup.clone());
        }
    }

    Ok(Some(BazelModTidyValue::create(
        fixups,
        buildozer.as_path(),
        root_module_file_value.module_file_paths.clone(),
        errors,
    )))
}
