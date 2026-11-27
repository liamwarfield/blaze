//! Ports of `query2/ParallelSkyQueryUtils.java`, `query2/ParallelVisitorUtils.java` and the
//! visitors SkyQuery runs through them: `AbstractTargetOuputtingVisitor`,
//! `DepsUnboundedVisitor`, `RdepsUnboundedVisitor`, `RdepsBoundedVisitor`,
//! `RdepsVisitorUtils`, `AbstractUnfilteredLabelDTCVisitor`, `UnfilteredSkyKeyLabelDTCVisitor`
//! and `AbstractSkyKeyParallelVisitor`.
//!
//! PORT: the visitations run on the calling thread (see `blaze_sky_loading::parallel_visitor`):
//! the result sets are Java's; the order in which results are streamed is the order of the
//! sequential visitation (Java's depends on thread scheduling). Java's `extraGlobalDeps` is
//! always empty in Bazel and is not ported.

use std::collections::{HashMap, HashSet};

use blaze_cmdline::PackageIdentifier;
use blaze_packages::target::Target;
use blaze_query_engine::{
    Callback, MinDepthUniquifier, QueryEnvironment, QueryEnvironmentFutures, QueryError,
    QueryExpression, QueryExpressionContext, QueryTaskFuture, Uniquifier, callback_fn,
};
use blaze_sky_loading::parallel_visitor::{self, ParallelVisitor, Visit, partition};
use blaze_skyframe::SkyKey;
use blaze_vfs::PathFragment;
use indexmap::{IndexMap, IndexSet};

use crate::sky_query::rbuild_files::RBuildFilesVisitor;
use crate::sky_query::sky_query_environment::{
    BATCH_CALLBACK_SIZE, SkyQueryEnvironment, is_label, label_key,
    make_package_key_to_target_key_map, sky_key_to_label, target_to_sky_key,
};

// Java: com/google/devtools/build/lib/query2/ParallelSkyQueryUtils.java:53 ParallelSkyQueryUtils.VISIT_BATCH_SIZE
pub const VISIT_BATCH_SIZE: usize = 10000;

/// A dependency edge seen from its reverse side (Java `DepAndRdep`).
// Java: com/google/devtools/build/lib/query2/ParallelSkyQueryUtils.java:184 ParallelSkyQueryUtils.DepAndRdep
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct DepAndRdep {
    pub dep: Option<SkyKey>,
    pub rdep: SkyKey,
}

// Java: com/google/devtools/build/lib/query2/ParallelSkyQueryUtils.java:210 ParallelSkyQueryUtils.DepAndRdepAtDepth
#[derive(Clone, Debug)]
pub struct DepAndRdepAtDepth {
    pub dep_and_rdep: DepAndRdep,
    pub rdep_depth: i32,
}

/// `env.eval(expression, context, createParallelVisitorCallback(factory))`: each batch of the
/// expression's result is visited by a new visitor from `make_visitor` (Java
/// `ParallelTargetVisitorCallback.process`).
// Java: com/google/devtools/build/lib/query2/ParallelVisitorUtils.java:60 ParallelVisitorUtils.ParallelTargetVisitorCallback
// Java: com/google/devtools/build/lib/query2/ParallelVisitorUtils.java:45 ParallelVisitorUtils.createParallelVisitorCallback
// Java: com/google/devtools/build/lib/query2/ParallelVisitorUtils.java:72 ParallelVisitorUtils.ParallelTargetVisitorCallback.process
// PORT: skipped, the callback is the closure below and each visitor a struct literal holding the environment's fields (no visit-task status callback, no executor): ParallelVisitorUtils.ParallelTargetVisitorCallback.<init>, ParallelVisitorUtils.ParallelQueryVisitor.<init>
fn eval_with_parallel_visitor_callback<V>(
    env: &SkyQueryEnvironment,
    expression: &QueryExpression,
    context: &QueryExpressionContext<Target>,
    make_visitor: &dyn Fn() -> V,
) -> QueryTaskFuture<()>
where
    V: ParallelVisitor<Input = SkyKey, Error = QueryError>,
{
    env.eval(
        expression,
        context,
        &callback_fn(|partial_result: &[Target]| {
            let visitor = make_visitor();
            parallel_visitor::visit_and_wait_for_completion(
                &visitor,
                partial_result.iter().map(target_to_sky_key).collect(),
            )
        }),
    )
}

// Java: com/google/devtools/build/lib/query2/ParallelSkyQueryUtils.java:58 ParallelSkyQueryUtils.getAllRdepsUnboundedParallel
pub fn get_all_rdeps_unbounded_parallel(
    env: &SkyQueryEnvironment,
    expression: &QueryExpression,
    context: &QueryExpressionContext<Target>,
    callback: &dyn Callback<Target>,
) -> QueryTaskFuture<()> {
    let valid_rdep_uniquifier = env.create_sky_key_uniquifier();
    let unfiltered_universe = |_: &SkyKey| true;
    eval_with_parallel_visitor_callback(env, expression, context, &|| RdepsUnboundedVisitor {
        env,
        valid_rdep_uniquifier: &valid_rdep_uniquifier,
        unfiltered_universe: &unfiltered_universe,
        callback,
    })
}

// Java: com/google/devtools/build/lib/query2/ParallelSkyQueryUtils.java:74 ParallelSkyQueryUtils.getAllRdepsBoundedParallel
pub fn get_all_rdeps_bounded_parallel(
    env: &SkyQueryEnvironment,
    expression: &QueryExpression,
    depth: i32,
    context: &QueryExpressionContext<Target>,
    callback: &dyn Callback<Target>,
) -> QueryTaskFuture<()> {
    get_rdeps_in_universe_bounded_parallel(env, expression, depth, &|_| true, context, callback)
}

// Java: com/google/devtools/build/lib/query2/ParallelSkyQueryUtils.java:92 ParallelSkyQueryUtils.getRdepsInUniverseUnboundedParallel
pub fn get_rdeps_in_universe_unbounded_parallel(
    env: &SkyQueryEnvironment,
    expression: &QueryExpression,
    unfiltered_universe: &dyn Fn(&SkyKey) -> bool,
    context: &QueryExpressionContext<Target>,
    callback: &dyn Callback<Target>,
) -> QueryTaskFuture<()> {
    let valid_rdep_uniquifier = env.create_sky_key_uniquifier();
    eval_with_parallel_visitor_callback(env, expression, context, &|| RdepsUnboundedVisitor {
        env,
        valid_rdep_uniquifier: &valid_rdep_uniquifier,
        unfiltered_universe,
        callback,
    })
}

/// The label keys of the transitive closure of the universe expression's targets.
// Java: com/google/devtools/build/lib/query2/ParallelSkyQueryUtils.java:106 ParallelSkyQueryUtils.getDTCSkyKeyPredicateFuture
pub fn get_dtc_sky_key_predicate_future(
    env: &SkyQueryEnvironment,
    expression: &QueryExpression,
    context: &QueryExpressionContext<Target>,
) -> QueryTaskFuture<HashSet<SkyKey>> {
    let universe_value_future = blaze_query_engine::query_util::eval_all(env, context, expression);
    env.transform_async(universe_value_future, |universe_value| {
        env.execute(|| {
            // Java ThreadSafeAggregateAllSkyKeysCallback.
            let aggregate_all_callback: std::cell::RefCell<HashSet<SkyKey>> =
                std::cell::RefCell::new(HashSet::new());
            let uniquifier = env.create_sky_key_uniquifier();
            let visitor = UnfilteredSkyKeyLabelDTCVisitor {
                env,
                uniquifier: &uniquifier,
                process_results_batch_size: BATCH_CALLBACK_SIZE,
                callback: &|partial_result: Vec<SkyKey>| {
                    aggregate_all_callback.borrow_mut().extend(partial_result);
                },
            };
            parallel_visitor::visit_and_wait_for_completion(
                &visitor,
                universe_value
                    .to_vec()
                    .iter()
                    .map(target_to_sky_key)
                    .collect(),
            )?;
            Ok(aggregate_all_callback.into_inner())
        })
    })
}

// Java: com/google/devtools/build/lib/query2/ParallelSkyQueryUtils.java:139 ParallelSkyQueryUtils.getRdepsInUniverseBoundedParallel
pub fn get_rdeps_in_universe_bounded_parallel(
    env: &SkyQueryEnvironment,
    expression: &QueryExpression,
    depth: i32,
    universe: &dyn Fn(&SkyKey) -> bool,
    context: &QueryExpressionContext<Target>,
    callback: &dyn Callback<Target>,
) -> QueryTaskFuture<()> {
    let valid_rdep_min_depth_uniquifier = env.create_min_depth_sky_key_uniquifier();
    eval_with_parallel_visitor_callback(env, expression, context, &|| RdepsBoundedVisitor {
        env,
        depth,
        valid_rdep_min_depth_uniquifier: &valid_rdep_min_depth_uniquifier,
        universe,
        callback,
    })
}

// Java: com/google/devtools/build/lib/query2/ParallelSkyQueryUtils.java:155 ParallelSkyQueryUtils.getRBuildFilesParallel
pub fn get_rbuild_files_parallel(
    env: &SkyQueryEnvironment,
    file_identifiers: &[PathFragment],
    callback: &dyn Callback<Target>,
) -> Result<(), QueryError> {
    let visit_uniquifier = env.create_sky_key_uniquifier();
    let result_uniquifier = env.create_sky_key_uniquifier();
    let visitor = RBuildFilesVisitor::new(env, &visit_uniquifier, &result_uniquifier, callback);
    visitor.visit_file_identifiers_and_wait_for_completion(&*env.graph(), file_identifiers)
}

// Java: com/google/devtools/build/lib/query2/ParallelSkyQueryUtils.java:170 ParallelSkyQueryUtils.getDepsUnboundedParallel
pub fn get_deps_unbounded_parallel(
    env: &SkyQueryEnvironment,
    expression: &QueryExpression,
    context: &QueryExpressionContext<Target>,
    callback: &dyn Callback<Target>,
    deps_need_filtering: bool,
    caller: &QueryExpression,
) -> QueryTaskFuture<()> {
    let valid_dep_uniquifier = env.create_sky_key_uniquifier();
    eval_with_parallel_visitor_callback(env, expression, context, &|| DepsUnboundedVisitor {
        env,
        valid_dep_uniquifier: &valid_dep_uniquifier,
        callback,
        deps_need_filtering,
        context,
        caller,
    })
}

// ---------------------------------------------------------------------------------------------
// AbstractTargetOuputtingVisitor

// PORT: skipped, each visitor is a struct literal with the batch sizes as constants, the package semaphore bounds parallel tasks (the visits run one at a time here), and visitationKeyToOutputKey is the closure argument of `target_outputting_get_visit_tasks`: AbstractTargetOuputtingVisitor.<init>, AbstractTargetOuputtingVisitor.getPackageSemaphore, AbstractTargetOuputtingVisitor.visitationKeyToOutputKey
// Java: com/google/devtools/build/lib/query2/AbstractTargetOuputtingVisitor.java:39 AbstractTargetOuputtingVisitor.PROCESS_RESULTS_BATCH_SIZE
const TARGET_OUTPUTTING_PROCESS_RESULTS_BATCH_SIZE: usize = BATCH_CALLBACK_SIZE;

/// Java `AbstractTargetOuputtingVisitor.outputKeysToOutputValues` with the visitor's
/// `handleMissingTargets` (the default, which does nothing, is `&|_, _| Ok(())`).
// Java: com/google/devtools/build/lib/query2/AbstractTargetOuputtingVisitor.java:62 AbstractTargetOuputtingVisitor.handleMissingTargets
// Java: com/google/devtools/build/lib/query2/AbstractTargetOuputtingVisitor.java:53 AbstractTargetOuputtingVisitor.outputKeysToOutputValues
fn target_outputting_output_keys_to_output_values(
    env: &SkyQueryEnvironment,
    target_keys: Vec<SkyKey>,
    handle_missing_targets: &dyn Fn(
        &[(blaze_cmdline::Label, Target)],
        &[SkyKey],
    ) -> Result<(), QueryError>,
) -> Result<Vec<Target>, QueryError> {
    let labels: Vec<blaze_cmdline::Label> =
        target_keys.iter().filter_map(sky_key_to_label).collect();
    let targets = env.get_targets_by_label(&labels);
    let unique_keys: Vec<SkyKey> = target_keys
        .into_iter()
        .collect::<IndexSet<SkyKey>>()
        .into_iter()
        .collect();
    handle_missing_targets(&targets, &unique_keys)?;
    Ok(targets.into_iter().map(|(_, t)| t).collect())
}

/// Java `AbstractTargetOuputtingVisitor.getVisitTasks`: the visits grouped by the package of
/// their output key, in batches of `VISIT_BATCH_SIZE`.
///
/// PORT: Java's `ArrayListMultimap` groups the packages in hash order; here in first-seen order.
// Java: com/google/devtools/build/lib/query2/AbstractTargetOuputtingVisitor.java:68 AbstractTargetOuputtingVisitor.getVisitTasks
fn target_outputting_get_visit_tasks<K>(
    pending_visits: Vec<K>,
    visitation_key_to_output_key: impl Fn(&K) -> Option<SkyKey>,
) -> Vec<Vec<K>> {
    let mut visits_by_package: IndexMap<PackageIdentifier, Vec<K>> = IndexMap::new();
    for visitation_key in pending_visits {
        if let Some(sky_key) = visitation_key_to_output_key(&visitation_key) {
            let label = sky_key_to_label(&sky_key).expect("a label key");
            visits_by_package
                .entry(label.get_package_identifier().clone())
                .or_default()
                .push(visitation_key);
        }
    }
    partition(
        visits_by_package.into_values().flatten().collect(),
        VISIT_BATCH_SIZE,
    )
}

// ---------------------------------------------------------------------------------------------
// DepsUnboundedVisitor

/// A visitor that visits the transitive dependencies of targets and outputs them.
// Java: com/google/devtools/build/lib/query2/DepsUnboundedVisitor.java:38 DepsUnboundedVisitor
struct DepsUnboundedVisitor<'a> {
    env: &'a SkyQueryEnvironment,
    valid_dep_uniquifier: &'a dyn Uniquifier<SkyKey>,
    callback: &'a dyn Callback<Target>,
    deps_need_filtering: bool,
    context: &'a QueryExpressionContext<Target>,
    caller: &'a QueryExpression,
}

impl ParallelVisitor for DepsUnboundedVisitor<'_> {
    type Input = SkyKey;
    type VisitKey = SkyKey;
    type OutputKey = SkyKey;
    type OutputResult = Target;
    type Error = QueryError;

    fn visit_batch_size(&self) -> usize {
        VISIT_BATCH_SIZE
    }

    fn process_results_batch_size(&self) -> usize {
        TARGET_OUTPUTTING_PROCESS_RESULTS_BATCH_SIZE
    }

    fn output_keys_to_output_values(
        &self,
        target_keys: Vec<SkyKey>,
    ) -> Result<Vec<Target>, QueryError> {
        target_outputting_output_keys_to_output_values(self.env, target_keys, &|found, all| {
            // Java: com/google/devtools/build/lib/query2/DepsUnboundedVisitor.java:149 DepsUnboundedVisitor.handleMissingTargets
            self.env
                .report_unsuccessful_or_missing_targets(found, all, self.caller)
        })
    }

    // Java: com/google/devtools/build/lib/query2/DepsUnboundedVisitor.java:99 DepsUnboundedVisitor.getVisitResult
    fn get_visit_result(&self, keys: Vec<SkyKey>) -> Result<Visit<SkyKey, SkyKey>, QueryError> {
        if self.deps_need_filtering {
            let package_key_to_target_key_map = make_package_key_to_target_key_map(&keys);
            let targets: Vec<Target> = self
                .env
                .get_target_key_to_target_map_for_package_key_to_target_key_map(
                    &package_key_to_target_key_map,
                )
                .into_values()
                .collect();
            let deps_as_targets = self
                .env
                .get_fwd_deps(&targets, self.context)
                .map_err(QueryError::Interrupted)?;
            let deps_as_sky_keys: Vec<SkyKey> = deps_as_targets
                .iter()
                .map(|t| label_key(t.get_label()))
                .collect();
            return Ok(Visit {
                keys_to_use_for_result: keys,
                keys_to_visit: deps_as_sky_keys,
            });
        }
        // ImmutableSet.copyOf(Iterables.concat(fwdDepLabels.values())).
        let mut keys_to_visit: IndexSet<SkyKey> = IndexSet::new();
        for (_, deps) in self.env.get_fwd_dep_labels(&keys) {
            keys_to_visit.extend(deps);
        }
        Ok(Visit {
            keys_to_use_for_result: keys,
            keys_to_visit: keys_to_visit.into_iter().collect(),
        })
    }

    // Java: com/google/devtools/build/lib/query2/DepsUnboundedVisitor.java:133 DepsUnboundedVisitor.preprocessInitialVisit
    fn preprocess_initial_visit(&self, visitation_keys: Vec<SkyKey>) -> Vec<SkyKey> {
        visitation_keys
    }

    // Java: com/google/devtools/build/lib/query2/DepsUnboundedVisitor.java:143 DepsUnboundedVisitor.noteAndReturnUniqueVisitationKeys
    fn note_and_return_unique_visitation_keys(
        &self,
        prospective_visitation_keys: Vec<SkyKey>,
    ) -> Result<Vec<SkyKey>, QueryError> {
        Ok(self
            .valid_dep_uniquifier
            .unique_all(&prospective_visitation_keys)?)
    }

    fn get_visit_tasks(&self, pending: Vec<SkyKey>) -> Result<Vec<Vec<SkyKey>>, QueryError> {
        // Java: com/google/devtools/build/lib/query2/DepsUnboundedVisitor.java:138 DepsUnboundedVisitor.visitationKeyToOutputKey
        Ok(target_outputting_get_visit_tasks(pending, |k| {
            Some(k.clone())
        }))
    }

    fn process(&self, partial_result: Vec<Target>) -> Result<(), QueryError> {
        self.callback.process(&partial_result)
    }
}

// ---------------------------------------------------------------------------------------------
// RdepsVisitorUtils

// Java: com/google/devtools/build/lib/query2/RdepsVisitorUtils.java:31 RdepsVisitorUtils.getMaybeFilteredRdeps
fn get_maybe_filtered_rdeps(
    dep_and_rdeps: &[DepAndRdep],
    env: &SkyQueryEnvironment,
) -> Vec<SkyKey> {
    if env.has_dependency_filter() {
        get_filtered_rdeps(dep_and_rdeps, env)
    } else {
        get_rdeps(dep_and_rdeps)
    }
}

// Java: com/google/devtools/build/lib/query2/RdepsVisitorUtils.java:36 RdepsVisitorUtils.getFilteredRdeps
fn get_filtered_rdeps(dep_and_rdeps: &[DepAndRdep], env: &SkyQueryEnvironment) -> Vec<SkyKey> {
    let mut filtered_rdeps: Vec<SkyKey> = Vec::new();
    // ArrayListMultimap<dep, rdep>: deps in first-seen order (PORT: Java's hash order).
    let mut reverse_dep_multimap: IndexMap<SkyKey, Vec<SkyKey>> = IndexMap::new();
    for dep_and_rdep in dep_and_rdeps {
        match &dep_and_rdep.dep {
            None => filtered_rdeps.push(dep_and_rdep.rdep.clone()),
            Some(dep) => reverse_dep_multimap
                .entry(dep.clone())
                .or_default()
                .push(dep_and_rdep.rdep.clone()),
        }
    }
    let package_key_to_target_key_map =
        make_package_key_to_target_key_map(reverse_dep_multimap.values().flatten());
    if !reverse_dep_multimap.is_empty() {
        let as_map: Vec<(SkyKey, Vec<SkyKey>)> = reverse_dep_multimap.into_iter().collect();
        let filtered_targets = env.filter_raw_reverse_deps_of_transitive_traversal_keys(
            &as_map,
            &package_key_to_target_key_map,
        );
        filtered_rdeps.extend(filtered_targets.iter().map(target_to_sky_key));
    }
    filtered_rdeps
}

// Java: com/google/devtools/build/lib/query2/RdepsVisitorUtils.java:73 RdepsVisitorUtils.getRdeps
fn get_rdeps(dep_and_rdeps: &[DepAndRdep]) -> Vec<SkyKey> {
    dep_and_rdeps.iter().map(|d| d.rdep.clone()).collect()
}

// ---------------------------------------------------------------------------------------------
// RdepsUnboundedVisitor

/// A visitor that visits the reverse dependencies of targets, in the universe, and outputs them.
// Java: com/google/devtools/build/lib/query2/RdepsUnboundedVisitor.java:45 RdepsUnboundedVisitor
struct RdepsUnboundedVisitor<'a> {
    env: &'a SkyQueryEnvironment,
    valid_rdep_uniquifier: &'a dyn Uniquifier<SkyKey>,
    unfiltered_universe: &'a dyn Fn(&SkyKey) -> bool,
    callback: &'a dyn Callback<Target>,
}

impl ParallelVisitor for RdepsUnboundedVisitor<'_> {
    type Input = SkyKey;
    type VisitKey = DepAndRdep;
    type OutputKey = SkyKey;
    type OutputResult = Target;
    type Error = QueryError;

    fn visit_batch_size(&self) -> usize {
        VISIT_BATCH_SIZE
    }

    fn process_results_batch_size(&self) -> usize {
        TARGET_OUTPUTTING_PROCESS_RESULTS_BATCH_SIZE
    }

    fn output_keys_to_output_values(
        &self,
        target_keys: Vec<SkyKey>,
    ) -> Result<Vec<Target>, QueryError> {
        target_outputting_output_keys_to_output_values(self.env, target_keys, &|_, _| Ok(()))
    }

    // Java: com/google/devtools/build/lib/query2/RdepsUnboundedVisitor.java:103 RdepsUnboundedVisitor.getVisitResult
    fn get_visit_result(
        &self,
        dep_and_rdeps: Vec<DepAndRdep>,
    ) -> Result<Visit<SkyKey, DepAndRdep>, QueryError> {
        let mut unique_valid_rdeps: Vec<SkyKey> = Vec::new();
        for rdep in get_maybe_filtered_rdeps(&dep_and_rdeps, self.env) {
            if self.valid_rdep_uniquifier.unique(&rdep)? {
                unique_valid_rdeps.push(rdep);
            }
        }
        let mut dep_and_rdeps_to_visit: Vec<DepAndRdep> = Vec::new();
        for (key, value) in self
            .env
            .get_reverse_dep_labels_of_labels(&unique_valid_rdeps)
        {
            for rdep in value {
                if is_label(&rdep) && (self.unfiltered_universe)(&rdep) {
                    dep_and_rdeps_to_visit.push(DepAndRdep {
                        dep: Some(key.clone()),
                        rdep,
                    });
                }
            }
        }
        Ok(Visit {
            keys_to_use_for_result: unique_valid_rdeps,
            keys_to_visit: dep_and_rdeps_to_visit,
        })
    }

    // Java: com/google/devtools/build/lib/query2/RdepsUnboundedVisitor.java:152 RdepsUnboundedVisitor.preprocessInitialVisit
    fn preprocess_initial_visit(&self, sky_keys: Vec<SkyKey>) -> Vec<DepAndRdep> {
        sky_keys
            .into_iter()
            .filter(|k| (self.unfiltered_universe)(k))
            .map(|key| DepAndRdep {
                dep: None,
                rdep: key,
            })
            .collect()
    }

    // Java: com/google/devtools/build/lib/query2/RdepsUnboundedVisitor.java:140 RdepsUnboundedVisitor.noteAndReturnUniqueVisitationKeys
    fn note_and_return_unique_visitation_keys(
        &self,
        prospective_visitation_keys: Vec<DepAndRdep>,
    ) -> Result<Vec<DepAndRdep>, QueryError> {
        // Iterables.filter is lazy in Java: the predicate runs when the keys are consumed,
        // which is right away here.
        Ok(prospective_visitation_keys
            .into_iter()
            .filter(|dep_and_rdep| self.valid_rdep_uniquifier.unique_pure(&dep_and_rdep.rdep))
            .collect())
    }

    fn get_visit_tasks(
        &self,
        pending: Vec<DepAndRdep>,
    ) -> Result<Vec<Vec<DepAndRdep>>, QueryError> {
        // Java: com/google/devtools/build/lib/query2/RdepsUnboundedVisitor.java:135 RdepsUnboundedVisitor.visitationKeyToOutputKey
        Ok(target_outputting_get_visit_tasks(pending, |k| {
            Some(k.rdep.clone())
        }))
    }

    fn process(&self, partial_result: Vec<Target>) -> Result<(), QueryError> {
        self.callback.process(&partial_result)
    }
}

// ---------------------------------------------------------------------------------------------
// RdepsBoundedVisitor

/// A visitor that visits the reverse dependencies of targets up to a depth, in the universe.
// Java: com/google/devtools/build/lib/query2/RdepsBoundedVisitor.java:42 RdepsBoundedVisitor
struct RdepsBoundedVisitor<'a> {
    env: &'a SkyQueryEnvironment,
    depth: i32,
    valid_rdep_min_depth_uniquifier: &'a dyn MinDepthUniquifier<SkyKey>,
    universe: &'a dyn Fn(&SkyKey) -> bool,
    callback: &'a dyn Callback<Target>,
}

impl ParallelVisitor for RdepsBoundedVisitor<'_> {
    type Input = SkyKey;
    type VisitKey = DepAndRdepAtDepth;
    type OutputKey = SkyKey;
    type OutputResult = Target;
    type Error = QueryError;

    fn visit_batch_size(&self) -> usize {
        VISIT_BATCH_SIZE
    }

    fn process_results_batch_size(&self) -> usize {
        TARGET_OUTPUTTING_PROCESS_RESULTS_BATCH_SIZE
    }

    // Java: com/google/devtools/build/lib/query2/RdepsBoundedVisitor.java:170 RdepsBoundedVisitor.outputKeysToOutputValues
    fn output_keys_to_output_values(
        &self,
        target_keys: Vec<SkyKey>,
    ) -> Result<Vec<Target>, QueryError> {
        let mut not_yet_output_keys: Vec<SkyKey> = Vec::with_capacity(target_keys.len());
        for target_key in target_keys {
            if self
                .valid_rdep_min_depth_uniquifier
                .unique_for_output(&target_key)
            {
                not_yet_output_keys.push(target_key);
            }
        }
        target_outputting_output_keys_to_output_values(
            self.env,
            not_yet_output_keys,
            &|_, _| Ok(()),
        )
    }

    // Java: com/google/devtools/build/lib/query2/RdepsBoundedVisitor.java:92 RdepsBoundedVisitor.getVisitResult
    fn get_visit_result(
        &self,
        dep_and_rdep_at_depths: Vec<DepAndRdepAtDepth>,
    ) -> Result<Visit<SkyKey, DepAndRdepAtDepth>, QueryError> {
        let mut shallowest_rdep_depth_map: HashMap<SkyKey, i32> = HashMap::new();
        for dep_and_rdep_at_depth in &dep_and_rdep_at_depths {
            shallowest_rdep_depth_map
                .entry(dep_and_rdep_at_depth.dep_and_rdep.rdep.clone())
                .and_modify(|d| *d = (*d).min(dep_and_rdep_at_depth.rdep_depth))
                .or_insert(dep_and_rdep_at_depth.rdep_depth);
        }
        let dep_and_rdeps: Vec<DepAndRdep> = dep_and_rdep_at_depths
            .iter()
            .map(|d| d.dep_and_rdep.clone())
            .collect();
        let mut unique_valid_rdeps: Vec<SkyKey> = Vec::new();
        for valid_rdep in get_maybe_filtered_rdeps(&dep_and_rdeps, self.env) {
            let depth = shallowest_rdep_depth_map
                .get(&valid_rdep)
                .copied()
                .expect("NullPointerException: shallowestRdepDepthMap.get(validRdep)");
            if self
                .valid_rdep_min_depth_uniquifier
                .unique_at_depth_less_than_or_equal_to(&valid_rdep, depth)?
            {
                unique_valid_rdeps.push(valid_rdep);
            }
        }
        let unique_valid_rdeps_below_depth_bound: Vec<SkyKey> = unique_valid_rdeps
            .iter()
            .filter(|k| shallowest_rdep_depth_map[*k] < self.depth)
            .cloned()
            .collect();
        let unfiltered_rdeps_of_rdeps = self
            .env
            .get_reverse_dep_labels_of_labels(&unique_valid_rdeps_below_depth_bound);
        let mut dep_and_rdep_at_depths_to_visit: Vec<DepAndRdepAtDepth> = Vec::new();
        for (rdep, rdeps_of_rdep) in unfiltered_rdeps_of_rdeps {
            let depth_of_rdep_of_rdep = shallowest_rdep_depth_map[&rdep] + 1;
            for rdep_of_rdep in rdeps_of_rdep {
                if is_label(&rdep_of_rdep) && (self.universe)(&rdep_of_rdep) {
                    dep_and_rdep_at_depths_to_visit.push(DepAndRdepAtDepth {
                        dep_and_rdep: DepAndRdep {
                            dep: Some(rdep.clone()),
                            rdep: rdep_of_rdep,
                        },
                        rdep_depth: depth_of_rdep_of_rdep,
                    });
                }
            }
        }
        Ok(Visit {
            keys_to_use_for_result: unique_valid_rdeps,
            keys_to_visit: dep_and_rdep_at_depths_to_visit,
        })
    }

    // Java: com/google/devtools/build/lib/query2/RdepsBoundedVisitor.java:164 RdepsBoundedVisitor.preprocessInitialVisit
    fn preprocess_initial_visit(&self, sky_keys: Vec<SkyKey>) -> Vec<DepAndRdepAtDepth> {
        sky_keys
            .into_iter()
            .map(|key| DepAndRdepAtDepth {
                dep_and_rdep: DepAndRdep {
                    dep: None,
                    rdep: key,
                },
                rdep_depth: 0,
            })
            .collect()
    }

    // Java: com/google/devtools/build/lib/query2/RdepsBoundedVisitor.java:153 RdepsBoundedVisitor.noteAndReturnUniqueVisitationKeys
    fn note_and_return_unique_visitation_keys(
        &self,
        prospective_visitation_keys: Vec<DepAndRdepAtDepth>,
    ) -> Result<Vec<DepAndRdepAtDepth>, QueryError> {
        Ok(prospective_visitation_keys
            .into_iter()
            .filter(|d| {
                self.valid_rdep_min_depth_uniquifier
                    .unique_at_depth_less_than_or_equal_to_pure(&d.dep_and_rdep.rdep, d.rdep_depth)
            })
            .collect())
    }

    fn get_visit_tasks(
        &self,
        pending: Vec<DepAndRdepAtDepth>,
    ) -> Result<Vec<Vec<DepAndRdepAtDepth>>, QueryError> {
        // Java: com/google/devtools/build/lib/query2/RdepsBoundedVisitor.java:148 RdepsBoundedVisitor.visitationKeyToOutputKey
        Ok(target_outputting_get_visit_tasks(pending, |k| {
            Some(k.dep_and_rdep.rdep.clone())
        }))
    }

    fn process(&self, partial_result: Vec<Target>) -> Result<(), QueryError> {
        self.callback.process(&partial_result)
    }
}

// ---------------------------------------------------------------------------------------------
// UnfilteredSkyKeyLabelDTCVisitor

/// A visitor that collects the label keys of the transitive closure, without edge filtering.
// Java: com/google/devtools/build/lib/query2/UnfilteredSkyKeyLabelDTCVisitor.java:27 UnfilteredSkyKeyLabelDTCVisitor
// PORT: skipped, the visitor is a struct literal made where Java calls its factory: UnfilteredSkyKeyLabelDTCVisitor.<init>, UnfilteredSkyKeyLabelDTCVisitor.Factory.<init>, UnfilteredSkyKeyLabelDTCVisitor.Factory.create
// Java: com/google/devtools/build/lib/query2/AbstractUnfilteredLabelDTCVisitor.java:28 AbstractUnfilteredLabelDTCVisitor
// Java: com/google/devtools/build/lib/query2/AbstractSkyKeyParallelVisitor.java:27 AbstractSkyKeyParallelVisitor
struct UnfilteredSkyKeyLabelDTCVisitor<'a> {
    env: &'a SkyQueryEnvironment,
    uniquifier: &'a dyn Uniquifier<SkyKey>,
    process_results_batch_size: usize,
    callback: &'a dyn Fn(Vec<SkyKey>),
}

impl ParallelVisitor for UnfilteredSkyKeyLabelDTCVisitor<'_> {
    type Input = SkyKey;
    type VisitKey = SkyKey;
    type OutputKey = SkyKey;
    type OutputResult = SkyKey;
    type Error = QueryError;

    fn visit_batch_size(&self) -> usize {
        VISIT_BATCH_SIZE
    }

    fn process_results_batch_size(&self) -> usize {
        self.process_results_batch_size
    }

    // Java: com/google/devtools/build/lib/query2/UnfilteredSkyKeyLabelDTCVisitor.java:38 UnfilteredSkyKeyLabelDTCVisitor.outputKeysToOutputValues
    fn output_keys_to_output_values(
        &self,
        target_keys: Vec<SkyKey>,
    ) -> Result<Vec<SkyKey>, QueryError> {
        Ok(target_keys)
    }

    // Java: com/google/devtools/build/lib/query2/AbstractUnfilteredLabelDTCVisitor.java:51 AbstractUnfilteredLabelDTCVisitor.getVisitResult
    fn get_visit_result(
        &self,
        label_keys: Vec<SkyKey>,
    ) -> Result<Visit<SkyKey, SkyKey>, QueryError> {
        let deps_map = self.env.get_fwd_dep_labels(&label_keys);
        Ok(Visit {
            keys_to_use_for_result: label_keys,
            keys_to_visit: deps_map.into_iter().flat_map(|(_, v)| v).collect(),
        })
    }

    // Java: com/google/devtools/build/lib/query2/AbstractUnfilteredLabelDTCVisitor.java:58 AbstractUnfilteredLabelDTCVisitor.preprocessInitialVisit
    fn preprocess_initial_visit(&self, visitation_keys: Vec<SkyKey>) -> Vec<SkyKey> {
        blaze_util::check_state!(
            visitation_keys.iter().all(is_label),
            "{:?}",
            visitation_keys
        );
        visitation_keys
    }

    // Java: com/google/devtools/build/lib/query2/AbstractSkyKeyParallelVisitor.java:42 AbstractSkyKeyParallelVisitor.noteAndReturnUniqueVisitationKeys
    fn note_and_return_unique_visitation_keys(
        &self,
        prospective_visitation_keys: Vec<SkyKey>,
    ) -> Result<Vec<SkyKey>, QueryError> {
        Ok(self.uniquifier.unique_all(&prospective_visitation_keys)?)
    }

    fn process(&self, partial_result: Vec<SkyKey>) -> Result<(), QueryError> {
        (self.callback)(partial_result);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dep_and_rdep_equality_uses_dep_and_rdep() {
        let a = SkyKey::new(
            blaze_sky_loading::transitive_traversal::TransitiveTraversalKey::key(
                blaze_cmdline::Label::parse_canonical_unchecked("//a:a"),
            ),
        );
        let b = SkyKey::new(
            blaze_sky_loading::transitive_traversal::TransitiveTraversalKey::key(
                blaze_cmdline::Label::parse_canonical_unchecked("//b:b"),
            ),
        );
        assert_eq!(
            DepAndRdep {
                dep: Some(a.clone()),
                rdep: b.clone()
            },
            DepAndRdep {
                dep: Some(a.clone()),
                rdep: b.clone()
            }
        );
        assert_ne!(
            DepAndRdep {
                dep: None,
                rdep: b.clone()
            },
            DepAndRdep {
                dep: Some(a),
                rdep: b
            }
        );
    }
}
