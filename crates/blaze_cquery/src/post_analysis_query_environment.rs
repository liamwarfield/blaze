//! Port of `query2/PostAnalysisQueryEnvironment.java` (with the generic parts of
//! `query2/common/AbstractBlazeQueryEnvironment.java` it inherits), `query2/SkyQueryUtils.java`,
//! `query2/NamedThreadSafeOutputFormatterCallback.java`,
//! `skyframe/GraphBackedRecursivePackageProvider.java` and
//! `skyframe/RecursivePkgValueRootPackageExtractor.java`.
//!
//! PORT: Java's abstract class `PostAnalysisQueryEnvironment<T>` with the subclasses
//! `ConfiguredTargetQueryEnvironment` (`T = CqueryNode`) and `ActionGraphQueryEnvironment`
//! (`T = ConfiguredTargetValue`) is one generic struct; the per-node-type methods are
//! [`PostAnalysisNode`] (both subclasses implement them by unwrapping the configured target) and
//! the `TargetAccessor<T>` implementations in `cquery` and `aquery`. blaze_query's
//! `AbstractBlazeQueryEnvironment` is specialized to `Target`; its generic parts used here
//! (`evaluateQueryInternal`, `eval`, `handleError`, `transitiveLoadFiles`) are ported generically
//! in this file.
//!
//! PORT: java-hash-order. The order of cquery results and of aquery actions (and so every id in
//! aquery's output) is the iteration order of Java hash collections keyed by
//! `ConfiguredTargetKey`; it is replicated here (validated against the oracle's output of both
//! corpus projects, see `tests/java_order_test.rs`):
//!
//! - Result sets, the `deps()` frontier and `getFwdDeps` results are
//!   `ThreadSafeMutableKeyExtractorBackedSetImpl`s over
//!   `new ConcurrentHashMap<>(SkyQueryEnvironment.DEFAULT_THREAD_COUNT, 0.75f)`: [`KeyedNodeSet`]
//!   over `blaze_query_engine::JavaConcurrentHashMap`. `DEFAULT_THREAD_COUNT` is
//!   `Runtime.getRuntime().availableProcessors()`, so Bazel's output order depends on the number
//!   of processors of the machine ([`default_thread_count`]).
//! - `getFwdDeps` / `getReverseDeps` pass their keys through four `java.util.HashMap`s
//!   (`Maps.newHashMapWithExpectedSize`, `InMemoryGraphImpl.getBatchMap`,
//!   `DelegatingWalkableGraph.getDirectDeps`, `targetifyValues`): [`java_hash_map_key_order`],
//!   [`delegating_walkable_graph_map_order`].
//! - The hash codes are Java's `ConfiguredTargetKey.hashCode()`
//!   ([`configured_target_key_java_hash_code`]) and, for cquery's aspect nodes and the `ASPECT`
//!   keys among reverse dependencies, `AspectKey.hashCode()` ([`aspect_key_java_hash_code`]),
//!   which mixes in an identity hash code Bazel does not fix ([`java_class_identity_hash_code`]).
//! - Within one bin the order is the insertion order, which comes from the order of a node's
//!   direct deps in the Skyframe graph (`WalkableGraph.getDirectDeps`, Java's `GroupedDeps`
//!   order, i.e. the order in which `ConfiguredTargetFunction` and its state machines requested
//!   them).
//!
//! Not replicable: `getReverseDeps` iterates a `HashMap` keyed by the node objects (identity hash
//! codes), and a node's reverse deps are in the order the parents registered. Bazel's own
//! `rdeps()`/`allpaths()` order over several targets changes from run to run (recorded twice in
//! `scripts/w4/scenarios/query-order`); those two places keep insertion order here.

use std::cell::Cell;
use std::collections::{HashMap, HashSet};
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::{Arc, Mutex, OnceLock};

use blaze_actions::ActionLookupKeyRef;
use blaze_analysis::aspect_key_creator::AspectKey;
use blaze_analysis::aspect_value::AspectValue;
use blaze_analysis::configured_aspect::ConfiguredAspect;
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::configured_target_key::{CONFIGURED_TARGET, ConfiguredTargetKey};
use blaze_analysis::configured_target_value::ConfiguredTargetValue;
use blaze_analysis::configuredtargets::RuleConfiguredTarget;
use blaze_analysis::providers::alias_provider::AliasProvider;
use blaze_analysis::toolchains::toolchain_context_key::ToolchainContextKey;
use blaze_build_config::build_configuration_key::BuildConfigurationKey;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_cmdline::{
    Label, PackageIdentifier, RepositoryName, TargetParsingException, TargetPatternParser,
};
use blaze_events::{Event, EventHandler, ExtendedEventHandler};
use blaze_packages::aspect::{
    AspectClass, AspectDescriptor, AspectParameters, StarlarkAspectClass,
};
use blaze_packages::exceptions::NoSuchPackageException;
use blaze_packages::package::Package;
use blaze_packages::target::{LabelPrinter, Target};
use blaze_protos::failure_details::{
    FailureDetail, Query, configurable_query, failure_detail::Category, query,
};
use blaze_query::common::abstract_blaze_query_environment::{
    EvaluateQueryError, QueryErrorSensingEventHandler,
};
use blaze_query::compat::failure_details;
use blaze_query_engine::callback::{
    IoException, IoExceptionSlot, OutputFormatterError, output_formatter_callback_process,
};
use blaze_query_engine::java_concurrent_hash_map::{
    JavaConcurrentHashMap, JavaHashCode, java_hash_map_order_by, java_hash_map_table_size_for,
    java_internal_string_hash_code,
};
use blaze_query_engine::query_util::{self, MinDepthUniquifierImpl, UniquifierImpl};
use blaze_query_engine::{
    Callback, DetailedExitCode, EvaluateExpression, InterruptedException, KeyExtractor,
    MinDepthUniquifier, OutputFormatterCallback, QueryEnvironment, QueryEnvironmentFutures,
    QueryError, QueryEvalResult, QueryException, QueryExpression, QueryExpressionContext,
    QueryFunction, QueryTaskFuture, Setting, TargetAccessor, TargetNotFoundException,
    ThreadSafeMutableSet, ThreadSafeOutputFormatterCallback, TransitiveLoadFilesHelper, Uniquifier,
    default_query_functions,
};
use blaze_sky_loading::ignored_subdirectories::{IgnoredSubdirectories, IgnoredSubdirectoriesKey};
use blaze_sky_loading::package_function::{PackageKey, PackageValue};
use blaze_sky_loading::package_lookup::{PackageLookupKey, PackageLookupValue};
use blaze_sky_loading::pkgcache::PathPackageLocator;
use blaze_sky_loading::recursive_pkg::{RecursivePkgKey, RecursivePkgValue};
use blaze_sky_loading::repos::{RepositoryDirectoryKey, RepositoryDirectoryValue};
use blaze_sky_loading::target_pattern_eval::{self, TargetPatternEvalError};
use blaze_sky_loading::target_pattern_function::{
    FilteringPolicy, ProviderError, RecursivePackageProvider,
    RecursivePackageProviderBackedTargetPatternResolver,
};
use blaze_skyframe::{SkyKey, WalkableGraph};
use blaze_vfs::{PathFragment, Root, RootedPath};
use indexmap::{IndexMap, IndexSet};

static DEFAULT_THREAD_COUNT: OnceLock<i32> = OnceLock::new();

/// Sets `SkyQueryEnvironment.DEFAULT_THREAD_COUNT` to `Runtime.getRuntime().availableProcessors()`
/// (Java: a static initializer). The value belongs to the process: the first call stands.
///
/// PORT: the exact port of `Runtime.availableProcessors()` (HotSpot's
/// `os::active_processor_count`: the CPUs of the affinity mask, lowered to the cgroup CPU quota
/// rounded up) is `blaze_runtime::local_resources::available_processors`, in a crate above this
/// one; the cquery and aquery commands pass it in (`PostAnalysisCommandEnvironment
/// .available_processors`). `std::thread::available_parallelism` rounds a fractional quota down,
/// so it is not used.
// Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:154 SkyQueryEnvironment.DEFAULT_THREAD_COUNT
pub fn set_default_thread_count(available_processors: i32) {
    let _ = DEFAULT_THREAD_COUNT.set(available_processors);
}

/// `SkyQueryEnvironment.DEFAULT_THREAD_COUNT`: `Runtime.getRuntime().availableProcessors()`.
///
/// It sizes the `ConcurrentHashMap`s behind every post-analysis query set, so it decides the
/// order of cquery and aquery output (see the module documentation).
// Java: com/google/devtools/build/lib/query2/SkyQueryEnvironment.java:154 SkyQueryEnvironment.DEFAULT_THREAD_COUNT
pub fn default_thread_count() -> i32 {
    *DEFAULT_THREAD_COUNT.get().unwrap_or_else(|| {
        panic!(
            "IllegalStateException: SkyQueryEnvironment.DEFAULT_THREAD_COUNT is not set \
             (set_default_thread_count(Runtime.availableProcessors()) runs at the start of the \
             cquery and aquery commands)"
        )
    })
}

// -------------------------------------------------------------------------------------------
// Node types.

/// The node types of post-analysis queries: `CqueryNode` (a `ConfiguredTarget` or, with
/// `--experimental_explicit_aspects`, an `AspectKey`) for cquery and `ConfiguredTargetValue` for
/// aquery.
///
/// PORT: the abstract methods of `PostAnalysisQueryEnvironment<T>` that depend on `T` and the
/// `CqueryNode` defaults the shared code reads.
pub trait PostAnalysisNode: Clone + 'static {
    /// Java `getConfiguredTargetKey(T)` (`CqueryNode.getLookupKey()` for cquery).
    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:679 PostAnalysisQueryEnvironment.getConfiguredTargetKey
    fn get_lookup_key(&self) -> ActionLookupKeyRef;

    /// The key of the node's configuration (what `getConfiguration(T)` looks up).
    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:677 PostAnalysisQueryEnvironment.getConfiguration
    fn get_configuration_key(&self) -> Option<BuildConfigurationKey>;

    /// Java `getCorrectLabel(T)`: the original label (aliases are not dereferenced).
    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:238 PostAnalysisQueryEnvironment.getCorrectLabel
    fn get_original_label(&self) -> Label;

    /// The configured target behind the node; `None` for a node that is no configured target
    /// (cquery's `AspectKey` nodes). Java tests the node with `instanceof` in
    /// `getRuleConfiguredTarget`, `getOwningRuleforOutputConfiguredTarget` and
    /// `isAliasConfiguredTarget`.
    fn as_configured_target(&self) -> Option<&ConfiguredTarget>;

    /// Java `getValueFromKey(SkyKey)`: the node of a key in the graph, `None` if the graph has no
    /// value for it.
    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:276 PostAnalysisQueryEnvironment.getValueFromKey
    fn get_value_from_key(graph: &dyn WalkableGraph, key: &SkyKey) -> Option<Self>;

    /// Java `node instanceof RuleConfiguredTarget`, on the node object itself: true for a rule's
    /// `CqueryNode`, never true for aquery's `ConfiguredTargetValue`.
    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:357 PostAnalysisQueryEnvironment.filterReverseDeps
    fn is_instance_of_rule_configured_target(&self) -> bool;
}

impl PostAnalysisNode for Arc<ConfiguredTargetValue> {
    // Java: com/google/devtools/build/lib/query2/aquery/ActionGraphQueryEnvironment.java:319 ActionGraphQueryEnvironment.getConfiguredTargetKey
    fn get_lookup_key(&self) -> ActionLookupKeyRef {
        ConfiguredTargetValue::get_configured_target(self)
            .get_lookup_key()
            .clone()
    }

    // Java: com/google/devtools/build/lib/query2/aquery/ActionGraphQueryEnvironment.java:307 ActionGraphQueryEnvironment.getConfiguration
    fn get_configuration_key(&self) -> Option<BuildConfigurationKey> {
        ConfiguredTargetValue::get_configured_target(self).get_configuration_key()
    }

    // Java: com/google/devtools/build/lib/query2/aquery/ActionGraphQueryEnvironment.java:217 ActionGraphQueryEnvironment.getCorrectLabel
    fn get_original_label(&self) -> Label {
        ConfiguredTargetValue::get_configured_target(self).get_original_label()
    }

    fn as_configured_target(&self) -> Option<&ConfiguredTarget> {
        Some(ConfiguredTargetValue::get_configured_target(self))
    }

    // Java: com/google/devtools/build/lib/query2/aquery/ActionGraphQueryEnvironment.java:273 ActionGraphQueryEnvironment.getValueFromKey
    fn get_value_from_key(graph: &dyn WalkableGraph, key: &SkyKey) -> Option<Self> {
        blaze_util::check_state!(key.downcast::<ConfiguredTargetKey>().is_some());
        blaze_sky_top::skyframe_executor::get_configured_target_value(graph, key)
    }

    fn is_instance_of_rule_configured_target(&self) -> bool {
        // A ConfiguredTargetValue is not a RuleConfiguredTarget.
        false
    }
}

/// The `ActionLookupKey` of a node (Java `getConfiguredTargetKey(T)`).
pub fn get_configured_target_key<T: PostAnalysisNode>(target: &T) -> ActionLookupKeyRef {
    target.get_lookup_key()
}

/// The `ConfiguredTargetKey` behind a lookup key (Java `ConfiguredTargetKey.fromConfiguredTarget`).
pub fn as_configured_target_key(key: &ActionLookupKeyRef) -> Option<&ConfiguredTargetKey> {
    key.0.as_any().downcast_ref::<ConfiguredTargetKey>()
}

/// The `AspectKey` behind a lookup key (cquery's aspect nodes).
pub fn as_aspect_key(key: &ActionLookupKeyRef) -> Option<&AspectKey> {
    key.0.as_any().downcast_ref::<AspectKey>()
}

/// The SkyKey of a lookup key (Java: the `ActionLookupKey` is the `SkyKey`).
pub fn lookup_key_to_sky_key(key: &ActionLookupKeyRef) -> SkyKey {
    if let Some(k) = as_configured_target_key(key) {
        return SkyKey::new(k.clone());
    }
    if let Some(k) = as_aspect_key(key) {
        return SkyKey::new(k.clone());
    }
    panic!(
        "IllegalStateException: {} is neither a ConfiguredTargetKey nor an AspectKey",
        key
    )
}

/// A `KeyExtractor` over [`get_configured_target_key`].
pub fn configured_target_key_extractor<T: PostAnalysisNode>()
-> Arc<dyn KeyExtractor<T, ActionLookupKeyRef>> {
    Arc::new(|t: &T| get_configured_target_key(t))
}

/// `(AspectValue) graph.getValue(key)`.
pub fn get_aspect_value(graph: &dyn WalkableGraph, key: &SkyKey) -> Option<Arc<AspectValue>> {
    graph.get_value(key).map(|v| {
        v.downcast::<AspectValue>()
            .unwrap_or_else(|_| panic!("ClassCastException: ASPECT value of {:?}", key))
    })
}

// -------------------------------------------------------------------------------------------
// Java hash codes and hash collection orders (PORT: java-hash-order, see the module
// documentation).

/// `HashCodes.hashObjects(o1, o2, o3)` over the hash codes of its arguments (0 for null).
// Java: com/google/devtools/build/lib/util/HashCodes.java:52 HashCodes.hashObjects
fn hash_objects3(h1: i32, h2: i32, h3: i32) -> i32 {
    // h3 + MULTIPLIER * (h2 + (MULTIPLIER * (h1 + MULTIPLIER)))
    h3.wrapping_add(31i32.wrapping_mul(h2.wrapping_add(31i32.wrapping_mul(h1.wrapping_add(31)))))
}

/// `BuildConfigurationKey.hashCode()`: `options.hashCode()`, which is
/// `31 + checksum().hashCode()`.
// Java: com/google/devtools/build/lib/analysis/config/BuildOptions.java:284 BuildOptions.hashCode
fn build_configuration_key_java_hash_code(checksum: &str) -> i32 {
    31i32.wrapping_add(java_internal_string_hash_code(checksum))
}

/// Java `ConfiguredTargetKey.hashCode()` of the key with these fields
/// (`configuration_checksum` is `configurationKey.getOptionsChecksum()`, `None` for a null
/// configuration key).
// Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetKey.java:320 ConfiguredTargetKey.computeHashCode
pub fn configured_target_key_java_hash_code(
    label: &Label,
    configuration_checksum: Option<&str>,
    execution_platform_label: Option<&Label>,
    should_apply_rule_transition: bool,
) -> i32 {
    // int hashCode = hashObjects(label, configurationKey, executionPlatformLabel);
    let mut hash_code = hash_objects3(
        label.java_hash_code(),
        configuration_checksum.map_or(0, build_configuration_key_java_hash_code),
        execution_platform_label.map_or(0, |l| l.java_hash_code()),
    );
    if !should_apply_rule_transition {
        hash_code = !hash_code;
    }
    hash_code
}

/// Java `hashCode()` of a `ConfiguredTargetKey`.
// Java: com/google/devtools/build/lib/skyframe/ConfiguredTargetKey.java:122 ConfiguredTargetKey.hashCode
fn java_hash_code_of_configured_target_key(key: &ConfiguredTargetKey) -> i32 {
    configured_target_key_java_hash_code(
        key.get_label(),
        key.get_configuration_key()
            .map(|k| k.get_options_checksum()),
        key.get_execution_platform_label(),
        key.should_apply_rule_transition(),
    )
}

/// Java `hashCode()` of an `ActionLookupKey` that is a post-analysis query node key (a
/// `ConfiguredTargetKey`, or an `AspectKey` for cquery's explicit aspect nodes).
pub fn action_lookup_key_java_hash_code(key: &ActionLookupKeyRef) -> i32 {
    if let Some(k) = as_configured_target_key(key) {
        return java_hash_code_of_configured_target_key(k);
    }
    if let Some(k) = as_aspect_key(key) {
        return aspect_key_java_hash_code(k, &java_class_identity_hash_code);
    }
    panic!(
        "IllegalStateException: {} is neither a ConfiguredTargetKey nor an AspectKey",
        key
    )
}

/// `HashCodes.hashObjects(o1, o2)` over the hash codes of its arguments (0 for null).
// Java: com/google/devtools/build/lib/util/HashCodes.java:46 HashCodes.hashObjects
fn hash_objects2(h1: i32, h2: i32) -> i32 {
    // h2 + (MULTIPLIER * (h1 + MULTIPLIER))
    h2.wrapping_add(31i32.wrapping_mul(h1.wrapping_add(31)))
}

/// `List.hashCode()` (`ImmutableList`, `Lists.hashCodeImpl`) over the hash codes of the
/// elements.
// Java (JDK): java/util/List.java List.hashCode
fn java_list_hash_code(element_hash_codes: impl IntoIterator<Item = i32>) -> i32 {
    let mut hash_code: i32 = 1;
    for e in element_hash_codes {
        hash_code = hash_code.wrapping_mul(31).wrapping_add(e);
    }
    hash_code
}

/// The subclasses of `BzlLoadValue.Key` a `StarlarkAspectClass` can hold
/// (`StarlarkDefinedAspect.export` takes the key of the exporting `.bzl`; `BuildView` and
/// `StarlarkAspectClass.getAspectClassFromName` use `keyForBuild`).
// Java: com/google/devtools/build/lib/skyframe/BzlLoadValue.java:107 BzlLoadValue.Key
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum BzlLoadKeyClass {
    /// `BzlLoadValue.KeyForBuild` (a `.bzl` loaded on behalf of a BUILD file).
    KeyForBuild,
    /// `BzlLoadValue.KeyForBuiltins` (a `.bzl` of `@_builtins`).
    KeyForBuiltins,
}

/// The `BzlLoadValue.Key` class of the extension key of a `StarlarkAspectClass`.
///
/// PORT: `blaze_packages::aspect::StarlarkAspectClass` keeps the label of its extension key, not
/// the key. Labels in `@_builtins` are loaded with `keyForBuiltins`
/// (`BzlLoadValue.java:349`), every other aspect a build reaches through `keyForBuild`
/// (`BzlLoadValue.java:344`; `KeyForBzlmod` keys are for `.bzl` files loaded by MODULE.bazel and
/// its extensions, whose aspects no target can use).
fn bzl_load_key_class_of(starlark_aspect_class: &StarlarkAspectClass) -> BzlLoadKeyClass {
    if starlark_aspect_class
        .get_extension_label()
        .get_repository()
        .get_name()
        == "_builtins"
    {
        BzlLoadKeyClass::KeyForBuiltins
    } else {
        BzlLoadKeyClass::KeyForBuild
    }
}

/// `Object.hashCode()` of the `java.lang.Class` object of a `BzlLoadValue.Key` subclass: its
/// identity hash code, which `BzlLoadValue.Key.hashCode()` mixes in
/// (`HashCodes.hashObjects(getClass(), getLabel())`).
///
/// PORT: java-hash-order, not replicable. An identity hash code is drawn by the JVM when it is
/// first asked for (HotSpot: a thread-local xorshift generator), so it is fixed for one Bazel
/// server and differs between servers: every Java hash collection that holds an `AspectKey` of a
/// Starlark aspect iterates in an order Bazel itself does not reproduce (cquery with
/// `--experimental_explicit_aspects`; `rdeps` through delegating configured targets). Blaze takes
/// one fixed value per class, 0xBAD and 0xBAE (0xBAD is a value HotSpot's generator itself yields,
/// in place of 0), so its order is one of the orders a Bazel server can print. The formulas
/// around the value are Java's.
// Java (JDK): java/lang/Object.java Object.hashCode
pub fn java_class_identity_hash_code(class: BzlLoadKeyClass) -> i32 {
    match class {
        BzlLoadKeyClass::KeyForBuild => 0xBAD,
        BzlLoadKeyClass::KeyForBuiltins => 0xBAE,
    }
}

/// `BzlLoadValue.Key.hashCode()` of a key of class `class` for `label`, given the identity hash
/// code of that class.
// Java: com/google/devtools/build/lib/skyframe/BzlLoadValue.java:190 BzlLoadValue.Key.hashCode
pub fn bzl_load_key_java_hash_code(
    class: BzlLoadKeyClass,
    label: &Label,
    class_identity_hash_code: &dyn Fn(BzlLoadKeyClass) -> i32,
) -> i32 {
    // KeyForBuild.isBuildPrelude() is false for the keys of exported aspects (keyForBuild);
    // KeyForBuiltins.isBuildPrelude() is the default false.
    let is_build_prelude = false;
    let is_builtins = class == BzlLoadKeyClass::KeyForBuiltins;
    // int result = HashCodes.hashObjects(getClass(), getLabel());
    let mut result = hash_objects2(class_identity_hash_code(class), label.java_hash_code());
    // result = 31 * result + Boolean.hashCode(isBuildPrelude());
    result = result
        .wrapping_mul(31)
        .wrapping_add(java_boolean_hash_code(is_build_prelude));
    // result = 31 * result + Boolean.hashCode(isBuiltins());
    result
        .wrapping_mul(31)
        .wrapping_add(java_boolean_hash_code(is_builtins))
}

/// `StarlarkAspectClass.hashCode()`: `hashObjects(extensionKey, exportedName)`.
// Java: com/google/devtools/build/lib/packages/StarlarkAspectClass.java:69 StarlarkAspectClass.hashCode
pub fn starlark_aspect_class_java_hash_code(
    aspect_class: &StarlarkAspectClass,
    class_identity_hash_code: &dyn Fn(BzlLoadKeyClass) -> i32,
) -> i32 {
    hash_objects2(
        bzl_load_key_java_hash_code(
            bzl_load_key_class_of(aspect_class),
            aspect_class.get_extension_label(),
            class_identity_hash_code,
        ),
        // PORT: StringEncoding identity (the exported name is a Java internal-form string).
        java_internal_string_hash_code(aspect_class.get_exported_name()),
    )
}

/// `AspectClass.hashCode()`.
pub fn aspect_class_java_hash_code(
    aspect_class: &AspectClass,
    class_identity_hash_code: &dyn Fn(BzlLoadKeyClass) -> i32,
) -> i32 {
    match aspect_class {
        AspectClass::Starlark(s) => {
            starlark_aspect_class_java_hash_code(s, class_identity_hash_code)
        }
        // NativeAspectClass implementations do not override Object.hashCode(): the identity hash
        // code of the singleton.
        AspectClass::Native(n) => blaze_util::unported!(
            "NativeAspectClass.hashCode",
            "identity hash code of the native aspect class {}",
            n.get_name()
        ),
    }
}

/// `AspectParameters.hashCode()`: `Objects.hashCode(attributes)`, the hash code of an
/// `ImmutableListMultimap<String, String>`, which is `asMap().hashCode()`: the sum over the keys
/// of `key.hashCode() ^ values.hashCode()` (`Map.Entry.hashCode()`), the values being an
/// `ImmutableList<String>` (`List.hashCode()`). The strings are Java internal-form strings
/// (attribute names and values), so `String.hashCode()` runs over their UTF-8 bytes.
// Java: com/google/devtools/build/lib/packages/AspectParameters.java:118 AspectParameters.hashCode
pub fn aspect_parameters_java_hash_code(parameters: &AspectParameters) -> i32 {
    let mut hash_code: i32 = 0;
    for (key, values) in parameters.get_attributes().iter() {
        let values_hash_code =
            java_list_hash_code(values.iter().map(|v| java_internal_string_hash_code(v)));
        hash_code = hash_code.wrapping_add(java_internal_string_hash_code(key) ^ values_hash_code);
    }
    hash_code
}

/// `AspectDescriptor.hashCode()`: `HashCodes.hashObjects(aspectClass, aspectParameters)`.
// Java: com/google/devtools/build/lib/packages/AspectDescriptor.java:63 AspectDescriptor.hashCode
pub fn aspect_descriptor_java_hash_code(
    descriptor: &AspectDescriptor,
    class_identity_hash_code: &dyn Fn(BzlLoadKeyClass) -> i32,
) -> i32 {
    hash_objects2(
        aspect_class_java_hash_code(descriptor.get_aspect_class(), class_identity_hash_code),
        aspect_parameters_java_hash_code(descriptor.get_parameters()),
    )
}

/// `AspectKey.hashCode()`: the `hashCode` computed by `AspectKey.createAspectKey` and cached in
/// `AspectBaseKey`: `hashObjects(baseConfiguredTargetKey, aspectDescriptor)` for a
/// `SimpleAspectKey`, `hashObjects(baseConfiguredTargetKey, sortedBaseKeys, aspectDescriptor)`
/// for an `AspectKeyWithBaseAspects` (the base keys are kept sorted, as `getBaseKeys()` returns
/// them).
// Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:116 AspectKeyCreator.AspectKey.createAspectKey
// Java: com/google/devtools/build/lib/skyframe/AspectKeyCreator.java:85 AspectKeyCreator.AspectBaseKey.hashCode
pub fn aspect_key_java_hash_code(
    key: &AspectKey,
    class_identity_hash_code: &dyn Fn(BzlLoadKeyClass) -> i32,
) -> i32 {
    let base_configured_target_key =
        java_hash_code_of_configured_target_key(key.get_base_configured_target_key());
    let aspect_descriptor =
        aspect_descriptor_java_hash_code(key.get_aspect_descriptor(), class_identity_hash_code);
    if key.get_base_keys().is_empty() {
        return hash_objects2(base_configured_target_key, aspect_descriptor);
    }
    let sorted_base_keys = java_list_hash_code(
        key.get_base_keys()
            .iter()
            .map(|k| aspect_key_java_hash_code(k, class_identity_hash_code)),
    );
    hash_objects3(
        base_configured_target_key,
        sorted_base_keys,
        aspect_descriptor,
    )
}

/// `java.lang.Boolean.hashCode(boolean)`.
fn java_boolean_hash_code(b: bool) -> i32 {
    if b { 1231 } else { 1237 }
}

/// Java `hashCode()` of a `ToolchainContextKey` (an `@AutoValue` class: `h = 1`, then for each
/// property in declaration order `h *= 1000003; h ^= property.hashCode()`).
///
/// `ImmutableSet.hashCode()` is the sum of the element hash codes, `Optional.hashCode()` is the
/// value's hash code or 0, and the record `ToolchainTypeRequirement` hashes as
/// `(31 * (31 * toolchainType.hashCode() + hash(mandatory)) + hash(ignoreIfInvalid))`
/// (`java.lang.runtime.ObjectMethods`).
// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainContextKey.java:30 ToolchainContextKey
fn java_hash_code_of_toolchain_context_key(key: &ToolchainContextKey) -> i32 {
    let mut toolchain_types: i32 = 0;
    for requirement in key.toolchain_types() {
        let mut record: i32 = 0;
        record = record
            .wrapping_mul(31)
            .wrapping_add(requirement.toolchain_type.java_hash_code());
        record = record
            .wrapping_mul(31)
            .wrapping_add(java_boolean_hash_code(requirement.mandatory));
        record = record
            .wrapping_mul(31)
            .wrapping_add(java_boolean_hash_code(requirement.ignore_if_invalid));
        toolchain_types = toolchain_types.wrapping_add(record);
    }
    let mut exec_constraint_labels: i32 = 0;
    for label in key.exec_constraint_labels() {
        exec_constraint_labels = exec_constraint_labels.wrapping_add(label.java_hash_code());
    }
    let mut h: i32 = 1;
    h = h.wrapping_mul(1000003);
    h ^= build_configuration_key_java_hash_code(key.configuration_key().get_options_checksum());
    h = h.wrapping_mul(1000003);
    h ^= toolchain_types;
    h = h.wrapping_mul(1000003);
    h ^= exec_constraint_labels;
    h = h.wrapping_mul(1000003);
    h ^= key
        .force_execution_platform()
        .map_or(0, |l| l.java_hash_code());
    h = h.wrapping_mul(1000003);
    h ^= java_boolean_hash_code(key.debug_target());
    h
}

/// Java `hashCode()` of the SkyKeys `PostAnalysisQueryEnvironment.targetifyValues` looks at
/// (`CONFIGURED_TARGET`, `ASPECT` and `TOOLCHAIN_RESOLUTION` keys), `None` for any other key.
fn sky_key_java_hash_code_if_known(key: &SkyKey) -> Option<i32> {
    if let Some(k) = key.downcast::<ConfiguredTargetKey>() {
        return Some(java_hash_code_of_configured_target_key(k));
    }
    if let Some(k) = key.downcast::<AspectKey>() {
        return Some(aspect_key_java_hash_code(k, &java_class_identity_hash_code));
    }
    if let Some(k) = key.downcast::<ToolchainContextKey>() {
        return Some(java_hash_code_of_toolchain_context_key(k));
    }
    None
}

/// Java `hashCode()` of the key of a query node as a SkyKey (a `ConfiguredTargetKey`, or an
/// `AspectKey` for cquery's explicit aspect nodes).
fn sky_key_java_hash_code(key: &SkyKey) -> i32 {
    if let Some(k) = key.downcast::<ConfiguredTargetKey>() {
        return java_hash_code_of_configured_target_key(k);
    }
    if let Some(k) = key.downcast::<AspectKey>() {
        return aspect_key_java_hash_code(k, &java_class_identity_hash_code);
    }
    blaze_util::unported!(
        "SkyKey.hashCode",
        "Java hash code of the query node key {:?}",
        key
    )
}

/// Guava's `Maps.capacity(expectedSize)` (33.5.0): the `initialCapacity`
/// `Maps.newHashMapWithExpectedSize(expectedSize)` passes to `new HashMap<>(int)`.
// Java (Guava): com/google/common/collect/Maps.java Maps.capacity
pub fn guava_maps_capacity(expected_size: usize) -> i32 {
    if expected_size < 3 {
        return expected_size as i32 + 1;
    }
    // Ints.MAX_POWER_OF_TWO
    if expected_size < (1usize << 30) {
        // (int) ceil(expectedSize / 0.75)
        return (expected_size as f64 / 0.75).ceil() as i32;
    }
    i32::MAX // any large value
}

/// The entries of a `java.util.HashMap` whose table starts with `table_size` slots, in
/// iteration order, after `put`ting `entries` in order.
pub fn java_hash_map_entry_order<K: PartialEq, V>(
    table_size: usize,
    entries: Vec<(K, V)>,
    hash_code: &impl Fn(&K) -> i32,
) -> Vec<(K, V)> {
    java_hash_map_order_by(table_size, entries, |k| hash_code(k))
}

/// The key set of [`java_hash_map_entry_order`].
pub fn java_hash_map_key_order<K: PartialEq>(
    table_size: usize,
    keys: Vec<K>,
    hash_code: &impl Fn(&K) -> i32,
) -> Vec<K> {
    java_hash_map_entry_order(
        table_size,
        keys.into_iter().map(|k| (k, ())).collect(),
        hash_code,
    )
    .into_iter()
    .map(|(k, ())| k)
    .collect()
}

/// The key order of `Maps.newHashMapWithExpectedSize(expected_size)` after `put`ting `keys` in
/// order (`targetsByKey` of `getFwdDeps` and `getReverseDeps`, where `expected_size` is
/// `Iterables.size(targets)`).
// Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:288 PostAnalysisQueryEnvironment.getFwdDeps
pub fn targets_by_key_order<K: PartialEq>(
    expected_size: usize,
    keys: Vec<K>,
    hash_code: &impl Fn(&K) -> i32,
) -> Vec<K> {
    java_hash_map_key_order(
        java_hash_map_table_size_for(guava_maps_capacity(expected_size)),
        keys,
        hash_code,
    )
}

/// The entry order of the map `DelegatingWalkableGraph.getDirectDeps(keys)` /
/// `getReverseDeps(keys)` returns, given the entries of the keys that have a (done) node, in the
/// order of `keys`.
// Java: com/google/devtools/build/skyframe/DelegatingWalkableGraph.java:109 DelegatingWalkableGraph.getDirectDeps
pub fn delegating_walkable_graph_map_order<K: PartialEq, V>(
    entries: Vec<(K, V)>,
    hash_code: &impl Fn(&K) -> i32,
) -> Vec<(K, V)> {
    // getBatch(null, reason, keys): InMemoryGraphImpl.getBatchMap puts the entries of the keys
    // into `new HashMap<>()`.
    // Java: com/google/devtools/build/skyframe/InMemoryGraphImpl.java:180 InMemoryGraphImpl.getBatchMap
    let entries = java_hash_map_entry_order(16, entries, hash_code);
    // Map<SkyKey, Iterable<SkyKey>> result = new HashMap<>(entries.size());
    let table_size = java_hash_map_table_size_for(entries.len() as i32);
    java_hash_map_entry_order(table_size, entries, hash_code)
}

/// The iteration order of `new HashSet<>()` after `add`ing `elements` in order, restricted to
/// the elements whose Java hash code is known (`Some`). The other elements only count for the
/// growth of the table.
///
/// `HashMap.resize` keeps the relative order of the nodes of a bin and `putVal` appends, so the
/// iteration order is the insertion order stably sorted by the bin index in the final table,
/// whose length depends only on the number of elements, unless a bin reaches 9 nodes
/// (`treeifyBin`: a resize below 64 slots, a red-black tree above). Whether that happens depends
/// on the hash codes of all elements, so this fails loudly as soon as the elements of unknown
/// hash code could make a bin that long.
// Java (JDK): java/util/HashMap.java HashMap.putVal
fn java_hash_set_order_of_known<K: PartialEq + Clone + std::fmt::Debug>(
    elements: Vec<(K, Option<i32>)>,
) -> Vec<K> {
    const TREEIFY_THRESHOLD: usize = 8;
    // HashMap.hash: (h = key.hashCode()) ^ (h >>> 16)
    fn hash(h: i32) -> usize {
        let h = h as u32;
        (h ^ (h >> 16)) as usize
    }
    let mut table_size: usize = 16;
    let mut size: usize = 0;
    let mut unknown: usize = 0;
    let mut seen: Vec<K> = Vec::new();
    let mut known: Vec<(K, usize)> = Vec::new();
    let mut bin_counts: Vec<usize> = vec![0; table_size];
    for (element, hash_code) in elements {
        // HashSet.add: an element already present changes nothing.
        if seen.contains(&element) {
            continue;
        }
        match hash_code {
            Some(h) => {
                let h = hash(h);
                let index = h & (table_size - 1);
                bin_counts[index] += 1;
                if bin_counts[index] + unknown > TREEIFY_THRESHOLD {
                    blaze_util::unported!(
                        "HashMap.treeifyBin",
                        "iteration order of a HashSet<SkyKey> in which {} keys share a bin with \
                         possibly {} keys of unported hashCode (adding {:?})",
                        bin_counts[index],
                        unknown,
                        element
                    );
                }
                seen.push(element.clone());
                known.push((element, h));
            }
            None => {
                unknown += 1;
                if let Some(max) = bin_counts.iter().max()
                    && max + unknown > TREEIFY_THRESHOLD
                {
                    blaze_util::unported!(
                        "HashMap.treeifyBin",
                        "iteration order of a HashSet<SkyKey> with {} keys of unported hashCode \
                         (adding {:?})",
                        unknown,
                        element
                    );
                }
                seen.push(element);
            }
        }
        // if (++size > threshold) resize();  (threshold is 0.75 * table length)
        size += 1;
        if size > table_size * 3 / 4 {
            table_size *= 2;
            bin_counts = vec![0; table_size];
            for (_, h) in &known {
                bin_counts[h & (table_size - 1)] += 1;
            }
        }
    }
    let mut bins: Vec<Vec<K>> = (0..table_size).map(|_| Vec::new()).collect();
    for (element, h) in known {
        bins[h & (table_size - 1)].push(element);
    }
    bins.into_iter().flatten().collect()
}

/// An `ActionLookupKey` with its Java `hashCode()` (the key type of the query sets).
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct JavaActionLookupKey(pub ActionLookupKeyRef);

impl JavaHashCode for JavaActionLookupKey {
    fn java_hash_code(&self) -> i32 {
        action_lookup_key_java_hash_code(&self.0)
    }
}

/// `ThreadSafeMutableKeyExtractorBackedSetImpl` keyed by the configured target key
/// (`createThreadSafeMutableSet` of both environments).
///
/// PORT: java-hash-order (see the module documentation): the backing
/// `new ConcurrentHashMap<>(/*initialCapacity=*/ concurrencyLevel, /*loadFactor=*/ 0.75f)` is
/// `blaze_query_engine::JavaConcurrentHashMap`, so iteration follows Java's order.
// Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:179 QueryUtil.ThreadSafeMutableKeyExtractorBackedSetImpl
pub struct KeyedNodeSet<T> {
    map: Mutex<JavaConcurrentHashMap<JavaActionLookupKey, T>>,
}

impl<T> KeyedNodeSet<T> {
    /// The set with `concurrencyLevel = SkyQueryEnvironment.DEFAULT_THREAD_COUNT`.
    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:565 ConfiguredTargetQueryEnvironment.createThreadSafeMutableSet
    pub fn new() -> Self {
        Self::with_concurrency_level(default_thread_count())
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:190 ThreadSafeMutableKeyExtractorBackedSetImpl.<init>
    pub fn with_concurrency_level(concurrency_level: i32) -> Self {
        KeyedNodeSet {
            map: Mutex::new(JavaConcurrentHashMap::with_capacity(
                concurrency_level,
                0.75,
                1,
            )),
        }
    }
}

impl<T: PostAnalysisNode> ThreadSafeMutableSet<T> for KeyedNodeSet<T> {
    // Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:211 ThreadSafeMutableKeyExtractorBackedSetImpl.add
    fn add(&self, element: T) -> bool {
        let key = JavaActionLookupKey(get_configured_target_key(&element));
        self.map.lock().unwrap().put_if_absent(key, element)
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:216 ThreadSafeMutableKeyExtractorBackedSetImpl.contains
    fn contains(&self, element: &T) -> bool {
        self.map
            .lock()
            .unwrap()
            .contains_key(&JavaActionLookupKey(get_configured_target_key(element)))
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:225 ThreadSafeMutableKeyExtractorBackedSetImpl.remove
    fn remove(&self, element: &T) -> bool {
        self.map
            .lock()
            .unwrap()
            .remove(&JavaActionLookupKey(get_configured_target_key(element)))
            .is_some()
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:206 ThreadSafeMutableKeyExtractorBackedSetImpl.size
    fn size(&self) -> i32 {
        self.map.lock().unwrap().len() as i32
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:201 ThreadSafeMutableKeyExtractorBackedSetImpl.iterator
    fn to_vec(&self) -> Vec<T> {
        self.map.lock().unwrap().values().cloned().collect()
    }
}

// -------------------------------------------------------------------------------------------
// TopLevelConfigurations.

/// A wrapper class for the set of top-level configurations in a query.
// Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:738 PostAnalysisQueryEnvironment.TopLevelConfigurations
pub struct TopLevelConfigurations {
    /// A map of non-null configured top-level targets sorted by configuration checksum.
    non_nulls: IndexMap<Label, Arc<BuildConfigurationValue>>,
    /// `nonNulls` may often have many duplicate values in its value set so we store a sorted set
    /// of all the non-null configurations here.
    non_null_configs: Vec<Arc<BuildConfigurationValue>>,
    /// A list of null configured top-level targets.
    nulls: Vec<Label>,
}

impl TopLevelConfigurations {
    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:752 TopLevelConfigurations.<init>
    pub fn new(
        top_level_targets_and_configurations: &[blaze_sky_top::skyframe_executor::TargetAndConfiguration],
    ) -> Self {
        let mut non_nulls: IndexMap<Label, Arc<BuildConfigurationValue>> = IndexMap::new();
        let mut nulls: Vec<Label> = Vec::new();
        for target_and_configuration in top_level_targets_and_configurations {
            match target_and_configuration.get_configuration() {
                None => nulls.push(target_and_configuration.get_label().clone()),
                Some(configuration) => {
                    let label = target_and_configuration.get_label().clone();
                    // ImmutableMap.Builder.buildOrThrow
                    if non_nulls
                        .insert(label.clone(), configuration.clone())
                        .is_some()
                    {
                        panic!("IllegalArgumentException: Multiple entries with same key: {label}");
                    }
                }
            }
        }
        // ImmutableSortedSet.copyOf(Comparator.comparing(checksum), nonNulls.values())
        let mut non_null_configs: Vec<Arc<BuildConfigurationValue>> = Vec::new();
        for config in non_nulls.values() {
            if !non_null_configs
                .iter()
                .any(|c| c.checksum() == config.checksum())
            {
                non_null_configs.push(config.clone());
            }
        }
        non_null_configs.sort_by(|a, b| a.checksum().cmp(b.checksum()));
        TopLevelConfigurations {
            non_nulls,
            non_null_configs,
            nulls,
        }
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:772 TopLevelConfigurations.isTopLevelTarget
    pub fn is_top_level_target(&self, label: &Label) -> bool {
        self.non_nulls.contains_key(label) || self.nulls.contains(label)
    }

    /// This method returns the configuration of a top-level target if it's not null-configured
    /// and otherwise returns `None` (signifying it is null configured).
    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:779 TopLevelConfigurations.getConfigurationForTopLevelTarget
    pub fn get_configuration_for_top_level_target(
        &self,
        label: &Label,
    ) -> Option<&Arc<BuildConfigurationValue>> {
        blaze_util::check_argument!(
            self.is_top_level_target(label),
            "Attempting to get top-level configuration for non-top-level target {}.",
            label
        );
        self.non_nulls.get(label)
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:787 TopLevelConfigurations.getConfigurations
    pub fn get_configurations(&self) -> Vec<Option<Arc<BuildConfigurationValue>>> {
        let mut result: Vec<Option<Arc<BuildConfigurationValue>>> =
            self.non_null_configs.iter().cloned().map(Some).collect();
        if !self.nulls.is_empty() {
            result.push(None);
        }
        result
    }
}

// -------------------------------------------------------------------------------------------
// NamedThreadSafeOutputFormatterCallback.

/// A `ThreadSafeOutputFormatterCallback` with a name (the `--output` value it implements).
// Java: com/google/devtools/build/lib/query2/NamedThreadSafeOutputFormatterCallback.java:25 NamedThreadSafeOutputFormatterCallback
pub trait NamedThreadSafeOutputFormatterCallback<T>: ThreadSafeOutputFormatterCallback<T> {
    fn get_name(&self) -> String;
}

// Java: com/google/devtools/build/lib/query2/NamedThreadSafeOutputFormatterCallback.java:30 NamedThreadSafeOutputFormatterCallback.callbackNames
pub fn callback_names<T>(
    callbacks: &[Box<dyn NamedThreadSafeOutputFormatterCallback<T> + '_>],
) -> String {
    callbacks
        .iter()
        .map(|c| c.get_name())
        .collect::<Vec<_>>()
        .join(", ")
}

/// Returns the index of the callback named `type_`, if any.
// Java: com/google/devtools/build/lib/query2/NamedThreadSafeOutputFormatterCallback.java:38 NamedThreadSafeOutputFormatterCallback.selectCallback
pub fn select_callback<T>(
    type_: &str,
    callbacks: &[Box<dyn NamedThreadSafeOutputFormatterCallback<T> + '_>],
) -> Option<usize> {
    callbacks.iter().position(|c| c.get_name() == type_)
}

// -------------------------------------------------------------------------------------------
// The environment.

/// A dependency with whether it is attached implicitly.
// Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:650 PostAnalysisQueryEnvironment.ClassifiedDependency
#[derive(Clone)]
struct ClassifiedDependency<T> {
    implicit: bool,
    dependency: T,
}

/// `QueryEnvironment` that runs queries based on results from the analysis phase.
///
/// This environment can theoretically be used for multiple queries, but currently is only ever
/// used for one over the course of its lifetime.
// Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:112 PostAnalysisQueryEnvironment
pub struct PostAnalysisQueryEnvironment<T> {
    // AbstractBlazeQueryEnvironment
    event_handler: Arc<QueryErrorSensingEventHandler>,
    event_handler_dyn: Arc<dyn ExtendedEventHandler>,
    keep_going: bool,
    /// Java `dependencyFilter == DependencyFilter.ALL_DEPS` (an identity comparison: the filter
    /// is ALL_DEPS exactly when no filtering setting is enabled).
    dependency_filter_is_all_deps: bool,
    pub(crate) settings: HashSet<Setting>,
    extra_functions: Vec<Arc<dyn QueryFunction<T>>>,
    label_printer: LabelPrinter,
    // PostAnalysisQueryEnvironment
    pub(crate) top_level_configurations: Arc<TopLevelConfigurations>,
    main_repo_target_parser: TargetPatternParser,
    pkg_path: PathPackageLocator,
    pub(crate) graph: Arc<dyn WalkableGraph>,
    /// Stores every configuration in the transitive closure of the build graph as a map from its
    /// user-friendly hash to the configuration itself.
    pub(crate) transitive_configurations: Arc<IndexMap<String, Arc<BuildConfigurationValue>>>,
    /// cquery's `ConfiguredTargetAccessor.topLevelAspectsByTarget` (empty for aquery, whose
    /// environment takes no top-level aspects).
    pub(crate) top_level_aspects_by_target: IndexMap<ConfiguredTargetKey, Vec<ConfiguredAspect>>,
}

impl<T: PostAnalysisNode> PostAnalysisQueryEnvironment<T> {
    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:141 PostAnalysisQueryEnvironment.<init>
    pub fn new(
        keep_going: bool,
        event_handler: Arc<dyn ExtendedEventHandler>,
        extra_functions: Vec<Arc<dyn QueryFunction<T>>>,
        top_level_configurations: TopLevelConfigurations,
        transitive_configurations: IndexMap<String, Arc<BuildConfigurationValue>>,
        main_repo_target_parser: TargetPatternParser,
        pkg_path: PathPackageLocator,
        graph: Arc<dyn WalkableGraph>,
        settings: HashSet<Setting>,
        label_printer: LabelPrinter,
    ) -> Self {
        // super(keepGoing, true, Rule.ALL_LABELS, eventHandler, settings, extraFunctions,
        // labelPrinter)
        let event_handler = Arc::new(QueryErrorSensingEventHandler::new(event_handler));
        PostAnalysisQueryEnvironment {
            event_handler_dyn: event_handler.clone(),
            event_handler,
            keep_going,
            dependency_filter_is_all_deps: is_all_deps_filter(&settings),
            settings,
            extra_functions,
            label_printer,
            top_level_configurations: Arc::new(top_level_configurations),
            main_repo_target_parser,
            pkg_path,
            graph,
            transitive_configurations: Arc::new(transitive_configurations),
            top_level_aspects_by_target: IndexMap::new(),
        }
    }

    /// `eventHandler` as the `ExtendedEventHandler` loading calls take.
    pub fn event_handler(&self) -> &Arc<dyn ExtendedEventHandler> {
        &self.event_handler_dyn
    }

    pub fn get_label_printer(&self) -> &LabelPrinter {
        &self.label_printer
    }

    pub fn get_graph(&self) -> &Arc<dyn WalkableGraph> {
        &self.graph
    }

    pub fn get_transitive_configurations(&self) -> &IndexMap<String, Arc<BuildConfigurationValue>> {
        &self.transitive_configurations
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:176 PostAnalysisQueryEnvironment.evaluateQuery
    pub fn evaluate_query(
        &self,
        expr: &QueryExpression,
        callback: &dyn ThreadSafeOutputFormatterCallback<T>,
    ) -> Result<QueryEvalResult, EvaluateQueryError>
    where
        Self: TargetAccessor<T>,
    {
        self.before_evaluate_query()?;
        self.evaluate_query_internal(expr, callback)
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:183 PostAnalysisQueryEnvironment.beforeEvaluateQuery
    fn before_evaluate_query(&self) -> Result<(), QueryException> {
        // PORT: the graph is given at construction; the resolver (GraphBackedRecursivePackageProvider
        // over it) is created per pattern in get_targets_matching_pattern.
        check_settings(&self.settings)
    }

    // TODO(bazel-team): It's weird that this untemplated function exists. Fix? Or don't implement?
    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:219 PostAnalysisQueryEnvironment.getTarget
    pub fn get_target(&self, label: &Label) -> Result<Target, TargetNotFoundException> {
        let value = self
            .graph
            .get_value(&SkyKey::new(PackageKey::key(
                label.get_package_identifier().clone(),
            )))
            .unwrap_or_else(|| {
                panic!(
                    "NullPointerException: package {} not in the graph",
                    label.get_package_identifier()
                )
            })
            .downcast::<PackageValue>()
            .unwrap_or_else(|_| panic!("PACKAGE value"));
        match value.get_package().get_target(label.get_name()) {
            Ok(t) => Ok(t.clone()),
            Err(e) => {
                let detailed_exit_code = e.get_detailed_exit_code();
                Err(TargetNotFoundException::new(
                    Arc::new(e.clone()),
                    failure_details::from_packages(&detailed_exit_code),
                ))
            }
        }
    }

    /// The target behind a node; panics like Java's accessors (`IllegalStateException`).
    pub fn get_target_or_panic(&self, label: &Label) -> Target {
        match self.get_target(label) {
            Ok(t) => t,
            Err(e) => panic!(
                "IllegalStateException: Unable to get target from package in accessor.: {}",
                e.get_message()
            ),
        }
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:247 PostAnalysisQueryEnvironment.getConfiguredTargetValue
    pub fn get_configured_target_value(&self, key: &SkyKey) -> Option<Arc<ConfiguredTargetValue>> {
        blaze_sky_top::skyframe_executor::get_configured_target_value(&*self.graph, key)
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:252 PostAnalysisQueryEnvironment.getAspectValue
    pub fn get_aspect_value(&self, key: &SkyKey) -> Option<Arc<AspectValue>> {
        get_aspect_value(&*self.graph, key)
    }

    /// Java `getValueFromKey`.
    pub fn get_value_from_key(&self, key: &SkyKey) -> Option<T> {
        self.graph_lookup().get_value_from_key(key)
    }

    /// The graph, top-level and transitive configurations (what `ConfigFunction` reads).
    pub fn graph_lookup(&self) -> PostAnalysisGraph {
        PostAnalysisGraph {
            graph: self.graph.clone(),
            top_level_configurations: self.top_level_configurations.clone(),
            transitive_configurations: self.transitive_configurations.clone(),
        }
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:263 PostAnalysisQueryEnvironment.getIgnoredSubdirectories
    fn get_ignored_subdirectories(
        &self,
        repository_name: &RepositoryName,
    ) -> IgnoredSubdirectories {
        match self
            .graph
            .get_value(&SkyKey::new(IgnoredSubdirectoriesKey::key(
                repository_name.clone(),
            ))) {
            None => IgnoredSubdirectories::empty(),
            Some(v) => v
                .downcast::<blaze_sky_loading::ignored_subdirectories::IgnoredSubdirectoriesValue>()
                .unwrap_or_else(|_| panic!("IGNORED_SUBDIRECTORIES value"))
                .as_ignored_subdirectories()
                .clone(),
        }
    }

    /// The configuration of a node.
    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:537 ConfiguredTargetQueryEnvironment.getConfiguration
    pub fn get_configuration(&self, target: &T) -> Option<Arc<BuildConfigurationValue>> {
        let key = target.get_configuration_key()?;
        blaze_sky_top::skyframe_executor::get_configuration_from_graph(&*self.graph, Some(&key))
    }

    /// This method has to exist because `AliasConfiguredTarget.getLabel()` returns the label of
    /// the "actual" target instead of the alias target.
    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:477 ConfiguredTargetQueryEnvironment.getCorrectLabel
    pub fn get_correct_label(&self, target: &T) -> Label {
        // Dereference any aliases that might be present.
        target.get_original_label()
    }

    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:325 ConfiguredTargetQueryEnvironment.getConfiguredTarget
    pub fn get_configured_target(
        &self,
        label: &Label,
        configuration: Option<&Arc<BuildConfigurationValue>>,
    ) -> Option<T> {
        self.graph_lookup()
            .get_configured_target(label, configuration)
    }

    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:483 ConfiguredTargetQueryEnvironment.getTargetConfiguredTarget
    pub fn get_target_configured_target(&self, label: &Label) -> Option<T> {
        self.graph_lookup().get_target_configured_target(label)
    }

    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:501 ConfiguredTargetQueryEnvironment.getNullConfiguredTarget
    pub fn get_null_configured_target(&self, label: &Label) -> Option<T> {
        self.graph_lookup().get_null_configured_target(label)
    }

    /// Returns all configured targets in Skyframe with the given label.
    ///
    /// If there are no matches, returns an empty list.
    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:363 ConfiguredTargetQueryEnvironment.getConfiguredTargetsForLabel
    fn get_configured_targets_for_label(&self, label: &Label) -> Vec<T> {
        let mut ans: Vec<T> = Vec::new();
        let mut extra_configured_target_keys: Option<HashSet<ConfiguredTargetKey>> = None;
        for configuration_value in self.transitive_configurations.values() {
            let configuration_key = configuration_value.get_key();
            let Some(target) = self.get_value_from_key(&SkyKey::new(
                ConfiguredTargetKey::builder()
                    .set_label(label.clone())
                    .set_configuration_key(Some(configuration_key.clone()))
                    .build(),
            )) else {
                continue;
            };
            // The configurations might not match if the target's configuration changed due to a
            // transition or trimming. Filter such targets, with one exception: if the target is
            // subject to a non-idempotent rule transition, we have to keep it once if the keys
            // requested above, which never have shouldApplyRuleTransition set to false, don't
            // cover it. This case is rare, so we optimize for it not being hit.
            if Some(&configuration_key) != target.get_configuration_key().as_ref() {
                let lookup_key = get_configured_target_key(&target);
                let target_key = as_configured_target_key(&lookup_key)
                    .expect("configured target key")
                    .clone();
                if target_key.should_apply_rule_transition()
                    || self
                        .get_value_from_key(&SkyKey::new(
                            ConfiguredTargetKey::builder()
                                .set_label(label.clone())
                                .set_configuration_key(target_key.get_configuration_key().cloned())
                                .build(),
                        ))
                        .is_some()
                {
                    continue;
                }
                let extra = extra_configured_target_keys.get_or_insert_with(HashSet::new);
                if !extra.insert(target_key) {
                    continue;
                }
            }
            ans.push(target);
        }
        if let Some(null_configured_target) = self.get_null_configured_target(label) {
            ans.push(null_configured_target);
        }
        ans
    }

    /// `targetsByKey` of `getFwdDeps` and `getReverseDeps`
    /// (`Maps.newHashMapWithExpectedSize(Iterables.size(targets))` filled in the order of
    /// `targets`) and its key set in Java's iteration order.
    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:288 PostAnalysisQueryEnvironment.getFwdDeps
    fn targets_by_key(&self, targets: &[T]) -> (HashMap<SkyKey, T>, Vec<SkyKey>) {
        let mut targets_by_key: HashMap<SkyKey, T> = HashMap::with_capacity(targets.len());
        let mut keys: Vec<SkyKey> = Vec::with_capacity(targets.len());
        for target in targets {
            let key = lookup_key_to_sky_key(&get_configured_target_key(target));
            targets_by_key.insert(key.clone(), target.clone());
            keys.push(key);
        }
        // PORT: java-hash-order.
        let key_set = targets_by_key_order(targets.len(), keys, &sky_key_java_hash_code);
        (targets_by_key, key_set)
    }

    /// The warning of `getFwdDeps` / `getReverseDeps` for the targets the graph has no entry for:
    /// `Sets.difference(targetsByKey.keySet(), result.keySet())` in the order of the first set.
    fn warn_missing_targets<V>(&self, targets_by_key_set: &[SkyKey], result: &[(SkyKey, V)]) {
        let missing_targets: Vec<String> = targets_by_key_set
            .iter()
            .filter(|k| !result.iter().any(|(r, _)| r == *k))
            // SKYKEY_TO_CTKEY: `(ConfiguredTargetKey) skyKey.argument()`, then
            // ConfiguredTargetKey.toString(); the cast fails for an aspect node's key.
            .map(|k| {
                let ct_key = k.downcast::<ConfiguredTargetKey>().unwrap_or_else(|| {
                    panic!(
                        "ClassCastException: {:?} cannot be cast to ConfiguredTargetKey",
                        k
                    )
                });
                format!("{:?}", ct_key)
            })
            .collect();
        self.event_handler.handle(&Event::warn(format!(
            "Targets were missing from graph: [{}]",
            missing_targets.join(", ")
        )));
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:287 PostAnalysisQueryEnvironment.getFwdDeps
    pub fn get_fwd_deps_set(&self, targets: &[T]) -> Arc<dyn ThreadSafeMutableSet<T>> {
        let (targets_by_key, targets_by_key_set) = self.targets_by_key(targets);
        // graph.getDirectDeps(targetsByKey.keySet())
        let mut direct_deps_of_keys: Vec<(SkyKey, Vec<SkyKey>)> = Vec::new();
        for key in &targets_by_key_set {
            // Java WalkableGraph.getDirectDeps(keys) only returns the keys that have a node.
            if self.graph.get_value(key).is_none() && self.graph.get_exception(key).is_none() {
                continue;
            }
            direct_deps_of_keys.push((key.clone(), self.graph.get_direct_deps(key)));
        }
        // PORT: java-hash-order (the HashMaps of InMemoryGraphImpl.getBatchMap and
        // DelegatingWalkableGraph.getDirectDeps).
        let direct_deps_of_keys =
            delegating_walkable_graph_map_order(direct_deps_of_keys, &sky_key_java_hash_code);
        let direct_deps = self.targetify_values_map(&targets_by_key, direct_deps_of_keys);
        if targets_by_key.len() != direct_deps.len() {
            self.warn_missing_targets(&targets_by_key_set, &direct_deps);
        }
        let result = self.create_set();
        for (key, deps) in &direct_deps {
            result.add_all(&self.filter_fwd_deps(&targets_by_key[key], deps));
        }
        result
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:314 PostAnalysisQueryEnvironment.filterFwdDeps
    fn filter_fwd_deps(
        &self,
        config_target: &T,
        raw_fwd_deps: &[ClassifiedDependency<T>],
    ) -> Vec<T> {
        if self.settings.is_empty() {
            return get_dependencies(raw_fwd_deps);
        }
        self.get_allowed_deps(config_target, raw_fwd_deps.to_vec())
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:323 PostAnalysisQueryEnvironment.getReverseDeps
    fn get_reverse_deps_impl(&self, targets: &[T]) -> Vec<T> {
        let (targets_by_key, targets_by_key_set) = self.targets_by_key(targets);
        // graph.getReverseDeps(targetsByKey.keySet()): the entries of the done nodes.
        // PORT: java-hash-order (the HashMaps of InMemoryGraphImpl.getBatchMap and
        // DelegatingWalkableGraph.getReverseDeps). A node that exists without being done is in
        // Java's first map and not in its second; WalkableGraph only shows done nodes here.
        let reverse_deps = delegating_walkable_graph_map_order(
            self.graph.get_reverse_deps(&targets_by_key_set),
            &sky_key_java_hash_code,
        );
        let reverse_deps_by_key = self.targetify_values_map(
            &targets_by_key,
            self.skip_delegating_ancestors(reverse_deps),
        );
        if targets_by_key.len() != reverse_deps_by_key.len() {
            self.warn_missing_targets(&targets_by_key_set, &reverse_deps_by_key);
        }
        // Map<T, ImmutableList<ClassifiedDependency<T>>> reverseDepsByCT = new HashMap<>();
        // PORT: java-hash-order, not replicable. The map is keyed by the nodes, whose hashCode is
        // the identity hash code (neither ConfiguredTarget nor ConfiguredTargetValue overrides
        // it), so Java's iteration order changes from run to run; here it is the order of
        // `reverseDepsByKey`.
        let mut reverse_deps_by_ct: Vec<(T, Vec<ClassifiedDependency<T>>)> = Vec::new();
        for (key, deps) in reverse_deps_by_key {
            reverse_deps_by_ct.push((targets_by_key[&key].clone(), deps));
        }
        if reverse_deps_by_ct.is_empty() {
            Vec::new()
        } else {
            self.filter_reverse_deps(reverse_deps_by_ct)
        }
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:348 PostAnalysisQueryEnvironment.filterReverseDeps
    fn filter_reverse_deps(
        &self,
        raw_reverse_deps: Vec<(T, Vec<ClassifiedDependency<T>>)>,
    ) -> Vec<T> {
        // Set<T> result = CompactHashSet.create();
        // PORT: a CompactHashSet iterates in insertion order. Java compares the nodes by
        // identity; a node is identified by its lookup key here.
        let mut result: IndexMap<ActionLookupKeyRef, T> = IndexMap::new();
        for (target, rdeps) in raw_reverse_deps {
            let mut rule_deps: Vec<ClassifiedDependency<T>> = Vec::new();
            for parent in rdeps {
                // `parent.dependency instanceof RuleConfiguredTarget` is a test of the node
                // object: never true for aquery, whose nodes are ConfiguredTargetValues.
                if parent.dependency.is_instance_of_rule_configured_target()
                    && !self.dependency_filter_is_all_deps
                {
                    rule_deps.push(parent);
                } else {
                    let dependency = parent.dependency;
                    result
                        .entry(get_configured_target_key(&dependency))
                        .or_insert(dependency);
                }
            }
            for dep in self.get_allowed_deps(&target, rule_deps) {
                result.entry(get_configured_target_key(&dep)).or_insert(dep);
            }
        }
        result.into_values().collect()
    }

    /// Expands any delegating ancestors when computing reverse dependencies.
    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:379 PostAnalysisQueryEnvironment.skipDelegatingAncestors
    fn skip_delegating_ancestors(
        &self,
        reverse_deps: Vec<(SkyKey, Vec<SkyKey>)>,
    ) -> Vec<(SkyKey, Vec<SkyKey>)> {
        // var result = ImmutableListMultimap.<SkyKey, SkyKey>builder();
        let mut result: Vec<(SkyKey, Vec<SkyKey>)> = Vec::new();
        for (child, rdeps) in reverse_deps {
            let unwound_rdeps =
                self.unwind_reverse_dependency_delegation_layers_if_found(&child, &rdeps);
            let values = unwound_rdeps.unwrap_or(rdeps);
            // result.putAll(child, values): ImmutableMultimap.Builder.putAll adds nothing, not
            // even the key, for an empty collection, so a node without reverse deps (a top-level
            // target) is absent from the result and reported as missing from the graph.
            if values.is_empty() {
                continue;
            }
            result.push((child, values));
        }
        result
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:392 PostAnalysisQueryEnvironment.unwindReverseDependencyDelegationLayersIfFound
    fn unwind_reverse_dependency_delegation_layers_if_found(
        &self,
        child: &SkyKey,
        rdeps: &[SkyKey],
    ) -> Option<Vec<SkyKey>> {
        // Most rdeps will not be delegating. Performs an optimistic pass that avoids copying.
        let mut found_delegating_rdep = false;
        for rdep_key in rdeps {
            if rdep_key.function_name() != CONFIGURED_TARGET {
                continue;
            }
            let Some(rdep_value) = self.get_value_from_key(rdep_key) else {
                // Cannot find the actual value, possibly because it failed during analysis.
                continue;
            };
            let actual_parent_key = lookup_key_to_sky_key(&get_configured_target_key(&rdep_value));
            if actual_parent_key == *child {
                // The parent has the same value as the child because it is delegating.
                found_delegating_rdep = true;
                break;
            }
        }
        if !found_delegating_rdep {
            return None;
        }
        // var logicalParents = new HashSet<SkyKey>();
        let mut logical_parents: Vec<SkyKey> = Vec::new();
        self.unwind_reverse_dependency_delegation_layers(child, rdeps, &mut logical_parents);
        // PORT: java-hash-order. The caller iterates the HashSet. The keys targetifyValues looks
        // at (CONFIGURED_TARGET, ASPECT, TOOLCHAIN_RESOLUTION) come in Java's order (for ASPECT
        // keys, with the identity hash code of `java_class_identity_hash_code`); the keys of
        // other functions, which it skips, follow them (their place cannot be observed).
        let mut ignored: Vec<SkyKey> = Vec::new();
        let mut elements: Vec<(SkyKey, Option<i32>)> = Vec::new();
        for key in logical_parents {
            let hash_code = sky_key_java_hash_code_if_known(&key);
            if hash_code.is_none() {
                if !ignored.contains(&key) {
                    ignored.push(key.clone());
                }
            }
            elements.push((key, hash_code));
        }
        let mut ordered = java_hash_set_order_of_known(elements);
        ordered.extend(ignored);
        Some(ordered)
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:421 PostAnalysisQueryEnvironment.unwindReverseDependencyDelegationLayers
    fn unwind_reverse_dependency_delegation_layers(
        &self,
        child: &SkyKey,
        rdeps: &[SkyKey],
        output: &mut Vec<SkyKey>,
    ) {
        // Checks the value of each rdep to see if it is delegating to `child`. If so, fetches
        // its rdeps and processes those, applying the same expansion as needed.
        for rdep_key in rdeps {
            if rdep_key.function_name() != CONFIGURED_TARGET {
                output.push(rdep_key.clone());
                continue;
            }
            let Some(rdep_value) = self.get_value_from_key(rdep_key) else {
                // Cannot find the actual value, possibly because it failed during analysis.
                continue;
            };
            let actual_parent_key = lookup_key_to_sky_key(&get_configured_target_key(&rdep_value));
            if actual_parent_key != *child {
                output.push(rdep_key.clone());
                continue;
            }
            // Otherwise `rdepKey` is delegating to child and needs to be unwound.
            let rdep_parents: Vec<SkyKey> = self
                .graph
                .get_reverse_deps(std::slice::from_ref(rdep_key))
                .into_iter()
                .next()
                .map(|(_, v)| v)
                .unwrap_or_default();
            // Applies this recursively in case there are multiple layers of delegation.
            self.unwind_reverse_dependency_delegation_layers(child, &rdep_parents, output);
        }
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:452 PostAnalysisQueryEnvironment.getAllowedDeps
    fn get_allowed_deps(&self, target: &T, mut deps: Vec<ClassifiedDependency<T>>) -> Vec<T> {
        // It's possible to query on a target that's configured in an exec configuration. In
        // those cases if --notool_deps is turned on, we only allow reachable targets that are
        // ALSO in an exec config. This is somewhat counterintuitive and subject to change in the
        // future but seems like the best option right now.
        if self.settings.contains(&Setting::OnlyTargetDeps) {
            let current_config = self.get_configuration(target);
            if current_config
                .as_ref()
                .is_some_and(|c| c.is_tool_configuration())
            {
                deps.retain(|dep| {
                    self.get_configuration(&dep.dependency)
                        .is_some_and(|c| c.is_tool_configuration())
                });
            } else {
                deps.retain(|dep| {
                    // We include source files, which have null configuration, even though they
                    // can also appear on exec-configured attributes like genrule#tools. While
                    // this may not be strictly correct, it's better to overapproximate than
                    // underapproximate the results.
                    match self.get_configuration(&dep.dependency) {
                        None => true,
                        Some(c) => !c.is_tool_configuration(),
                    }
                });
            }
        }
        if self.settings.contains(&Setting::NoImplicitDeps) {
            deps.retain(|dep| !dep.implicit);
        }
        get_dependencies(&deps)
    }

    /// Java `isAliasConfiguredTarget(ConfiguredTargetKey)`: whether the configured target of
    /// `key` in the graph is an alias.
    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:256 PostAnalysisQueryEnvironment.isAliasConfiguredTarget
    fn is_alias_configured_target_key(&self, key: &ConfiguredTargetKey) -> bool {
        let value = self
            .get_configured_target_value(&SkyKey::new(key.clone()))
            .unwrap_or_else(|| {
                panic!(
                    "NullPointerException: no ConfiguredTargetValue for {:?}",
                    key
                )
            });
        AliasProvider::is_alias(value.get_configured_target())
    }

    /// Java `getRuleConfiguredTarget(T)`.
    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:525 ConfiguredTargetQueryEnvironment.getRuleConfiguredTarget
    fn get_rule_configured_target(target: &T) -> Option<&Arc<RuleConfiguredTarget>> {
        match target.as_configured_target() {
            Some(ConfiguredTarget::Rule(r)) => Some(r),
            _ => None,
        }
    }

    /// If `target` is an `OutputFileConfiguredTarget`, return the rule that produces it.
    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:534 ConfiguredTargetQueryEnvironment.getOwningRuleforOutputConfiguredTarget
    fn get_owning_rule_for_output_configured_target(
        target: &T,
    ) -> Option<Arc<RuleConfiguredTarget>> {
        match target.as_configured_target() {
            Some(ConfiguredTarget::OutputFile(o)) => Some(o.get_generating_rule().clone()),
            _ => None,
        }
    }

    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:543 ConfiguredTargetQueryEnvironment.isAliasConfiguredTarget
    fn is_alias_configured_target(target: &T) -> bool {
        matches!(
            target.as_configured_target(),
            Some(ConfiguredTarget::Alias(_))
        )
    }

    /// Returns targetified dependencies wrapped as `ClassifiedDependency` objects which include
    /// information on if the target is an implicit or explicit dependency.
    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:518 PostAnalysisQueryEnvironment.targetifyValues
    fn targetify_values(
        &self,
        parent: Option<&T>,
        dependencies: &[SkyKey],
        known_ct_deps: &mut HashSet<SkyKey>,
        resolved_aspect_classes: &mut HashSet<AspectClass>,
    ) -> Vec<ClassifiedDependency<T>> {
        let mut implicit_deps: Option<IndexSet<ConfiguredTargetKey>> = None;
        if let Some(parent) = parent {
            if let Some(rule_configured_target) = Self::get_rule_configured_target(parent) {
                implicit_deps = Some(rule_configured_target.get_implicit_deps().clone());
            }
        }

        let explicit_aspects = self.settings.contains(&Setting::IncludeAspects)
            && self.settings.contains(&Setting::ExplicitAspects);

        let mut values: Vec<ClassifiedDependency<T>> = Vec::new();
        // TODO(bazel-team): The end-goal approach is to treat aspects and toolchains as
        // first-class query nodes just like targets.
        for key in dependencies {
            if known_ct_deps.contains(key) {
                continue;
            }
            let function_name = key.function_name().get_name();
            if function_name == CONFIGURED_TARGET.get_name()
                || (explicit_aspects && function_name == "ASPECT")
            {
                let dependency = self.get_value_from_key(key);
                if dependency.is_none() && self.keep_going {
                    self.event_handler.handle(&Event::warn(format!(
                        "Dependency {:?} is unavaillable for querying. This may be because this \
                         query sets --keep_going, which continues analysis even some of the build \
                         graph  fails to analyze. For more accurate results ensure this request \
                         can build.",
                        key
                    )));
                    continue;
                }
                // If --keep_going wasn't applied and a build node is missing, something's wrong
                // with the build graph.
                let dependency = dependency.unwrap_or_else(|| {
                    panic!(
                        "IllegalStateException: query-requested node '{:?}' was unavailable in \
                         the query environment graph. If you come across this error, please file \
                         an issue at https://github.com/bazelbuild/bazel or contact the bazel \
                         configurability team.",
                        key
                    )
                });

                let implicit_dep = if key.downcast::<ConfiguredTargetKey>().is_none() {
                    // All non-configured target deps are implicit.
                    true
                } else if parent.is_some_and(Self::is_alias_configured_target) {
                    // Aliases have only one configured target dep: the ":actual" parameter.
                    false
                } else if let Some(owning_rule) =
                    parent.and_then(Self::get_owning_rule_for_output_configured_target)
                {
                    // An output file's generating target is an explicit dep.
                    match Self::get_rule_configured_target(&dependency) {
                        None => false,
                        Some(rule) => rule.base.lookup_key != owning_rule.base.lookup_key,
                    }
                } else if let Some(implicit_deps) = &implicit_deps {
                    // Check both the original guess key and the second correct key. In the case
                    // of the target platform, Util.findImplicitDeps also uses the original guess
                    // key.
                    let key_ctk = key.downcast::<ConfiguredTargetKey>().expect("checked");
                    implicit_deps.contains(key_ctk)
                        || as_configured_target_key(&get_configured_target_key(&dependency))
                            .is_some_and(|k| implicit_deps.contains(k))
                } else {
                    // No set of implicit deps available. Assume they're all implicit. This
                    // implies the parent isn't a rule configured target.
                    blaze_util::check_state!(
                        !parent.is_some_and(|p| Self::get_rule_configured_target(p).is_some())
                    );
                    true
                };

                values.push(ClassifiedDependency {
                    dependency,
                    implicit: implicit_dep,
                });
                known_ct_deps.insert(key.clone());
            } else if self.settings.contains(&Setting::IncludeAspects) && function_name == "ASPECT"
            {
                blaze_util::check_state!(!self.settings.contains(&Setting::ExplicitAspects));
                let aspect_key = key
                    .downcast::<AspectKey>()
                    .unwrap_or_else(|| panic!("ClassCastException: {:?} is not an AspectKey", key));
                if resolved_aspect_classes.contains(aspect_key.get_aspect_class()) {
                    continue;
                }
                // When an aspect is attached to an alias configured target, it bypasses standard
                // dependency resolution and just Skyframe-loads the same aspect for the alias'
                // referent. That means the original aspect's attribute deps aren't
                // Skyframe-resolved through AspectFunction's usual call to
                // ConfiguredTargetFunction.computeDependencies, so graph.getDirectDeps() won't
                // include them. So we defer "resolving" the aspect class to the non-alias version,
                // which properly reflects all dependencies. See AspectFunction for details.
                if !self.is_alias_configured_target_key(aspect_key.get_base_configured_target_key())
                {
                    // Make sure we don't examine aspects of this type again. This saves us from
                    // unnecessarily traversing a target's transitive deps because it propagates an
                    // aspect down those deps. The deps added by the aspect are a function of the
                    // aspect's class, not the target it's attached to. And they can't be
                    // configured because aspects have no UI for overriding attribute defaults. So
                    // it's sufficient to examine only a single instance of a given aspect class.
                    // This has real memory and performance consequences: see b/163052263.
                    // Note the aspect could attach *another* aspect type to its deps. That will
                    // still get examined through the recursive call.
                    resolved_aspect_classes.insert(aspect_key.get_aspect_class().clone());
                }
                let deps = self.graph.get_direct_deps(key);
                values.extend(self.targetify_values(
                    None,
                    &deps,
                    known_ct_deps,
                    resolved_aspect_classes,
                ));
            } else if function_name == "TOOLCHAIN_RESOLUTION" {
                let deps = self.graph.get_direct_deps(key);
                values.extend(self.targetify_values(
                    None,
                    &deps,
                    known_ct_deps,
                    resolved_aspect_classes,
                ));
            }
        }
        values
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:632 PostAnalysisQueryEnvironment.targetifyValues
    fn targetify_values_map(
        &self,
        from_targets_by_key: &HashMap<SkyKey, T>,
        input: Vec<(SkyKey, Vec<SkyKey>)>,
    ) -> Vec<(SkyKey, Vec<ClassifiedDependency<T>>)> {
        // Map<SkyKey, ImmutableList<ClassifiedDependency<T>>> result = new HashMap<>();
        let mut result: Vec<(SkyKey, Vec<ClassifiedDependency<T>>)> =
            Vec::with_capacity(input.len());
        for (from_key, deps) in input {
            let values = self.targetify_values(
                from_targets_by_key.get(&from_key),
                &deps,
                &mut HashSet::new(),
                &mut HashSet::new(),
            );
            result.push((from_key, values));
        }
        // PORT: java-hash-order.
        java_hash_map_entry_order(16, result, &sky_key_java_hash_code)
    }

    fn create_set(&self) -> Arc<dyn ThreadSafeMutableSet<T>> {
        Arc::new(KeyedNodeSet::<T>::new())
    }

    /// `getTargetsMatchingPattern` of both subclasses.
    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:257 ConfiguredTargetQueryEnvironment.getTargetsMatchingPattern
    fn get_targets_matching_pattern_impl(
        &self,
        owner: &QueryExpression,
        pattern: &str,
        callback: &dyn Callback<T>,
    ) -> Result<(), QueryError> {
        let pattern_to_eval = match self.main_repo_target_parser.parse(pattern) {
            Ok(p) => p,
            Err(tpe) => {
                self.handle_error_impl(
                    owner,
                    tpe.get_message(),
                    Some(&target_parsing_detailed_exit_code(&tpe)),
                )?;
                return Ok(());
            }
        };
        let provider = GraphBackedRecursivePackageProvider {
            graph: &*self.graph,
            pkg_roots: self.pkg_path.get_path_entries().to_vec(),
        };
        let resolver = RecursivePackageProviderBackedTargetPatternResolver::new(
            &provider,
            self.event_handler.as_ref(),
            FilteringPolicy::NoFilter,
        );
        let repository = pattern_to_eval.get_repository();
        let ignored = self.get_ignored_subdirectories(&repository);
        let mut callback_error: Option<QueryError> = None;
        let eval_result = target_pattern_eval::eval(
            &pattern_to_eval,
            &resolver,
            &|| Ok(ignored.clone()),
            /* excluded_subdirectories= */ &[],
            &mut |partial_result: Vec<Target>| {
                if callback_error.is_some() {
                    return;
                }
                let mut transformed_result: Vec<T> = Vec::new();
                for target in &partial_result {
                    transformed_result
                        .extend(self.get_configured_targets_for_label(target.get_label()));
                }
                if let Err(e) = callback.process(&transformed_result) {
                    callback_error = Some(e);
                }
            },
        );
        if let Some(e) = callback_error {
            return Err(e);
        }
        match eval_result {
            Ok(()) => Ok(()),
            Err(TargetPatternEvalError::TargetParsing(exn)) => {
                self.handle_error_impl(
                    owner,
                    exn.get_message(),
                    Some(&target_parsing_detailed_exit_code(&exn)),
                )?;
                Ok(())
            }
            Err(TargetPatternEvalError::Interrupted) => {
                Err(QueryError::Interrupted(InterruptedException::new()))
            }
            // RecursivePkgValueRootPackageExtractor throws
            // `new QueryException(message, Code.TARGET_NOT_IN_UNIVERSE_SCOPE)`: no expression, so
            // the top level describes it as "Evaluation failed: ...".
            // Java: com/google/devtools/build/lib/skyframe/RecursivePkgValueRootPackageExtractor.java:60 RecursivePkgValueRootPackageExtractor.streamPackagesFromRoots
            Err(TargetPatternEvalError::ProcessPackageDirectory(message)) => Err(
                QueryException::new(None, message, query::Code::TargetNotInUniverseScope).into(),
            ),
            Err(other) => panic!(
                "IllegalStateException: unexpected target pattern evaluation failure: {:?}",
                other
            ),
        }
    }

    /// Handle an error during evaluation of `expression` by either throwing `QueryException` or
    /// emitting an event, depending on whether the evaluation is running in a "keep going" mode.
    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:422 AbstractBlazeQueryEnvironment.handleError
    fn handle_error_impl(
        &self,
        expression: &QueryExpression,
        message: &str,
        detailed_exit_code: Option<&DetailedExitCode>,
    ) -> Result<(), QueryException> {
        if !self.keep_going {
            if let Some(detailed_exit_code) = detailed_exit_code {
                return Err(QueryException::with_failure_detail(
                    Some(expression),
                    message,
                    detailed_exit_code
                        .get_failure_detail()
                        .cloned()
                        .expect("DetailedExitCode without FailureDetail"),
                ));
            } else {
                // PORT: skipped, Google-internal: BugReport.sendBugReport("Undetailed failure:
                // ...").
                return Err(QueryException::new(
                    Some(expression),
                    message,
                    query::Code::NonDetailedError,
                ));
            }
        }
        self.event_handler
            .handle(&create_error_event(expression, message, detailed_exit_code));
        Ok(())
    }

    /// Evaluate the specified query expression in this environment, streaming results to the
    /// given `callback`. `callback.start()` will be called before query evaluation and
    /// `callback.close()` will be unconditionally called at the end of query evaluation.
    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:160 AbstractBlazeQueryEnvironment.evaluateQueryInternal
    fn evaluate_query_internal(
        &self,
        expr: &QueryExpression,
        callback: &dyn ThreadSafeOutputFormatterCallback<T>,
    ) -> Result<QueryEvalResult, EvaluateQueryError>
    where
        Self: TargetAccessor<T>,
    {
        let empty_sensing_callback = EmptinessSensingCallback {
            callback,
            num_targets: AtomicI32::new(0),
            io_exception: IoExceptionSlot::new(),
        };
        // Target patterns are resolved on the fly so no pre-work to be done here
        // (PostAnalysisQueryEnvironment.preloadOrThrow).
        let mut io_exn: Option<IoException> = None;
        let mut fail_fast = true;
        let result = (|| -> Result<(), EvaluateQueryError> {
            empty_sensing_callback
                .start()
                .map_err(EvaluateQueryError::Io)?;
            // evalTopLevelInternal
            let env: &dyn QueryEnvironment<T> = self;
            match env
                .eval(
                    expr,
                    &QueryExpressionContext::empty(),
                    &empty_sensing_callback,
                )
                .get_checked()
            {
                Ok(()) => {}
                Err(QueryError::Query(e)) => {
                    return Err(QueryException::wrap_toplevel(e, expr).into());
                }
                Err(QueryError::Interrupted(e)) => return Err(e.into()),
                Err(QueryError::Cancellation) => {
                    panic!("java.util.concurrent.CancellationException")
                }
            }
            fail_fast = false;
            Ok(())
        })();
        // finally
        match empty_sensing_callback.close(fail_fast) {
            Ok(()) => {}
            // Only throw this IOException if we weren't about to throw a different exception.
            Err(OutputFormatterError::Io(e)) => io_exn = Some(e),
            Err(OutputFormatterError::Interrupted(e)) => return Err(e.into()),
        }
        result?;
        if let Some(io_exn) = io_exn {
            return Err(EvaluateQueryError::Io(io_exn));
        }

        let event_handler = &self.event_handler;
        if event_handler.has_errors() {
            let detailed_exit_code = event_handler.get_error_property();
            if !self.keep_going {
                if let Some(detailed_exit_code) = detailed_exit_code {
                    return Err(QueryException::with_failure_detail(
                        None,
                        format!(
                            "Evaluation of query \"{}\" failed",
                            expr.to_trunctated_string()
                        ),
                        detailed_exit_code
                            .get_failure_detail()
                            .cloned()
                            .expect("DetailedExitCode without FailureDetail"),
                    )
                    .into());
                }
                return Err(QueryException::new(
                    None,
                    format!(
                        "Evaluation of query \"{}\" failed due to BUILD file errors",
                        expr.to_trunctated_string()
                    ),
                    query::Code::BuildFileError,
                )
                .into());
            }
            event_handler.handle(&Event::warn(
                "--keep_going specified, ignoring errors. Results may be inaccurate",
            ));
            if let Some(detailed_exit_code) = detailed_exit_code {
                return Ok(QueryEvalResult::failure(
                    empty_sensing_callback.is_empty(),
                    detailed_exit_code,
                ));
            } else {
                return Ok(QueryEvalResult::failure(
                    empty_sensing_callback.is_empty(),
                    DetailedExitCode::of(FailureDetail {
                        message: format!(
                            "Evaluation of query \"{}\" failed due to BUILD file errors",
                            expr.to_trunctated_string()
                        ),
                        category: Some(Category::Query(Query {
                            code: query::Code::BuildFileError as i32,
                        })),
                    }),
                ));
            }
        }
        Ok(QueryEvalResult::success(empty_sensing_callback.is_empty()))
    }
}

/// Whether `constructDependencyFilter(settings)` returns `DependencyFilter.ALL_DEPS` itself.
// Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:117 AbstractBlazeQueryEnvironment.constructDependencyFilter
fn is_all_deps_filter(settings: &HashSet<Setting>) -> bool {
    // ONLY_TARGET_DEPS starts from ONLY_TARGET_DEPS; NO_IMPLICIT_DEPS and NO_NODEP_DEPS `and` a
    // new filter onto it; otherwise the ALL_DEPS constant is returned.
    !settings.contains(&Setting::OnlyTargetDeps)
        && !settings.contains(&Setting::NoImplicitDeps)
        && !settings.contains(&Setting::NoNodepDeps)
}

/// Java's `Setting` enum constant names.
fn setting_java_name(setting: &Setting) -> &'static str {
    match setting {
        Setting::TestsExpressionStrict => "TESTS_EXPRESSION_STRICT",
        Setting::NoImplicitDeps => "NO_IMPLICIT_DEPS",
        Setting::OnlyTargetDeps => "ONLY_TARGET_DEPS",
        Setting::NoNodepDeps => "NO_NODEP_DEPS",
        Setting::IncludeAspects => "INCLUDE_ASPECTS",
        Setting::ExplicitAspects => "EXPLICIT_ASPECTS",
    }
}

/// Check to make sure the settings requested are currently supported by this class.
// Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:203 PostAnalysisQueryEnvironment.checkSettings
fn check_settings(settings: &HashSet<Setting>) -> Result<(), QueryException> {
    if settings.contains(&Setting::NoNodepDeps)
        || settings.contains(&Setting::TestsExpressionStrict)
    {
        // Sets.difference(settings, {ONLY_TARGET_DEPS, NO_IMPLICIT_DEPS}): a view of `settings`,
        // an EnumSet (CommonQueryOptions.toSettings), which iterates in ordinal order.
        let mut remaining: Vec<&Setting> = settings
            .iter()
            .filter(|s| **s != Setting::OnlyTargetDeps && **s != Setting::NoImplicitDeps)
            .collect();
        remaining.sort();
        return Err(QueryException::with_configurable_query_code(
            None,
            format!(
                "The following filter(s) are not currently supported by configured query: [{}]",
                remaining
                    .iter()
                    .map(|s| setting_java_name(s))
                    .collect::<Vec<_>>()
                    .join(", ")
            ),
            configurable_query::Code::FiltersNotSupported,
        ));
    }
    Ok(())
}

/// `TargetParsingException.getDetailedExitCode()`.
fn target_parsing_detailed_exit_code(e: &TargetParsingException) -> DetailedExitCode {
    DetailedExitCode::of_exit_code(
        e.get_code().exit_code(),
        failure_details::target_parsing_failure_detail(e),
    )
}

// Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:584 AbstractBlazeQueryEnvironment.createErrorEvent
fn create_error_event(
    expr: &QueryExpression,
    message: &str,
    detailed_exit_code: Option<&DetailedExitCode>,
) -> Event {
    let event_message = format!(
        "Evaluation of query \"{}\" failed: {}",
        expr.to_trunctated_string(),
        message
    );
    let mut event = Event::error(event_message.clone());
    if let Some(detailed_exit_code) = detailed_exit_code {
        let mut failure_detail = detailed_exit_code
            .get_failure_detail()
            .cloned()
            .expect("DetailedExitCode without FailureDetail");
        failure_detail.message = event_message;
        event = event.with_property(Some(DetailedExitCode::of_exit_code(
            detailed_exit_code.get_exit_code(),
            failure_detail,
        )));
    }
    event
}

fn get_dependencies<T: Clone>(classified_dependencies: &[ClassifiedDependency<T>]) -> Vec<T> {
    classified_dependencies
        .iter()
        .map(|dep| dep.dependency.clone())
        .collect()
}

/// A callback counting the targets it forwards.
// Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:337 AbstractBlazeQueryEnvironment.EmptinessSensingCallback
struct EmptinessSensingCallback<'a, T> {
    callback: &'a dyn OutputFormatterCallback<T>,
    num_targets: AtomicI32,
    io_exception: IoExceptionSlot,
}

impl<T> EmptinessSensingCallback<'_, T> {
    fn is_empty(&self) -> bool {
        self.num_targets.load(Ordering::SeqCst) == 0
    }
}

impl<T> Callback<T> for EmptinessSensingCallback<'_, T> {
    fn process(&self, partial_result: &[T]) -> Result<(), QueryError> {
        output_formatter_callback_process(self, partial_result)
    }
}

impl<T> OutputFormatterCallback<T> for EmptinessSensingCallback<'_, T> {
    fn start(&self) -> Result<(), IoException> {
        self.callback.start()
    }

    fn process_output(&self, partial_result: &[T]) -> Result<(), OutputFormatterError> {
        self.num_targets
            .fetch_add(partial_result.len() as i32, Ordering::SeqCst);
        self.callback.process_output(partial_result)
    }

    fn close(&self, fail_fast: bool) -> Result<(), OutputFormatterError> {
        self.callback.close(fail_fast)
    }

    fn io_exception_slot(&self) -> &IoExceptionSlot {
        &self.io_exception
    }
}

/// Wrapper for evaluating query expression in a non-streaming blaze query environment.
// Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:224 AbstractBlazeQueryEnvironment.AbstractBlazeQueryEvaluateExpressionImpl
struct EvaluateExpressionImpl<'a, T> {
    env: &'a dyn QueryEnvironment<T>,
    expression: QueryExpression,
    context: QueryExpressionContext<T>,
    query_task_future_cancelled: Cell<Option<bool>>,
}

impl<T: Clone + 'static> EvaluateExpression<T> for EvaluateExpressionImpl<'_, T> {
    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:236 AbstractBlazeQueryEvaluateExpressionImpl.eval
    fn eval(&self, callback: &dyn Callback<T>) -> QueryTaskFuture<()> {
        let query_task_future = self.env.eval(&self.expression, &self.context, callback);
        self.query_task_future_cancelled
            .set(Some(query_task_future.is_cancelled()));
        query_task_future
    }

    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:244 AbstractBlazeQueryEvaluateExpressionImpl.gracefullyCancel
    fn gracefully_cancel(&self) -> bool {
        false
    }

    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:251 AbstractBlazeQueryEvaluateExpressionImpl.isUngracefullyCancelled
    fn is_ungracefully_cancelled(&self) -> bool {
        self.query_task_future_cancelled.get().unwrap_or(false)
    }
}

impl<T: PostAnalysisNode> QueryEnvironment<T> for PostAnalysisQueryEnvironment<T>
where
    PostAnalysisQueryEnvironment<T>: TargetAccessor<T>,
{
    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:728 PostAnalysisQueryEnvironment.getSiblingTargetsInPackage
    fn get_sibling_targets_in_package(&self, _target: &T) -> Result<Vec<T>, QueryError> {
        Err(QueryException::with_configurable_query_code(
            None,
            "siblings() not supported for post analysis queries",
            configurable_query::Code::SiblingsFunctionNotSupported,
        )
        .into())
    }

    fn get_targets_matching_pattern(
        &self,
        owner: &QueryExpression,
        pattern: &str,
        callback: &dyn Callback<T>,
    ) -> QueryTaskFuture<()> {
        match self.get_targets_matching_pattern_impl(owner, pattern, callback) {
            Ok(()) => self.immediate_successful_future(()),
            Err(QueryError::Query(e)) => self.immediate_failed_future(e),
            Err(QueryError::Interrupted(_)) => self.immediate_cancelled_future(),
            Err(QueryError::Cancellation) => panic!("java.util.concurrent.CancellationException"),
        }
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:230 PostAnalysisQueryEnvironment.getOrCreate
    fn get_or_create(&self, target: &T) -> T {
        target.clone()
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:309 PostAnalysisQueryEnvironment.getFwdDeps
    fn get_fwd_deps(
        &self,
        targets: &[T],
        _context: &QueryExpressionContext<T>,
    ) -> Result<Vec<T>, InterruptedException> {
        Ok(self.get_fwd_deps_set(targets).to_vec())
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:323 PostAnalysisQueryEnvironment.getReverseDeps
    fn get_reverse_deps(
        &self,
        targets: &[T],
        _context: &QueryExpressionContext<T>,
    ) -> Result<Vec<T>, InterruptedException> {
        Ok(self.get_reverse_deps_impl(targets))
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:682 PostAnalysisQueryEnvironment.getTransitiveClosure
    fn get_transitive_closure(
        &self,
        targets: &dyn ThreadSafeMutableSet<T>,
        _context: &QueryExpressionContext<T>,
    ) -> Result<Arc<dyn ThreadSafeMutableSet<T>>, InterruptedException> {
        // Java: com/google/devtools/build/lib/query2/SkyQueryUtils.java:36 SkyQueryUtils.getTransitiveClosure
        let visited = self.create_set();
        let mut current: Vec<T> = targets.to_vec();
        while !current.is_empty() {
            let to_visit: Vec<T> = current
                .into_iter()
                .filter(|obj| !visited.contains(obj))
                .collect();
            current = self.get_fwd_deps_set(&to_visit).to_vec();
            for t in to_visit {
                visited.add(t);
            }
        }
        Ok(visited)
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:690 PostAnalysisQueryEnvironment.buildTransitiveClosure
    fn build_transitive_closure(
        &self,
        _caller: &QueryExpression,
        _target_nodes: &dyn ThreadSafeMutableSet<T>,
        _max_depth: Option<i32>,
    ) -> Result<(), QueryError> {
        // TODO(bazel-team): implement this. Just needed for error-checking.
        Ok(())
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:696 PostAnalysisQueryEnvironment.getNodesOnPath
    fn get_nodes_on_path(
        &self,
        from: &T,
        to: &T,
        _context: &QueryExpressionContext<T>,
    ) -> Result<Vec<T>, InterruptedException> {
        // Java: com/google/devtools/build/lib/query2/SkyQueryUtils.java:63 SkyQueryUtils.getNodesOnPath
        let label = |t: &T| get_configured_target_key(t);
        // Tree of nodes visited so far.
        let mut node_to_parent: HashMap<ActionLookupKeyRef, Option<ActionLookupKeyRef>> =
            HashMap::new();
        let mut label_to_target: HashMap<ActionLookupKeyRef, T> = HashMap::new();
        // Contains all nodes left to visit in a (LIFO) stack.
        let mut to_visit: std::collections::VecDeque<T> = std::collections::VecDeque::new();
        to_visit.push_back(from.clone());
        node_to_parent.insert(label(from), None);
        label_to_target.insert(label(from), from.clone());
        while let Some(current) = to_visit.pop_front() {
            if label(to) == label(&current) {
                // Digraph.getPathToTreeNode(nodeToParent, label(to))
                let mut label_path: Vec<ActionLookupKeyRef> = Vec::new();
                let mut node = Some(label(to));
                while let Some(n) = node {
                    node = node_to_parent.get(&n).cloned().flatten();
                    label_path.push(n);
                }
                label_path.reverse();
                return Ok(label_path
                    .into_iter()
                    .map(|item| {
                        label_to_target
                            .get(&item)
                            .cloned()
                            .unwrap_or_else(|| panic!("NullPointerException: {}", item))
                    })
                    .collect());
            }
            for dep in self
                .get_fwd_deps_set(std::slice::from_ref(&current))
                .to_vec()
            {
                let dep_label = label(&dep);
                if !node_to_parent.contains_key(&dep_label) {
                    node_to_parent.insert(dep_label.clone(), Some(label(&current)));
                    label_to_target.insert(dep_label, dep.clone());
                    to_visit.push_front(dep);
                }
            }
        }
        // Note that the only current caller of this method checks first to see if there is a
        // path before calling this method. It is not clear what the return value should be here.
        // PORT: Java returns null; the Rust trait returns a list, so the empty list.
        Ok(Vec::new())
    }

    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:197 AbstractBlazeQueryEnvironment.eval
    fn eval(
        &self,
        expr: &QueryExpression,
        context: &QueryExpressionContext<T>,
        callback: &dyn Callback<T>,
    ) -> QueryTaskFuture<()> {
        // Not all QueryEnvironment implementations embrace the async+streaming evaluation
        // framework. In particular, the streaming callbacks employed by functions like 'deps' use
        // buildTransitiveClosure. So if the implementation of that method does some heavyweight
        // blocking work, then it's best to do this blocking work in a single batch. Importantly,
        // the callback we pass in needs to maintain order.
        let aggregate_all_callback =
            query_util::new_ordered_aggregate_all_output_formatter_callback(self);
        let eval_all_future = expr.eval(
            self,
            context,
            &blaze_query_engine::callback_fn(|partial_result: &[T]| {
                aggregate_all_callback.process(partial_result)
            }),
        );
        self.when_succeeds_call(&eval_all_future, || {
            callback.process(&aggregate_all_callback.get_result())
        })
    }

    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:263 AbstractBlazeQueryEnvironment.createEvaluateExpression
    fn create_evaluate_expression<'a>(
        &'a self,
        expr: &QueryExpression,
        context: &QueryExpressionContext<T>,
    ) -> Box<dyn EvaluateExpression<T> + 'a> {
        Box::new(EvaluateExpressionImpl {
            env: self,
            expression: expr.clone(),
            context: context.clone(),
            query_task_future_cancelled: Cell::new(None),
        })
    }

    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:557 ConfiguredTargetQueryEnvironment.createThreadSafeMutableSet
    fn create_thread_safe_mutable_set(&self) -> Arc<dyn ThreadSafeMutableSet<T>> {
        self.create_set()
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:706 PostAnalysisQueryEnvironment.createUniquifier
    fn create_uniquifier(&self) -> Box<dyn Uniquifier<T>> {
        Box::new(UniquifierImpl::new(configured_target_key_extractor::<T>()))
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:711 PostAnalysisQueryEnvironment.createMinDepthUniquifier
    fn create_min_depth_uniquifier(&self) -> Box<dyn MinDepthUniquifier<T>> {
        Box::new(MinDepthUniquifierImpl::new(
            configured_target_key_extractor::<T>(),
            default_thread_count(),
        ))
    }

    fn handle_error(
        &self,
        expression: &QueryExpression,
        message: &str,
        detailed_exit_code: Option<&DetailedExitCode>,
    ) -> Result<(), QueryException> {
        self.handle_error_impl(expression, message, detailed_exit_code)
    }

    // Java: com/google/devtools/build/lib/query2/PostAnalysisQueryEnvironment.java:721 PostAnalysisQueryEnvironment.getTransitiveLoadFilesHelper
    fn get_transitive_load_files_helper(
        &self,
    ) -> Result<Box<dyn TransitiveLoadFilesHelper<T> + '_>, QueryException> {
        Err(QueryException::with_configurable_query_code(
            None,
            "buildfiles() doesn't make sense for the configured target graph",
            configurable_query::Code::BuildfilesFunctionNotSupported,
        ))
    }

    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:502 AbstractBlazeQueryEnvironment.transitiveLoadFiles
    fn transitive_load_files(
        &self,
        _targets: &[T],
        _also_add_build_files: bool,
        _seen_packages: &Mutex<HashSet<PackageIdentifier>>,
        _seen_bzl_labels: &Mutex<HashSet<Label>>,
        _uniquifier: &dyn Uniquifier<T>,
        _helper: &dyn TransitiveLoadFilesHelper<T>,
        _callback: &dyn Callback<T>,
    ) -> Result<(), QueryError> {
        // PORT: unreachable: getTransitiveLoadFilesHelper always throws for post-analysis
        // queries, and the helper is required to call this.
        panic!("IllegalStateException: transitiveLoadFiles without a TransitiveLoadFilesHelper")
    }

    fn get_accessor(&self) -> &dyn TargetAccessor<T> {
        self
    }

    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:560 AbstractBlazeQueryEnvironment.isSettingEnabled
    fn is_setting_enabled(&self, setting: Setting) -> bool {
        self.settings.contains(&setting)
    }

    // Java: com/google/devtools/build/lib/query2/common/AbstractBlazeQueryEnvironment.java:565 AbstractBlazeQueryEnvironment.getFunctions
    fn get_functions(&self) -> Vec<Arc<dyn QueryFunction<T>>> {
        let mut builder = default_query_functions::<T>();
        builder.extend(self.extra_functions.iter().cloned());
        builder
    }
}

// -------------------------------------------------------------------------------------------
// The graph lookups of the environment.

/// The graph and configurations a post-analysis environment looks configured targets up in
/// (shared with `ConfigFunction`, which Java gives the environment itself by a cast).
#[derive(Clone)]
pub struct PostAnalysisGraph {
    pub graph: Arc<dyn WalkableGraph>,
    pub top_level_configurations: Arc<TopLevelConfigurations>,
    pub transitive_configurations: Arc<IndexMap<String, Arc<BuildConfigurationValue>>>,
}

impl PostAnalysisGraph {
    /// Java `getValueFromKey` of the environment of `T`.
    pub fn get_value_from_key<T: PostAnalysisNode>(&self, key: &SkyKey) -> Option<T> {
        T::get_value_from_key(&*self.graph, key)
    }

    /// Returns the node for the given label and configuration if it exists, else `None`.
    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:325 ConfiguredTargetQueryEnvironment.getConfiguredTarget
    pub fn get_configured_target<T: PostAnalysisNode>(
        &self,
        label: &Label,
        configuration: Option<&Arc<BuildConfigurationValue>>,
    ) -> Option<T> {
        let configuration_key: Option<BuildConfigurationKey> = configuration.map(|c| c.get_key());
        let target = self.get_value_from_key::<T>(&SkyKey::new(
            ConfiguredTargetKey::builder()
                .set_label(label.clone())
                .set_configuration_key(configuration_key.clone())
                .build(),
        ))?;
        // The configurations might not match if the target's configuration changed due to a
        // transition or trimming. Filters such targets.
        if configuration_key != target.get_configuration_key() {
            return None;
        }
        Some(target)
    }

    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:483 ConfiguredTargetQueryEnvironment.getTargetConfiguredTarget
    pub fn get_target_configured_target<T: PostAnalysisNode>(&self, label: &Label) -> Option<T> {
        if self.top_level_configurations.is_top_level_target(label) {
            self.get_configured_target::<T>(
                label,
                self.top_level_configurations
                    .get_configuration_for_top_level_target(label),
            )
        } else {
            for configuration in self.top_level_configurations.get_configurations() {
                if let Some(to_return) =
                    self.get_configured_target::<T>(label, configuration.as_ref())
                {
                    return Some(to_return);
                }
            }
            None
        }
    }

    // Java: com/google/devtools/build/lib/query2/cquery/ConfiguredTargetQueryEnvironment.java:501 ConfiguredTargetQueryEnvironment.getNullConfiguredTarget
    pub fn get_null_configured_target<T: PostAnalysisNode>(&self, label: &Label) -> Option<T> {
        self.get_configured_target::<T>(label, None)
    }
}

// -------------------------------------------------------------------------------------------
// GraphBackedRecursivePackageProvider.

/// A `RecursivePackageProvider` backed by a `WalkableGraph`, used by post-analysis queries
/// (`UniverseTargetPattern.all()`, `RecursivePkgValueRootPackageExtractor`).
// Java: com/google/devtools/build/lib/skyframe/GraphBackedRecursivePackageProvider.java:58 GraphBackedRecursivePackageProvider
pub struct GraphBackedRecursivePackageProvider<'g> {
    pub graph: &'g dyn WalkableGraph,
    pub pkg_roots: Vec<Root>,
}

impl GraphBackedRecursivePackageProvider<'_> {
    // Java: com/google/devtools/build/lib/skyframe/GraphBackedRecursivePackageProvider.java:131 GraphBackedRecursivePackageProvider.getPackageInternal
    fn get_package_internal(
        &self,
        package_name: &PackageIdentifier,
    ) -> Result<Option<Arc<Package>>, NoSuchPackageException> {
        let key = SkyKey::new(PackageKey::key(package_name.clone()));
        if let Some(pkg_value) = self.graph.get_value(&key) {
            let pkg_value = pkg_value
                .downcast::<PackageValue>()
                .unwrap_or_else(|_| panic!("PACKAGE value"));
            return Ok(Some(pkg_value.get_package().clone()));
        }
        if let Some(e) = self.graph.get_exception(&key) {
            if let Some(nspe) = e.downcast_ref::<NoSuchPackageException>() {
                return Err(nspe.clone());
            }
            panic!("ClassCastException: {}", e);
        }
        if self.graph.is_cycle(&key) {
            return Err(NoSuchPackageException::new(
                package_name.clone(),
                "Package depends on a cycle",
            ));
        }
        Ok(None)
    }

    // Java: com/google/devtools/build/lib/skyframe/GraphBackedRecursivePackageProvider.java:213 GraphBackedRecursivePackageProvider.bulkIsPackage
    fn bulk_is_package(
        &self,
        event_handler: &dyn ExtendedEventHandler,
        pkg_ids: &[PackageIdentifier],
    ) -> Vec<PackageIdentifier> {
        let mut result_builder: Vec<PackageIdentifier> = Vec::new();
        let mut non_successful_package_lookup_keys: Vec<SkyKey> = Vec::new();
        for pkg_id in pkg_ids {
            let key = SkyKey::new(PackageLookupKey::key(pkg_id.clone()));
            match self.graph.get_value(&key) {
                None => {
                    // Could be because the node legitimately isn't in the graph (which definitely
                    // implies the package doesn't exist) but could also be because the node
                    // exists but is in error. In the latter case, we want to print that error
                    // message out.
                    non_successful_package_lookup_keys.push(key);
                }
                Some(v) => {
                    let package_lookup_value = v
                        .downcast::<PackageLookupValue>()
                        .unwrap_or_else(|_| panic!("PACKAGE_LOOKUP value"));
                    if package_lookup_value.package_exists() {
                        result_builder.push(pkg_id.clone());
                    }
                }
            }
        }
        if !non_successful_package_lookup_keys.is_empty() {
            for (_, exception) in self
                .graph
                .get_missing_and_exceptions(&non_successful_package_lookup_keys)
            {
                if let Some(e) = exception
                    .as_ref()
                    .and_then(|e| e.downcast_ref::<NoSuchPackageException>())
                {
                    event_handler.handle(&Event::error(e.get_message()));
                }
            }
        }
        result_builder
    }

    // Java: com/google/devtools/build/lib/skyframe/GraphBackedRecursivePackageProvider.java:266 GraphBackedRecursivePackageProvider.checkValidDirectoryAndGetRoots
    fn check_valid_directory_and_get_roots(&self, repository: &RepositoryName) -> Vec<Root> {
        // Check that this package is covered by at least one of our universe patterns.
        // UniverseTargetPattern.all(): every package is in the universe.
        if repository.is_main() {
            self.pkg_roots.clone()
        } else {
            match self
                .graph
                .get_value(&SkyKey::new(RepositoryDirectoryKey::key(
                    repository.clone(),
                )))
                .map(|v| {
                    v.downcast::<RepositoryDirectoryValue>()
                        .unwrap_or_else(|_| panic!("REPOSITORY_DIRECTORY value"))
                })
                .as_deref()
            {
                Some(RepositoryDirectoryValue::Success { root, .. }) => vec![root.clone()],
                // If this key doesn't exist, the repository is outside the universe, so we
                // return "nothing".
                _ => Vec::new(),
            }
        }
    }
}

impl RecursivePackageProvider for GraphBackedRecursivePackageProvider<'_> {
    // Java: com/google/devtools/build/lib/skyframe/GraphBackedRecursivePackageProvider.java:119 GraphBackedRecursivePackageProvider.getPackage
    fn get_package(
        &self,
        _event_handler: &dyn ExtendedEventHandler,
        package_name: &PackageIdentifier,
    ) -> Result<Arc<Package>, ProviderError> {
        if let Some(pkg) = self.get_package_internal(package_name)? {
            return Ok(pkg);
        }
        // If the package key does not exist in the graph, then it must not correspond to any
        // package, because the SkyQuery environment has already loaded the universe.
        Err(NoSuchPackageException::build_file_not_found(
            package_name.clone(),
            "BUILD file not found on package path",
        )
        .into())
    }

    // Java: com/google/devtools/build/lib/skyframe/GraphBackedRecursivePackageProvider.java:258 GraphBackedRecursivePackageProvider.isPackage
    fn is_package(
        &self,
        event_handler: &dyn ExtendedEventHandler,
        package_id: &PackageIdentifier,
    ) -> Result<bool, ProviderError> {
        Ok(!self
            .bulk_is_package(event_handler, std::slice::from_ref(package_id))
            .is_empty())
    }

    // Java: com/google/devtools/build/lib/skyframe/GraphBackedRecursivePackageProvider.java:300 GraphBackedRecursivePackageProvider.streamPackagesUnderDirectory
    fn stream_packages_under_directory(
        &self,
        results: &mut dyn FnMut(Vec<PackageIdentifier>),
        _event_handler: &dyn ExtendedEventHandler,
        repository: &RepositoryName,
        directory: &PathFragment,
        ignored_subdirectories: &IgnoredSubdirectories,
        excluded_subdirectories: &[PathFragment],
    ) -> Result<(), ProviderError> {
        // Java: com/google/devtools/build/lib/skyframe/RecursivePkgValueRootPackageExtractor.java:36 RecursivePkgValueRootPackageExtractor.streamPackagesFromRoots
        let roots = self.check_valid_directory_and_get_roots(repository);
        let filtered_ignored_subdirectories =
            ignored_subdirectories.filter_for_directory(directory);

        for root in roots {
            let rooted_path = RootedPath::to_rooted_path(root, directory);
            let lookup = self.graph.get_value(&SkyKey::new(RecursivePkgKey::key(
                repository.clone(),
                rooted_path.clone(),
                filtered_ignored_subdirectories.clone(),
            )));
            let Some(lookup) = lookup else {
                // A null lookup should only happen during post-analysis queries which have
                // access to --universe_scope logic.
                // PORT: Java throws a QueryException (TARGET_NOT_IN_UNIVERSE_SCOPE); the provider
                // error carries its message (mapped back in get_targets_matching_pattern_impl).
                return Err(ProviderError::Eval(
                    TargetPatternEvalError::ProcessPackageDirectory(format!(
                        "Unable to load package '{}' because package is not in scope. Check that \
                         all target patterns in query expression are within the --universe_scope \
                         of this query.",
                        rooted_path
                    )),
                ));
            };
            let lookup = lookup
                .downcast::<RecursivePkgValue>()
                .unwrap_or_else(|_| panic!("RECURSIVE_PKG value"));
            let mut package_ids: Vec<PackageIdentifier> = Vec::new();
            for package_name in lookup.get_packages() {
                // TODO(bazel-team): Make RecursivePkgValue return NestedSet<PathFragment> so this
                // transform is unnecessary.
                let package_name_path_fragment = PathFragment::create(package_name);
                if !excluded_subdirectories
                    .iter()
                    .any(|e| package_name_path_fragment.starts_with(e))
                {
                    package_ids.push(PackageIdentifier::create(
                        repository.clone(),
                        package_name_path_fragment,
                    ));
                }
            }
            results(package_ids);
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use blaze_analysis::aspect_key_creator::AspectKeyCreator;

    // Java: src/test/java/com/google/devtools/build/lib/query2/PostAnalysisQueryTest.java
    // PORT: the PostAnalysisQueryTest suite drives analysis through BuildViewTestCase.
    #[test]
    #[ignore = "needs the BuildViewTestCase analysis harness"]
    fn test_target_literal_with_missing_targets() {}

    // Oracle (scripts/w4/scenarios/query-errors, `cquery //:x --nodep_deps=false`): "The following
    // filter(s) are not currently supported by configured query: [NO_NODEP_DEPS,
    // INCLUDE_ASPECTS]" (the EnumSet's ordinal order).
    #[test]
    fn check_settings_lists_the_other_settings_in_ordinal_order() {
        let settings: HashSet<Setting> = [
            Setting::IncludeAspects,
            Setting::NoNodepDeps,
            Setting::NoImplicitDeps,
        ]
        .into_iter()
        .collect();
        let e = check_settings(&settings).unwrap_err();
        assert_eq!(
            e.get_message(),
            "The following filter(s) are not currently supported by configured query: [NO_NODEP_DEPS, INCLUDE_ASPECTS]"
        );
    }

    #[test]
    fn check_settings_rejects_nodep_filtering() {
        let settings: HashSet<Setting> = [Setting::NoNodepDeps, Setting::OnlyTargetDeps]
            .into_iter()
            .collect();
        let e = check_settings(&settings).unwrap_err();
        assert_eq!(
            e.get_message(),
            "The following filter(s) are not currently supported by configured query: [NO_NODEP_DEPS]"
        );
    }

    /// `BuildConfigurationKey.create(BuildOptions.builder().build())`: empty options, whose
    /// checksum is 64 zeros.
    fn empty_configuration_key() -> BuildConfigurationKey {
        let key = BuildConfigurationKey::create(Arc::new(
            blaze_build_config::build_options::BuildOptions::builder().build(),
        ));
        assert_eq!(key.get_options_checksum(), "0".repeat(64));
        key
    }

    // Expected values: `hashCode()` of the keys built by Bazel 9.3.2's own classes (the oracle's
    // A-server.jar run on JDK 25): ConfiguredTargetKey.builder().setLabel(label) with, in this
    // order, (1) no configuration key, (2) BuildConfigurationKey.create(BuildOptions.builder()
    // .build()), (3) that and setExecutionPlatformLabel(@@platforms//host:host), (4) as 2 with
    // setShouldApplyRuleTransition(false), (5) as 3 with setShouldApplyRuleTransition(false).
    // The last label is Java's internal form of a non-ASCII name (Latin-1 characters holding the
    // UTF-8 bytes).
    #[test]
    fn configured_target_key_hash_codes_match_java() {
        let cases: [(&str, [i32; 5]); 6] = [
            (
                "//:brotli",
                [-178071824, 954342065, 1214327892, -954342066, -1214327893],
            ),
            (
                "//a/b:c",
                [983405715, 2115819604, -1919161865, -2115819605, 1919161864],
            ),
            (
                "@@rules_cc+//cc:defs.bzl",
                [-316621397, 815792492, 1075778319, -815792493, -1075778320],
            ),
            (
                "@@platforms//host:host",
                [738306370, 1870720259, 2130706086, -1870720260, -2130706087],
            ),
            (
                "@@rules_python++python+python_3_11_x86_64-unknown-linux-gnu//:lib/python3.11/site-packages/setuptools/script (dev).tmpl",
                [765386214, 1897800103, -2137181366, -1897800104, 2137181365],
            ),
            (
                "//pkg:\u{e4}",
                [1181073805, -1981479602, -1721493775, 1981479601, 1721493774],
            ),
        ];
        let configuration_key = empty_configuration_key();
        let execution_platform_label = Label::parse_canonical_unchecked("@@platforms//host:host");
        for (raw, expected) in cases {
            let label = Label::parse_canonical_unchecked(raw);
            let builder = || ConfiguredTargetKey::builder().set_label(label.clone());
            let keys = [
                builder().build(),
                builder()
                    .set_configuration_key(Some(configuration_key.clone()))
                    .build(),
                builder()
                    .set_configuration_key(Some(configuration_key.clone()))
                    .set_execution_platform_label(Some(execution_platform_label.clone()))
                    .build(),
                builder()
                    .set_configuration_key(Some(configuration_key.clone()))
                    .set_should_apply_rule_transition(false)
                    .build(),
                builder()
                    .set_configuration_key(Some(configuration_key.clone()))
                    .set_execution_platform_label(Some(execution_platform_label.clone()))
                    .set_should_apply_rule_transition(false)
                    .build(),
            ];
            for (key, expected) in keys.into_iter().zip(expected) {
                assert_eq!(
                    java_hash_code_of_configured_target_key(&key),
                    expected,
                    "{:?}",
                    key
                );
                assert_eq!(sky_key_java_hash_code(&SkyKey::new(key.clone())), expected);
                assert_eq!(
                    JavaActionLookupKey(ActionLookupKeyRef::new(key)).java_hash_code(),
                    expected
                );
            }
        }
    }

    // Expected values: `hashCode()` of ToolchainContextKey.key().configurationKey(
    // BuildConfigurationKey.create(BuildOptions.builder().build()))...build() computed by Bazel
    // 9.3.2's own classes (the oracle's A-server.jar run on JDK 25).
    #[test]
    fn toolchain_context_key_hash_codes_match_java() {
        use blaze_analysis::toolchains::ToolchainTypeRequirement;
        let configuration_key = empty_configuration_key();
        // ToolchainTypeRequirement.create(@@bazel_tools//tools/cpp:toolchain_type)
        let mandatory = ToolchainTypeRequirement {
            toolchain_type: Label::parse_canonical_unchecked(
                "@@bazel_tools//tools/cpp:toolchain_type",
            ),
            mandatory: true,
            ignore_if_invalid: false,
        };
        // ToolchainTypeRequirement.builder(...default_test_toolchain_type).mandatory(false).build()
        let optional = ToolchainTypeRequirement {
            toolchain_type: Label::parse_canonical_unchecked(
                "@@bazel_tools//tools/test:default_test_toolchain_type",
            ),
            mandatory: false,
            ignore_if_invalid: false,
        };
        let hash_code = |configure: &dyn Fn(
            &mut blaze_analysis::toolchains::toolchain_context_key::ToolchainContextKeyBuilder,
        )| {
            let mut builder = ToolchainContextKey::key();
            builder.configuration_key(configuration_key.clone());
            configure(&mut builder);
            let key = builder.build();
            let hash_code = java_hash_code_of_toolchain_context_key(&key);
            assert_eq!(
                sky_key_java_hash_code_if_known(&SkyKey::new(key)),
                Some(hash_code)
            );
            hash_code
        };
        assert_eq!(hash_code(&|_| {}), -1093658679);
        assert_eq!(
            hash_code(&|b| {
                b.toolchain_types([mandatory.clone()]);
            }),
            -1381495969
        );
        assert_eq!(
            hash_code(&|b| {
                b.toolchain_types([mandatory.clone(), optional.clone()]);
            }),
            1840457987
        );
        assert_eq!(
            hash_code(&|b| {
                b.toolchain_types([mandatory.clone()])
                    .exec_constraint_labels([
                        Label::parse_canonical_unchecked("@@platforms//os:linux"),
                        Label::parse_canonical_unchecked("@@platforms//cpu:x86_64"),
                    ]);
            }),
            -1457625024
        );
        assert_eq!(
            hash_code(&|b| {
                b.toolchain_types([mandatory.clone()])
                    .force_execution_platform(Label::parse_canonical_unchecked(
                        "@@platforms//host:host",
                    ));
            }),
            -865283406
        );
        assert_eq!(
            hash_code(&|b| {
                b.toolchain_types([optional.clone()]).debug_target(true);
            }),
            1020521479
        );
    }

    // Expected values: Guava 33.5.0's Maps.capacity(expectedSize), called by reflection on the
    // oracle's A-server.jar.
    #[test]
    fn guava_maps_capacity_matches_guava() {
        let cases: [(usize, i32); 20] = [
            (0, 1),
            (1, 2),
            (2, 3),
            (3, 4),
            (4, 6),
            (5, 7),
            (6, 8),
            (7, 10),
            (8, 11),
            (9, 12),
            (12, 16),
            (13, 18),
            (15, 20),
            (16, 22),
            (24, 32),
            (25, 34),
            (48, 64),
            (49, 66),
            (100, 134),
            (3295, 4394),
        ];
        for (expected_size, capacity) in cases {
            assert_eq!(
                guava_maps_capacity(expected_size),
                capacity,
                "{expected_size}"
            );
        }
    }

    /// `Integer.hashCode()`.
    fn integer_hash_code(i: &i32) -> i32 {
        *i
    }

    // Expected values: the JVM of the oracle's JDK (25). For Integer keys put in the given order,
    // the key order of Maps.newHashMapWithExpectedSize(n); then of `new HashMap<>()` filled in
    // that order followed by `new HashMap<>(size)` filled in the order of the former (what
    // InMemoryGraphImpl.getBatchMap and DelegatingWalkableGraph.getDirectDeps do).
    #[test]
    fn hash_map_orders_of_get_fwd_deps_match_java() {
        let cases: [(&[i32], &[i32], &[i32]); 3] = [
            (&[33, 1, 17, 2], &[33, 1, 17, 2], &[33, 1, 17, 2]),
            (&[9, 1, 5, 13, 3], &[9, 1, 3, 5, 13], &[1, 9, 3, 5, 13]),
            (
                &[64, 0, 32, 16, 48, 8, 5],
                &[64, 0, 32, 16, 48, 5, 8],
                &[64, 0, 32, 16, 48, 5, 8],
            ),
        ];
        for (keys, expected_targets_by_key, expected_graph_map) in cases {
            let targets_by_key =
                targets_by_key_order(keys.len(), keys.to_vec(), &integer_hash_code);
            assert_eq!(targets_by_key, expected_targets_by_key);
            let graph_map: Vec<i32> = delegating_walkable_graph_map_order(
                targets_by_key.into_iter().map(|k| (k, ())).collect(),
                &integer_hash_code,
            )
            .into_iter()
            .map(|(k, ())| k)
            .collect();
            assert_eq!(graph_map, expected_graph_map);
        }
    }

    // Expected values: iteration order of `new HashSet<Integer>()` after adding the elements in
    // the given order, on the JVM of the oracle's JDK (25).
    #[test]
    fn java_hash_set_order_of_known_matches_java() {
        let cases: [(&[i32], &[i32]); 6] = [
            (&[17, 1, 33], &[17, 1, 33]),
            (&[5, 2, 20], &[2, 20, 5]),
            (&[100, 36, 4, 68, 7, 23], &[100, 36, 4, 68, 7, 23]),
            (
                &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 29, 45],
                &[1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12, 13, 45, 29],
            ),
            (
                &[65536, 1, 131072, 17, 196608],
                &[65536, 1, 17, 131072, 196608],
            ),
            (
                &[40, 8, 24, 56, 9, 72, 25, 41, 57, 10, 26, 42, 58, 11],
                &[40, 8, 72, 9, 41, 10, 42, 11, 24, 56, 25, 57, 26, 58],
            ),
        ];
        for (elements, expected) in cases {
            let got =
                java_hash_set_order_of_known(elements.iter().map(|e| (*e, Some(*e))).collect());
            assert_eq!(got, expected);
            // An element added twice changes nothing.
            let got = java_hash_set_order_of_known(
                elements
                    .iter()
                    .chain(elements.iter())
                    .map(|e| (*e, Some(*e)))
                    .collect(),
            );
            assert_eq!(got, expected);
        }
        // Elements of unknown hash code count for the growth of the table only: the last set
        // without the hash codes of 72 and 25 is Java's order without those two elements.
        let elements: [i32; 14] = [40, 8, 24, 56, 9, 72, 25, 41, 57, 10, 26, 42, 58, 11];
        let got = java_hash_set_order_of_known(
            elements
                .iter()
                .map(|e| (*e, if *e == 72 || *e == 25 { None } else { Some(*e) }))
                .collect(),
        );
        assert_eq!(got, [40, 8, 9, 41, 10, 42, 11, 24, 56, 57, 26, 58]);
    }

    // A bin of 9 nodes makes java.util.HashMap resize or build a tree; whether the elements of
    // unknown hash code produce one cannot be told, so the order is refused.
    #[test]
    #[should_panic(expected = "HashMap.treeifyBin")]
    fn java_hash_set_order_of_known_refuses_possible_tree_bins() {
        java_hash_set_order_of_known((0..9).map(|e: i32| (e, None)).collect());
    }

    // ---------------------------------------------------------------------------------------
    // AspectKey.hashCode (AspectKeyCreator.java:116, AspectDescriptor.java:63,
    // StarlarkAspectClass.java:69, BzlLoadValue.java:190, AspectParameters.java:118).
    //
    // Expected values derived by hand from the Java formulas (all arithmetic mod 2^32, as Java
    // int); `C` is the identity hash code of the `BzlLoadValue.Key` subclass, taken as 0 here
    // (`identity_zero`) so the arithmetic stays short:
    //
    //   "" .hashCode() = 0, "a" = 97, "b" = 98, "x" = 120, "y" = 121, "p" = 112, "v" = 118
    //   RepositoryName(main) = hashObjects("", null, null) = 31 * (31 * (0 + 31)) = 29791
    //   PackageIdentifier(//) = hashObjects(29791, "") = 0 + 31 * (29791 + 31) = 924482
    //   Label //:a = 31 * (31 + 97) + 924482 = 928450;  //:b = 31 * (31 + 98) + 924482 = 928481
    //   ConfiguredTargetKey(//:a, no configuration) = hashObjects(928450, null, null)
    //     = 31 * (31 * (928450 + 31)) = 961 * 928481 = 892270241
    //   KeyForBuild(//:b).hashCode():
    //     hashObjects(C = 0, 928481) = 928481 + 31 * (0 + 31) = 929442
    //     31 * 929442 + Boolean.hashCode(false) = 28812702 + 1237 = 28813939
    //     31 * 28813939 + Boolean.hashCode(false) = 893232109 + 1237 = 893233346
    //   StarlarkAspectClass(//:b, "x") = hashObjects(893233346, 120)
    //     = 120 + 31 * 893233377 = 27690234807 = 27690234807 - 6 * 2^32 = 1920431031
    //   StarlarkAspectClass(//:b, "y") = 1920431031 + 1 = 1920431032
    //   AspectParameters.EMPTY = 0;  {p=[v]} = 112 ^ (31 * 1 + 118) = 112 ^ 149 = 229
    //   AspectDescriptor(x, EMPTY) = hashObjects(1920431031, 0) = 31 * 1920431062
    //     = 59533362922 - 13 * 2^32 = 3698788074 -> -596179222 (as int)
    //   AspectDescriptor(x, {p=[v]}) = -596179222 + 229 = -596178993
    //   AspectDescriptor(y, EMPTY) = 31 * 1920431063 - 13 * 2^32 -> -596179191
    //   SimpleAspectKey(//:a, x) = hashObjects(892270241, -596179222)
    //     = -596179222 + 31 * 892270272 = 27064199210 - 6 * 2^32 = 1294395434
    //   AspectKeyWithBaseAspects(//:a, y, [that key]):
    //     ImmutableList.hashCode() = 31 * 1 + 1294395434 = 1294395465
    //     hashObjects(892270241, 1294395465, -596179191)
    //       = -596179191 + 31 * (1294395465 + 31 * (892270241 + 31)) -> -646353248
    //   With C = 0xBAD = 2989 (`java_class_identity_hash_code`): C enters
    //     hashObjects(C, label) times 31, then is multiplied by 31 in each of the next four steps
    //     (bzl key twice, class, descriptor) and added unmultiplied to the key: the key grows by
    //     31^5 * 2989 = 28629151 * 2989 = 85572532339 -> 1294395434 + 85572532339 - 20 * 2^32
    //     = 967581853.
    //   KeyForBuiltins(//:b) (isBuiltins true): 893232109 + 1231 = 893233340.

    fn identity_zero(_: BzlLoadKeyClass) -> i32 {
        0
    }

    fn aspect_descriptor(exported_name: &str, parameters: AspectParameters) -> AspectDescriptor {
        AspectDescriptor::of(
            AspectClass::Starlark(StarlarkAspectClass::new(
                Label::parse_canonical_unchecked("//:b"),
                exported_name,
            )),
            parameters,
        )
    }

    fn base_configured_target_key() -> ConfiguredTargetKey {
        ConfiguredTargetKey::builder()
            .set_label(Label::parse_canonical_unchecked("//:a"))
            .build()
    }

    #[test]
    fn bzl_load_key_hash_codes_match_java() {
        let label = Label::parse_canonical_unchecked("//:b");
        assert_eq!(label.java_hash_code(), 928481);
        assert_eq!(
            bzl_load_key_java_hash_code(BzlLoadKeyClass::KeyForBuild, &label, &identity_zero),
            893233346
        );
        assert_eq!(
            bzl_load_key_java_hash_code(BzlLoadKeyClass::KeyForBuiltins, &label, &identity_zero),
            893233340
        );
    }

    #[test]
    fn aspect_parameters_hash_codes_match_java() {
        assert_eq!(
            aspect_parameters_java_hash_code(&AspectParameters::empty()),
            0
        );
        let parameters = AspectParameters::builder().add_attribute("p", "v").build();
        assert_eq!(aspect_parameters_java_hash_code(&parameters), 229);
    }

    #[test]
    fn aspect_descriptor_hash_codes_match_java() {
        let x = aspect_descriptor("x", AspectParameters::empty());
        let AspectClass::Starlark(class) = x.get_aspect_class() else {
            unreachable!()
        };
        assert_eq!(
            starlark_aspect_class_java_hash_code(class, &identity_zero),
            1920431031
        );
        assert_eq!(
            aspect_descriptor_java_hash_code(&x, &identity_zero),
            -596179222
        );
        let x_with_parameters = aspect_descriptor(
            "x",
            AspectParameters::builder().add_attribute("p", "v").build(),
        );
        assert_eq!(
            aspect_descriptor_java_hash_code(&x_with_parameters, &identity_zero),
            -596178993
        );
        let y = aspect_descriptor("y", AspectParameters::empty());
        assert_eq!(
            aspect_descriptor_java_hash_code(&y, &identity_zero),
            -596179191
        );
    }

    #[test]
    fn aspect_key_hash_codes_match_java() {
        assert_eq!(
            java_hash_code_of_configured_target_key(&base_configured_target_key()),
            892270241
        );
        let simple = AspectKeyCreator::create_aspect_key(
            aspect_descriptor("x", AspectParameters::empty()),
            Vec::new(),
            base_configured_target_key(),
        );
        assert_eq!(
            aspect_key_java_hash_code(&simple, &identity_zero),
            1294395434
        );
        let with_base_aspects = AspectKeyCreator::create_aspect_key(
            aspect_descriptor("y", AspectParameters::empty()),
            vec![simple.clone()],
            base_configured_target_key(),
        );
        assert_eq!(
            aspect_key_java_hash_code(&with_base_aspects, &identity_zero),
            -646353248
        );
        // The query sets and maps use the stand-in identity hash code.
        assert_eq!(
            aspect_key_java_hash_code(&simple, &java_class_identity_hash_code),
            967581853
        );
        assert_eq!(
            JavaActionLookupKey(ActionLookupKeyRef::new(simple.clone())).java_hash_code(),
            967581853
        );
        assert_eq!(
            sky_key_java_hash_code(&SkyKey::new(simple.clone())),
            967581853
        );
        assert_eq!(
            sky_key_java_hash_code_if_known(&SkyKey::new(simple)),
            Some(967581853)
        );
    }

    #[test]
    fn check_settings_accepts_supported_filters() {
        let settings: HashSet<Setting> = [Setting::OnlyTargetDeps, Setting::NoImplicitDeps]
            .into_iter()
            .collect();
        assert!(check_settings(&settings).is_ok());
    }
}
