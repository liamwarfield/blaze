//! The module extension seam: SINGLE_EXTENSION_EVAL and SINGLE_EXTENSION.
//!
//! `SingleExtensionEvalFunction` runs the extension's Starlark implementation and is ported by
//! another lead. Until then, [`register_lockfile_only_extension_functions`] registers a
//! SINGLE_EXTENSION_EVAL function that only answers from the lockfiles, and a SINGLE_EXTENSION
//! function (`SingleExtensionFunction`, which validates imports) over it. The real functions are
//! registered for the same keys instead of these.

use blaze_bzlmod_model::{
    BazelLockFileValue, ExternalDepsCode, ExternalDepsException, Facts, LockFileModuleExtension,
    LockFileModuleExtensionWithFactors, LockfileMode, ModuleExtensionEvalFactors,
    ModuleExtensionId, ModuleExtensionUsage, ModuleKey,
};
use blaze_cmdline::RepositoryName;
use blaze_options::spelling::SpellChecker;
use blaze_skyframe::{ComputeError, Environment, EvaluatorBuilder, SkyKey, Transience};
use indexmap::IndexMap;

use super::keys::{
    BazelDepGraphKey, BazelLockFileKey, SingleExtensionEvalKey, SingleExtensionKey,
    SingleExtensionValue,
};
use super::precomputed::LOCKFILE_MODE;
use super::throw;

/// Registers the placeholder SINGLE_EXTENSION_EVAL function and SINGLE_EXTENSION.
///
/// PORT: placeholder until SingleExtensionEvalFunction is ported. Replace this call with the
/// registrations of the real `SingleExtensionEvalFunction` and `SingleExtensionFunction`.
pub fn register_lockfile_only_extension_functions(builder: EvaluatorBuilder) -> EvaluatorBuilder {
    builder
        .typed_function::<SingleExtensionEvalKey, _>(|key, env| {
            compute_lockfile_only_extension_eval(key, env)
        })
        .typed_function::<SingleExtensionKey, _>(compute_single_extension)
}

/// The evaluation factors of an extension on this host: `OS.getCurrent().toString()` and the
/// JVM's `os.arch` (`RegularRunnableExtension.getEvalFactors`), for the pinned target platform
/// (Linux x86-64), when the extension depends on them.
fn host_matches(factors: &ModuleExtensionEvalFactors) -> bool {
    (factors.get_os().is_empty() || factors.get_os() == "linux")
        && (factors.get_arch().is_empty() || factors.get_arch() == "amd64")
}

fn locked_extension(
    lockfile: &BazelLockFileValue,
    extension_id: &ModuleExtensionId,
) -> Option<(ModuleExtensionEvalFactors, LockFileModuleExtension)> {
    lockfile
        .module_extensions
        .get(extension_id)?
        .iter()
        .find(|(factors, _)| host_matches(factors))
        .map(|(f, e)| (f.clone(), e.clone()))
}

/// PORT: placeholder until SingleExtensionEvalFunction is ported. Returns the extension's entry
/// from the workspace lockfile (else the hidden lockfile) without checking that it is up to date
/// (Java: `tryGettingValueFromLockFile` compares the .bzl digest, usages and recorded inputs, which
/// needs the extension's Starlark). Without an entry it fails with EXTENSION_EVAL_ERROR.
/// Facts come from the workspace lockfile, else the hidden one, at their stored facts version.
// Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionEvalFunction.java:98 SingleExtensionEvalFunction.compute
fn compute_lockfile_only_extension_eval(
    key: &SingleExtensionEvalKey,
    env: &mut Environment<'_>,
) -> Result<Option<SingleExtensionValue>, ComputeError> {
    let extension_id = key.argument();
    let Some(dep_graph_value) = env.get_value(BazelDepGraphKey)? else {
        return Ok(None);
    };
    let unported = || {
        throw(
            ExternalDepsException::with_message(
                ExternalDepsCode::ExtensionEvalError,
                format!("unported: SingleExtensionEvalFunction for {extension_id}"),
            ),
            Transience::Persistent,
        )
    };
    let lockfile_mode = *LOCKFILE_MODE.get_required(env)?;
    if lockfile_mode == LockfileMode::Off {
        return Err(unported());
    }
    let keys = [
        SkyKey::new(BazelLockFileKey::KEY),
        SkyKey::new(BazelLockFileKey::HIDDEN_KEY),
    ];
    let lockfiles = env.get_values_and_exceptions(&keys)?;
    let workspace_lockfile = lockfiles.get_typed(&BazelLockFileKey::KEY);
    let hidden_lockfile = lockfiles.get_typed(&BazelLockFileKey::HIDDEN_KEY);
    let (Some(workspace_lockfile), Some(hidden_lockfile)) = (workspace_lockfile, hidden_lockfile)
    else {
        return Ok(None);
    };
    let Some((factors, locked_extension)) = locked_extension(&workspace_lockfile, extension_id)
        .or_else(|| locked_extension(&hidden_lockfile, extension_id))
    else {
        return Err(unported());
    };
    let (facts, facts_version) = if let Some(f) = workspace_lockfile.facts.get(extension_id) {
        (
            f.clone(),
            *workspace_lockfile
                .facts_versions
                .get(extension_id)
                .unwrap_or(&0),
        )
    } else if let Some(f) = hidden_lockfile.facts.get(extension_id) {
        (
            f.clone(),
            *hidden_lockfile
                .facts_versions
                .get(extension_id)
                .unwrap_or(&0),
        )
    } else {
        (Facts::empty(), 0)
    };
    let extension_unique_name = dep_graph_value
        .extension_unique_names
        .get(extension_id)
        .expect("We never request an extension without usages in Skyframe.");
    let generated_repo_specs = locked_extension.generated_repo_specs.clone();
    let canonical_repo_name_to_internal_names: IndexMap<RepositoryName, String> =
        generated_repo_specs
            .keys()
            .map(|internal_name| {
                (
                    SingleExtensionValue::repository_name(extension_unique_name, internal_name),
                    internal_name.clone(),
                )
            })
            .collect();
    let lock_file_info =
        if lockfile_mode == LockfileMode::Update || lockfile_mode == LockfileMode::Refresh {
            Some(LockFileModuleExtensionWithFactors {
                extension_factors: factors,
                module_extension: locked_extension,
            })
        } else {
            None
        };
    Ok(Some(SingleExtensionValue {
        generated_repo_specs,
        canonical_repo_name_to_internal_names,
        lock_file_info,
        fixup: None,
        facts,
        facts_version,
    }))
}

/// Validates the imports of an extension's usages against the repos it generated (the
/// lockfile-only setup of this crate's tests; the binary registers
/// `blaze_repo_rules::bzlmod::single_extension::compute`, which reads SINGLE_EXTENSION_USAGES as
/// Java does).
///
/// PORT: reads the usages from BAZEL_DEP_GRAPH: this crate is below SINGLE_EXTENSION_USAGES.
// Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionFunction.java:37 SingleExtensionFunction.compute
pub fn compute_single_extension(
    key: &SingleExtensionKey,
    env: &mut Environment<'_>,
) -> Result<Option<SingleExtensionValue>, ComputeError> {
    let extension_id = key.argument();
    let Some(dep_graph_value) = env.get_value(BazelDepGraphKey)? else {
        return Ok(None);
    };
    let Some(eval_only_value) =
        env.get_value(SingleExtensionValue::eval_key(extension_id.clone()))?
    else {
        return Ok(None);
    };
    let empty_usages = IndexMap::new();
    let extension_usages = dep_graph_value
        .extension_usages_table
        .row(extension_id)
        .unwrap_or(&empty_usages);
    let empty_overrides = IndexMap::new();
    let repo_overrides = dep_graph_value
        .repo_overrides
        .row(extension_id)
        .unwrap_or(&empty_overrides);
    check_single_extension(
        extension_id,
        &eval_only_value,
        extension_usages,
        repo_overrides,
        env,
    )?;
    Ok(Some((*eval_only_value).clone()))
}

/// The body of `SingleExtensionFunction.compute` after its two lookups: reports the fixup
/// warning and checks the imports and repo overrides of the usages.
// Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionFunction.java:51 SingleExtensionFunction.compute
/// `new SingleExtensionFunctionException(cause, transience)`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionFunction.java:113 SingleExtensionFunction.SingleExtensionFunctionException.<init>
fn single_extension_function_exception(
    cause: ExternalDepsException,
    transience: Transience,
) -> ComputeError {
    throw(cause, transience)
}

pub fn check_single_extension(
    extension_id: &ModuleExtensionId,
    eval_only_value: &SingleExtensionValue,
    extension_usages: &IndexMap<ModuleKey, ModuleExtensionUsage>,
    repo_overrides: &IndexMap<String, RepositoryName>,
    env: &mut Environment<'_>,
) -> Result<(), ComputeError> {
    // SingleExtensionEvalFunction doesn't handle the fixup warning so that bazel mod tidy doesn't
    // show it.
    if let Some(fixup) = &eval_only_value.fixup {
        env.listener().handle(&fixup.warning);
    }

    // Check that all imported repos have actually been generated.
    for usage in extension_usages.values() {
        for proxy in &usage.proxies {
            for (local_name, exported_name) in &proxy.imports {
                if !eval_only_value
                    .generated_repo_specs
                    .contains_key(exported_name)
                    && !repo_overrides.contains_key(exported_name)
                {
                    return Err(single_extension_function_exception(
                        ExternalDepsException::with_message(
                            ExternalDepsCode::InvalidExtensionImport,
                            format!(
                                "module extension {} does not generate repository \"{}\", yet it \
                                 is imported as \"{}\" in the usage at {}{}",
                                extension_id,
                                exported_name,
                                local_name,
                                proxy.location,
                                SpellChecker::did_you_mean(
                                    exported_name,
                                    eval_only_value.generated_repo_specs.keys()
                                )
                            ),
                        ),
                        Transience::Persistent,
                    ));
                }
            }
        }
    }

    // Check that repo overrides apply as declared.
    for usage in extension_usages.values() {
        for (name, override_) in &usage.repo_overrides {
            let repo_exists = eval_only_value.generated_repo_specs.contains_key(name);
            if repo_exists && !override_.must_exist {
                return Err(single_extension_function_exception(
                    ExternalDepsException::with_message(
                        ExternalDepsCode::InvalidExtensionImport,
                        format!(
                            "module extension {} generates repository \"{}\", yet it is injected \
                             via inject_repo() at {}. Use override_repo() instead to override an \
                             existing repository.",
                            extension_id, name, override_.location
                        ),
                    ),
                    Transience::Persistent,
                ));
            } else if !repo_exists && override_.must_exist {
                return Err(single_extension_function_exception(
                    ExternalDepsException::with_message(
                        ExternalDepsCode::InvalidExtensionImport,
                        format!(
                            "module extension {} does not generate repository \"{}\", yet it is \
                             overridden via override_repo() at {}. Use inject_repo() instead to \
                             inject a new repository.",
                            extension_id, name, override_.location
                        ),
                    ),
                    Transience::Persistent,
                ));
            }
        }
    }

    Ok(())
}
