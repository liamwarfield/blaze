//! The Skyframe part of `BazelModuleResolutionFunction` (BAZEL_MODULE_RESOLUTION): discovery
//! and selection run by the model ([`blaze_bzlmod_model::BazelModuleResolutionFunction`]) over
//! MODULE_FILE and YANKED_VERSIONS requests, then the RepoSpec batch and the final dep graph.

use std::cell::{Cell, RefCell};

use blaze_bzlmod_model::{
    BazelModuleResolutionFunction, BazelModuleResolutionValue, Checksum, DiscoverAndSelectResult,
    ExternalDepsCode, ExternalDepsException, ModuleFileLoader, ModuleKey, NonRootModuleFileValue,
    RepoSpec, RootModuleFileValue, YankedVersionsLookup, YankedVersionsUtil, YankedVersionsValue,
    resolution::BZLMOD_ALLOWED_YANKED_VERSIONS_ENV,
};
use blaze_events::StoredEventHandler;
use blaze_skyframe::{ComputeError, Environment, Interrupted, SkyKey, Transience};
use blaze_util::profiler::{Profiler, ProfilerTask};
use indexmap::{IndexMap, IndexSet};

use super::config::BzlmodConfig;
use super::keys::{ModuleFileKey, ModuleFileValue, RepoSpecKey, YankedVersionsKey};
use super::precomputed::{
    ALLOWED_YANKED_VERSIONS, BAZEL_COMPATIBILITY_MODE, CHECK_DIRECT_DEPENDENCIES,
    ClientEnvironmentKey,
};
use super::throw;

/// Why a Skyframe request made on behalf of the model could not produce a value.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum LookupFailure {
    /// A dependency was unavailable (Java: null, `env.valuesMissing()`).
    ValuesMissing,
    Interrupted,
}

/// The [`ModuleFileLoader`] and [`YankedVersionsLookup`] given to the model's
/// `discover_and_select`: each call is a Skyframe request on `env`.
///
/// PORT: a missing value (or an interrupt) is recorded here and handed to the model as a
/// sentinel exception. For a missing MODULE_FILE value the model goes on with the round, as
/// Java's `Discovery.run(env, root)` does ([`ModuleFileLoader::is_values_missing`]), so a
/// dependency's `ExternalDepsException` later in the round is still thrown with its dependency
/// chain (no-keep-going error bubbling, where values not computed yet are missing); otherwise
/// the caller returns null (or rethrows the interrupt).
struct SkyframeLookups<'e, 'a> {
    env: RefCell<&'e mut Environment<'a>>,
    failure: Cell<Option<LookupFailure>>,
}

/// The message of the sentinel exception of [`SkyframeLookups::fail`].
const VALUES_MISSING: &str = "values missing";

impl SkyframeLookups<'_, '_> {
    fn fail(&self, failure: LookupFailure) -> ExternalDepsException {
        if self.failure.get() != Some(LookupFailure::Interrupted) {
            self.failure.set(Some(failure));
        }
        ExternalDepsException::with_message(ExternalDepsCode::ExternalDepsUnknown, VALUES_MISSING)
    }
}

impl ModuleFileLoader for SkyframeLookups<'_, '_> {
    fn get_module_file(
        &self,
        key: &ModuleKey,
    ) -> Result<NonRootModuleFileValue, ExternalDepsException> {
        self.get_module_files(std::slice::from_ref(key))
            .pop()
            .expect("one result per key")
    }

    /// The sentinel of [`SkyframeLookups::fail`] for a missing value (not for an interrupt,
    /// which Java propagates at once).
    fn is_values_missing(&self, e: &ExternalDepsException) -> bool {
        self.failure.get() == Some(LookupFailure::ValuesMissing)
            && e.get_code() == ExternalDepsCode::ExternalDepsUnknown
            && e.get_message() == VALUES_MISSING
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/Discovery.java:114 DiscoveryRound.run
    fn get_module_files(
        &self,
        keys: &[ModuleKey],
    ) -> Vec<Result<NonRootModuleFileValue, ExternalDepsException>> {
        if self.failure.get() == Some(LookupFailure::Interrupted) {
            return keys
                .iter()
                .map(|_| Err(self.fail(LookupFailure::Interrupted)))
                .collect();
        }
        let module_file_keys: Vec<ModuleFileKey> =
            keys.iter().cloned().map(ModuleFileKey::key).collect();
        let sky_keys: Vec<SkyKey> = module_file_keys.iter().cloned().map(SkyKey::new).collect();
        let result = {
            let mut env = self.env.borrow_mut();
            env.get_values_and_exceptions(&sky_keys)
        };
        let result = match result {
            Ok(r) => r,
            Err(Interrupted) => {
                return keys
                    .iter()
                    .map(|_| Err(self.fail(LookupFailure::Interrupted)))
                    .collect();
            }
        };
        module_file_keys
            .iter()
            .map(|sky_key| {
                match result.get_or_throw_typed::<ModuleFileKey, ExternalDepsException>(sky_key) {
                    Err(e) => Err(e),
                    Ok(None) => Err(self.fail(LookupFailure::ValuesMissing)),
                    Ok(Some(value)) => Ok(match &*value {
                        ModuleFileValue::NonRoot(v) => v.clone(),
                        // Never requested: the root module is not part of any horizon.
                        ModuleFileValue::Root(v) => NonRootModuleFileValue {
                            module: v.module.clone(),
                            registry_file_hashes: IndexMap::new(),
                        },
                    }),
                }
            })
            .collect()
    }
}

impl YankedVersionsLookup for SkyframeLookups<'_, '_> {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleResolutionFunction.java:208 BazelModuleResolutionFunction.collectYankedVersionsValues
    fn get_yanked_versions(&self, module_name: &str, registry_url: &str) -> YankedVersionsValue {
        if self.failure.get().is_some() {
            return YankedVersionsValue::none_yanked();
        }
        let value = {
            let mut env = self.env.borrow_mut();
            env.get_value(YankedVersionsKey::create(module_name, registry_url))
        };
        match value {
            Ok(Some(v)) => (*v).clone(),
            Ok(None) => {
                self.fail(LookupFailure::ValuesMissing);
                YankedVersionsValue::none_yanked()
            }
            Err(Interrupted) => {
                self.fail(LookupFailure::Interrupted);
                YankedVersionsValue::none_yanked()
            }
        }
    }
}

/// PORT: Java keeps the discover-and-select result in a `SkyKeyComputeState`; the blocking
/// evaluator never restarts, so it is a local.
// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleResolutionFunction.java:80 BazelModuleResolutionFunction.compute
pub fn compute(
    config: &BzlmodConfig,
    env: &mut Environment<'_>,
) -> Result<Option<BazelModuleResolutionValue>, ComputeError> {
    let Some(allowed_yanked_versions_from_env) = env.get_value(ClientEnvironmentKey::key(
        BZLMOD_ALLOWED_YANKED_VERSIONS_ENV,
    ))?
    else {
        return Ok(None);
    };

    let allowed_yanked_versions_from_flag = ALLOWED_YANKED_VERSIONS.get_required(env)?;
    let allowed_yanked_versions = YankedVersionsUtil::parse_allowed_yanked_versions(
        allowed_yanked_versions_from_env.value(),
        &allowed_yanked_versions_from_flag,
    )
    .map_err(|e| throw(e, Transience::Persistent))?;

    let Some(root) = env.get_value(ModuleFileKey::key_for_root_module())? else {
        return Ok(None);
    };
    let root = root.as_root();

    let Some(discover_and_select_result) =
        discover_and_select(config, env, root, allowed_yanked_versions.as_ref())?
    else {
        return Ok(None);
    };

    let mut registry_file_hashes: IndexMap<String, Option<Checksum>> =
        discover_and_select_result.registry_file_hashes.clone();
    let repo_spec_keys: IndexSet<RepoSpecKey> = discover_and_select_result
        .selection_result
        .resolved_dep_graph
        .values()
        // Modules with a null registry have a non-registry override. We don't need to fetch or
        // store the repo spec in this case.
        .filter(|module| module.registry.is_some())
        .map(RepoSpecKey::of)
        .collect();
    let sky_keys: Vec<SkyKey> = repo_spec_keys.iter().cloned().map(SkyKey::new).collect();
    let repo_spec_results = env.get_values_and_exceptions(&sky_keys)?;
    let mut remote_repo_specs: IndexMap<ModuleKey, RepoSpec> = IndexMap::new();
    for repo_spec_key in &repo_spec_keys {
        let Some(repo_spec_value) = repo_spec_results.get_typed(repo_spec_key) else {
            return Ok(None);
        };
        remote_repo_specs.insert(
            repo_spec_key.module_key.clone(),
            repo_spec_value.repo_spec.clone(),
        );
        for (k, v) in &repo_spec_value.registry_file_hashes {
            registry_file_hashes.insert(k.clone(), v.clone());
        }
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleResolutionFunction.java:134 BazelModuleResolutionFunction.compute
    let final_dep_graph = {
        let _c = Profiler::instance().profile(ProfilerTask::BZLMOD, "compute final dep graph");
        BazelModuleResolutionFunction::compute_final_dep_graph(
            &discover_and_select_result
                .selection_result
                .resolved_dep_graph,
            &root.overrides,
            &remote_repo_specs,
        )
    };

    Ok(Some(BazelModuleResolutionValue {
        resolved_dep_graph: final_dep_graph,
        unpruned_dep_graph: discover_and_select_result
            .selection_result
            .unpruned_dep_graph
            .clone(),
        registry_file_hashes,
        selected_yanked_versions: discover_and_select_result.selected_yanked_versions,
    }))
}

/// Runs discovery, selection and the checks of `discoverAndSelect` (in the model) with Skyframe
/// lookups. Events the checks report are buffered and replayed on the environment's listener
/// unless a dependency turned out to be missing (Java returns null before reporting them).
// Java: com/google/devtools/build/lib/bazel/bzlmod/BazelModuleResolutionFunction.java:151 BazelModuleResolutionFunction.discoverAndSelect
fn discover_and_select(
    config: &BzlmodConfig,
    env: &mut Environment<'_>,
    root: &RootModuleFileValue,
    allowed_yanked_versions: Option<&IndexSet<ModuleKey>>,
) -> Result<Option<DiscoverAndSelectResult>, ComputeError> {
    // PORT: Java reads these two precomputed values after discovery; reading them first only
    // changes the order of dependency requests.
    let check_direct_dependencies = *CHECK_DIRECT_DEPENDENCIES.get_required(env)?;
    let bazel_compatibility_mode = *BAZEL_COMPATIBILITY_MODE.get_required(env)?;

    let events = StoredEventHandler::new();
    let (result, failure) = {
        let lookups = SkyframeLookups {
            env: RefCell::new(&mut *env),
            failure: Cell::new(None),
        };
        let result = BazelModuleResolutionFunction::discover_and_select(
            &lookups,
            &lookups,
            root,
            allowed_yanked_versions,
            check_direct_dependencies,
            bazel_compatibility_mode,
            &config.bazel_version,
            &events,
        );
        (result, lookups.failure.get())
    };
    if failure == Some(LookupFailure::Interrupted) {
        return Err(Interrupted.into());
    }
    // Java: Discovery throws a dependency's ExternalDepsException even when other values of the
    // round are missing; only then does `env.valuesMissing()` make it return null.
    if let Err(e) = &result
        && !(failure == Some(LookupFailure::ValuesMissing)
            && e.get_code() == ExternalDepsCode::ExternalDepsUnknown
            && e.get_message() == VALUES_MISSING)
    {
        // Java reports the checks' events as they happen, before throwing.
        events.replay_on(env.listener());
        return Err(throw(e.clone(), Transience::Persistent));
    }
    if failure == Some(LookupFailure::ValuesMissing) {
        return Ok(None);
    }
    events.replay_on(env.listener());
    match result {
        Ok(r) => Ok(Some(r)),
        Err(e) => Err(throw(e, Transience::Persistent)),
    }
}
