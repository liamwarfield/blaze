//! Port of `query2/engine/QueryEnvironment.java` and its nested types.
//!
//! PORT: `QueryEnvironment`'s nested types keep their simple Java names (`Argument`,
//! `ArgumentType`, `QueryFunction`, `Setting`, `TargetAccessor`, `ThreadSafeMutableSet`, ...):
//! Java code always imports them by simple name and they are unique within `query2`.
//!
//! PORT: Java's `QueryFunction.eval` is generic over the node type (`<T> eval(QueryEnvironment<T>
//! env, ...)`), which a Rust trait object cannot be. A function is split into the `T`-independent
//! [`QueryFunctionBase`] (name and signature, what the parser and visitors use) and
//! [`QueryFunction<T>`] (evaluation). Built-in functions implement `QueryFunction<T>` for every
//! `T`; a parsed `FunctionExpression` remembers the `QueryFunction<T>` of the environment that
//! parsed it and must be evaluated in an environment with the same `T` (as every Bazel caller
//! does: queries are parsed with the environment that evaluates them).

use std::collections::HashSet;
use std::fmt;
use std::sync::{Arc, Mutex};

use blaze_cmdline::{Label, PackageIdentifier};

use crate::all_paths_function::AllPathsFunction;
use crate::attr_function::AttrFunction;
use crate::build_files_function::BuildFilesFunction;
use crate::callback::Callback;
use crate::deps_function::DepsFunction;
use crate::detailed_exit_code::DetailedExitCode;
use crate::executables_function::ExecutablesFunction;
use crate::filter_function::FilterFunction;
use crate::kind_function::KindFunction;
use crate::labels_function::LabelsFunction;
use crate::load_files_function::LoadFilesFunction;
use crate::min_depth_uniquifier::MinDepthUniquifier;
use crate::query_exception::{InterruptedException, QueryError, QueryException};
use crate::query_expression::QueryExpression;
use crate::query_expression_context::QueryExpressionContext;
use crate::query_task_future::QueryTaskFuture;
use crate::query_visibility::QueryVisibility;
use crate::rdeps_function::RdepsFunction;
use crate::same_pkg_direct_rdeps_function::SamePkgDirectRdepsFunction;
use crate::siblings_function::SiblingsFunction;
use crate::some_function::SomeFunction;
use crate::some_path_function::SomePathFunction;
use crate::streamable_query_environment::StreamableQueryEnvironment;
use crate::tests_function::TestsFunction;
use crate::uniquifier::Uniquifier;
use crate::visible_function::VisibleFunction;

/// Type of an argument of a user-defined query function.
// Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:49 QueryEnvironment.ArgumentType
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ArgumentType {
    Expression,
    Word,
    Integer,
}

/// Value of an argument of a user-defined query function.
// Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:56 QueryEnvironment.Argument
#[derive(Clone)]
pub struct Argument {
    type_: ArgumentType,
    expression: Option<QueryExpression>,
    word: Option<String>,
    integer: i32,
}

impl Argument {
    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:69 Argument.of
    pub fn of_expression(expression: QueryExpression) -> Argument {
        Argument {
            type_: ArgumentType::Expression,
            expression: Some(expression),
            word: None,
            integer: 0,
        }
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:73 Argument.of
    pub fn of_word(word: impl Into<String>) -> Argument {
        Argument {
            type_: ArgumentType::Word,
            expression: None,
            word: Some(word.into()),
            integer: 0,
        }
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:77 Argument.of
    pub fn of_integer(integer: i32) -> Argument {
        Argument {
            type_: ArgumentType::Integer,
            expression: None,
            word: None,
            integer,
        }
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:81 Argument.getType
    pub fn get_type(&self) -> ArgumentType {
        self.type_
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:85 Argument.getExpression
    pub fn get_expression(&self) -> &QueryExpression {
        self.expression
            .as_ref()
            .unwrap_or_else(|| panic!("NullPointerException: Expected expression argument"))
    }

    /// Java returns null for a non-WORD argument; every caller asks a WORD argument.
    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:89 Argument.getWord
    pub fn get_word(&self) -> &str {
        self.word
            .as_deref()
            .unwrap_or_else(|| panic!("NullPointerException: not a WORD argument"))
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:93 Argument.getInteger
    pub fn get_integer(&self) -> i32 {
        self.integer
    }
}

impl fmt::Display for Argument {
    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:98 Argument.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.type_ {
            ArgumentType::Word => write!(f, "'{}'", self.word.as_deref().unwrap_or("null")),
            ArgumentType::Expression => write!(f, "{}", self.get_expression()),
            ArgumentType::Integer => write!(f, "{}", self.integer),
        }
    }
}

impl fmt::Debug for Argument {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

/// The `T`-independent part of a user-defined query function (see the module documentation).
// Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:108 QueryEnvironment.QueryFunction
pub trait QueryFunctionBase: Send + Sync {
    /// Name of the function as it appears in the query language.
    fn get_name(&self) -> String;

    /// The number of arguments that are required. The rest is optional.
    ///
    /// This should be greater than or equal to zero and at smaller than or equal to the length of
    /// the list returned by `get_argument_types`.
    fn get_mandatory_arguments(&self) -> i32;

    /// The types of the arguments of the function.
    fn get_argument_types(&self) -> Vec<ArgumentType>;

    /// Whether [`QueryFunctionBase::get_argument_types`] repeats forever (Java functions that
    /// return `Iterables.cycle(...)`, e.g. `rbuildfiles`: any number of arguments).
    fn get_argument_types_cycle(&self) -> bool {
        false
    }
}

/// A user-defined query function.
// Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:108 QueryEnvironment.QueryFunction
pub trait QueryFunction<T>: QueryFunctionBase {
    /// Returns a `QueryTaskFuture` representing the asynchronous application of this
    /// `QueryFunction` to the given `args`, feeding the results to the given `callback`.
    ///
    /// - `env`: the query environment this function is evaluated in.
    /// - `expression`: the expression being evaluated.
    /// - `context`: the context relevant to the expression being evaluated. Contains the variable
    ///   bindings from `LetExpression`s.
    /// - `args`: the input arguments. These are type-checked against the specification returned
    ///   by `get_argument_types` and `get_mandatory_arguments`.
    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:134 QueryFunction.eval
    fn eval(
        &self,
        env: &dyn QueryEnvironment<T>,
        context: &QueryExpressionContext<T>,
        expression: &QueryExpression,
        args: &[Argument],
        callback: &dyn Callback<T>,
    ) -> QueryTaskFuture<()>;

    /// A filtering function is one whose outputs are a subset of a single input argument. Returns
    /// the function as a filtering function if it is one and `None` otherwise.
    ///
    /// PORT: Java's `FilteringQueryFunction` makes this `final` and returns `this`; Rust
    /// implementors of [`FilteringQueryFunction`] override it to return `Some(self)`.
    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:146 QueryFunction.asFilteringFunction
    fn as_filtering_function(&self) -> Option<&dyn FilteringQueryFunction<T>> {
        None
    }
}

/// A `QueryFunction` whose output is a subset of some input argument expression.
// Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:152 QueryEnvironment.FilteringQueryFunction
pub trait FilteringQueryFunction<T>: QueryFunction<T> {
    /// Returns a function representing the filter but inverted.
    fn invert(&self) -> Arc<dyn FilteringQueryFunction<T>>;

    /// Returns the argument index of the expression that is used as the input to be filtered.
    fn get_expression_to_filter_index(&self) -> i32;
}

/// Exception type for the case where a target cannot be found. It's basically a wrapper for
/// whatever exception is internally thrown.
// Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:175 QueryEnvironment.TargetNotFoundException
#[derive(Debug, Clone)]
pub struct TargetNotFoundException {
    message: String,
    cause: Arc<dyn std::error::Error + Send + Sync>,
    detailed_exit_code: DetailedExitCode,
}

impl TargetNotFoundException {
    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:178 TargetNotFoundException.<init>
    pub fn new(
        cause: Arc<dyn std::error::Error + Send + Sync>,
        detailed_exit_code: DetailedExitCode,
    ) -> TargetNotFoundException {
        TargetNotFoundException {
            message: cause.to_string(),
            cause,
            detailed_exit_code,
        }
    }

    pub fn get_message(&self) -> &str {
        &self.message
    }

    pub fn get_cause(&self) -> &Arc<dyn std::error::Error + Send + Sync> {
        &self.cause
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:183 TargetNotFoundException.getDetailedExitCode
    pub fn get_detailed_exit_code(&self) -> &DetailedExitCode {
        &self.detailed_exit_code
    }
}

impl fmt::Display for TargetNotFoundException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for TargetNotFoundException {}

/// QueryEnvironment implementations can optionally also implement this interface to provide
/// custom implementations of various operators.
// Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:192 QueryEnvironment.CustomFunctionQueryEnvironment
pub trait CustomFunctionQueryEnvironment<T>: QueryEnvironment<T> {
    /// Returns a `QueryTaskFuture` representing the asynchronous evaluation of the given `expr`
    /// and passing of the results to the given `callback`.
    ///
    /// This provides callers the option to decide whether or not to batch futures resulting from
    /// the given `expr`. `batch`: whether or not to invoke the callback with a single batch of
    /// the result set.
    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:204 CustomFunctionQueryEnvironment.eval
    fn eval_with_batch(
        &self,
        expr: &QueryExpression,
        context: &QueryExpressionContext<T>,
        callback: &dyn Callback<T>,
        batch: bool,
    ) -> QueryTaskFuture<()>;

    /// Computes the transitive closure of dependencies at most maxDepth away from the given
    /// targets (if set), and calls the given callback with the results.
    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:214 CustomFunctionQueryEnvironment.deps
    fn deps(
        &self,
        from: &[T],
        max_depth: Option<i32>,
        caller: &QueryExpression,
        callback: &dyn Callback<T>,
    ) -> Result<(), QueryError>;

    /// Computes some path from a node in 'from' to a node in 'to'.
    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:218 CustomFunctionQueryEnvironment.somePath
    fn some_path(
        &self,
        from: &[T],
        to: &[T],
        caller: &QueryExpression,
        callback: &dyn Callback<T>,
    ) -> Result<(), QueryError>;

    /// Computes all paths from a node in 'from' to a node in 'to'.
    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:222 CustomFunctionQueryEnvironment.allPaths
    fn all_paths(
        &self,
        from: &[T],
        to: &[T],
        caller: &QueryExpression,
        callback: &dyn Callback<T>,
    ) -> Result<(), QueryError>;

    /// Computes all reverse dependencies of a node in 'from' with at most distance maxDepth
    /// within the transitive closure of 'universe'.
    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:229 CustomFunctionQueryEnvironment.rdeps
    fn rdeps(
        &self,
        from: &[T],
        universe: &[T],
        max_depth: Option<i32>,
        caller: &QueryExpression,
        callback: &dyn Callback<T>,
    ) -> Result<(), QueryError>;

    /// Computes direct reverse deps of all nodes in 'from' within the same package.
    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:238 CustomFunctionQueryEnvironment.samePkgDirectRdeps
    fn same_pkg_direct_rdeps(
        &self,
        from: &[T],
        caller: &QueryExpression,
        callback: &dyn Callback<T>,
    ) -> Result<(), QueryError>;
}

/// The environment of a Blaze query. Implementations do not need to be thread-safe. The generic
/// type T represents a node of the graph on which the query runs; as such, there is no
/// restriction on T. However, query assumes a certain graph model, and the [`TargetAccessor`]
/// class is used to access properties of these nodes. Also, the query engine doesn't assume T's
/// `Hash` and `Eq` are meaningful and instead uses [`QueryEnvironment::create_uniquifier`] and
/// [`QueryEnvironment::create_thread_safe_mutable_set`] when appropriate.
///
/// PORT: the future-composition methods (`immediateSuccessfulFuture`, `execute`,
/// `whenSucceedsCall`, `transformAsync`, ...) are the provided methods of
/// [`crate::query_task_future::QueryEnvironmentFutures`], implemented for every environment.
///
/// PORT: Java's `instanceof StreamableQueryEnvironment` / `instanceof
/// CustomFunctionQueryEnvironment` checks are [`QueryEnvironment::as_streamable`] and
/// [`QueryEnvironment::as_custom_function_query_environment`].
///
/// PORT: `getLabelPrinter()` returns `packages.LabelPrinter`, which this crate cannot name; it is
/// declared on blaze_query's `AbstractBlazeQueryEnvironment`. No engine code calls it.
// Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:47 QueryEnvironment
pub trait QueryEnvironment<T> {
    /// Returns all of the targets in `target`'s package, in some stable order.
    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:243 QueryEnvironment.getSiblingTargetsInPackage
    fn get_sibling_targets_in_package(&self, target: &T) -> Result<Vec<T>, QueryError>;

    /// Invokes `callback` with the set of target nodes in the graph for the specified target
    /// pattern, in 'blaze build' syntax.
    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:249 QueryEnvironment.getTargetsMatchingPattern
    fn get_targets_matching_pattern(
        &self,
        owner: &QueryExpression,
        pattern: &str,
        callback: &dyn Callback<T>,
    ) -> QueryTaskFuture<()>;

    /// Ensures the specified target exists.
    // NOTE(bazel-team): this method is left here as scaffolding from a previous refactoring. It
    // may be possible to remove it.
    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:255 QueryEnvironment.getOrCreate
    fn get_or_create(&self, target: &T) -> T;

    /// Returns the direct forward dependencies of the specified targets, in iteration order.
    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:258 QueryEnvironment.getFwdDeps
    fn get_fwd_deps(
        &self,
        targets: &[T],
        context: &QueryExpressionContext<T>,
    ) -> Result<Vec<T>, InterruptedException>;

    /// Returns the direct reverse dependencies of the specified targets, in iteration order.
    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:262 QueryEnvironment.getReverseDeps
    fn get_reverse_deps(
        &self,
        targets: &[T],
        context: &QueryExpressionContext<T>,
    ) -> Result<Vec<T>, InterruptedException>;

    /// Returns the forward transitive closure of all of the targets in "targets". Callers must
    /// ensure that `build_transitive_closure` has been called for the relevant subgraph.
    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:269 QueryEnvironment.getTransitiveClosure
    fn get_transitive_closure(
        &self,
        targets: &dyn ThreadSafeMutableSet<T>,
        context: &QueryExpressionContext<T>,
    ) -> Result<Arc<dyn ThreadSafeMutableSet<T>>, InterruptedException>;

    /// Construct the dependency graph for a depth-bounded forward transitive closure of all nodes
    /// in "targetNodes". The identity of the calling expression is required to produce error
    /// messages.
    ///
    /// The closure may not be loaded, depending on the value of `max_depth` and the
    /// implementation of this `QueryEnvironment`.
    ///
    /// Passing `None` for `max_depth` means that the full closure should be preloaded.
    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:283 QueryEnvironment.buildTransitiveClosure
    fn build_transitive_closure(
        &self,
        caller: &QueryExpression,
        target_nodes: &dyn ThreadSafeMutableSet<T>,
        max_depth: Option<i32>,
    ) -> Result<(), QueryError>;

    /// Returns the ordered sequence of nodes on some path from "from" to "to".
    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:292 QueryEnvironment.getNodesOnPath
    fn get_nodes_on_path(
        &self,
        from: &T,
        to: &T,
        context: &QueryExpressionContext<T>,
    ) -> Result<Vec<T>, InterruptedException>;

    /// Returns a `QueryTaskFuture` representing the asynchronous evaluation of the given `expr`
    /// and passing of the results to the given `callback`.
    ///
    /// Note that this method should guarantee that the callback does not see repeated elements.
    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:304 QueryEnvironment.eval
    fn eval(
        &self,
        expr: &QueryExpression,
        context: &QueryExpressionContext<T>,
        callback: &dyn Callback<T>,
    ) -> QueryTaskFuture<()>;

    /// Creates an `EvaluateExpression` instance based on `QueryEnvironment` type.
    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:359 QueryEnvironment.createEvaluateExpression
    fn create_evaluate_expression<'a>(
        &'a self,
        expr: &QueryExpression,
        context: &QueryExpressionContext<T>,
    ) -> Box<dyn EvaluateExpression<T> + 'a>;

    /// Returns a fresh `ThreadSafeMutableSet` instance for the type `T`.
    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:536 QueryEnvironment.createThreadSafeMutableSet
    fn create_thread_safe_mutable_set(&self) -> Arc<dyn ThreadSafeMutableSet<T>>;

    /// Creates a Uniquifier for use in a `QueryExpression`. Note that the usage of this
    /// uniquifier should not be used for returning unique results to the parent callback. It
    /// should only be used to avoid processing the same elements multiple times within this
    /// QueryExpression.
    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:543 QueryEnvironment.createUniquifier
    fn create_uniquifier(&self) -> Box<dyn Uniquifier<T>>;

    /// Creates a `MinDepthUniquifier` for use in a `QueryExpression`. Note that the usage of this
    /// uniquifier should not be used for returning unique results to the parent callback. It
    /// should only be used to try to avoid processing the same elements multiple times at the
    /// same depth bound within this QueryExpression.
    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:551 QueryEnvironment.createMinDepthUniquifier
    fn create_min_depth_uniquifier(&self) -> Box<dyn MinDepthUniquifier<T>>;

    /// Handle an error during evaluation of `expression` by either throwing `QueryException` or
    /// emitting an event, depending on whether the evaluation is running in a "keep going" mode.
    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:558 QueryEnvironment.handleError
    fn handle_error(
        &self,
        expression: &QueryExpression,
        message: &str,
        detailed_exit_code: Option<&DetailedExitCode>,
    ) -> Result<(), QueryException>;

    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:582 QueryEnvironment.getTransitiveLoadFilesHelper
    fn get_transitive_load_files_helper(
        &self,
    ) -> Result<Box<dyn TransitiveLoadFilesHelper<T> + '_>, QueryException>;

    /// Feeds to the given `callback` the transitive bzl files loaded (and BUILD files too, if
    /// `also_add_build_files` says to), represented as make-believe targets corresponding to
    /// their load labels, across all unique packages in `targets`, using `seen_packages` and
    /// `seen_bzl_labels` to avoid duplicate work and using `uniquifier` to avoid feeding
    /// duplicate results.
    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:591 QueryEnvironment.transitiveLoadFiles
    fn transitive_load_files(
        &self,
        targets: &[T],
        also_add_build_files: bool,
        seen_packages: &Mutex<HashSet<PackageIdentifier>>,
        seen_bzl_labels: &Mutex<HashSet<Label>>,
        uniquifier: &dyn Uniquifier<T>,
        helper: &dyn TransitiveLoadFilesHelper<T>,
        callback: &dyn Callback<T>,
    ) -> Result<(), QueryError>;

    /// Returns an object that can be used to query information about targets. Implementations
    /// should create a single instance and return that for all calls.
    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:607 QueryEnvironment.getAccessor
    fn get_accessor(&self) -> &dyn TargetAccessor<T>;

    /// Whether the given setting is enabled. The code should default to return `false` for all
    /// unknown settings.
    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:618 QueryEnvironment.isSettingEnabled
    fn is_setting_enabled(&self, setting: Setting) -> bool;

    /// Returns the set of query functions implemented by this query environment.
    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:621 QueryEnvironment.getFunctions
    fn get_functions(&self) -> Vec<Arc<dyn QueryFunction<T>>>;

    /// Java `env instanceof StreamableQueryEnvironment`.
    fn as_streamable(&self) -> Option<&dyn StreamableQueryEnvironment<T>> {
        None
    }

    /// Java `env instanceof <a concrete environment class>` (e.g. `RBuildFilesFunction`'s
    /// `env instanceof SkyQueryEnvironment`): the environment as `Any`, for the environments
    /// that functions downcast to.
    fn as_any(&self) -> Option<&dyn std::any::Any> {
        None
    }

    /// Java `env instanceof CustomFunctionQueryEnvironment`.
    fn as_custom_function_query_environment(
        &self,
    ) -> Option<&dyn CustomFunctionQueryEnvironment<T>> {
        None
    }
}

// Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:287 QueryEnvironment.shouldVisit
pub fn should_visit(max_depth: Option<i32>, current_depth: i32) -> bool {
    match max_depth {
        None => true,
        Some(max_depth) => max_depth >= current_depth,
    }
}

/// A wrapper for evaluating query expression. User does not need to provide callback at object
/// instantiation, and could manipulate the future in the callback implementation in some derived
/// classes.
///
/// For SkyQueryEnvironment-descended environments which feature streaming support, in
/// `SomeFunction#eval(...)` method, we want to cancel the `QueryTaskFuture` immediately after
/// targeted number of results are reached. So client will call `gracefully_cancel()` to cancel
/// the underlying `QueryTaskFuture`.
// Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:325 QueryEnvironment.EvaluateExpression
pub trait EvaluateExpression<T> {
    /// Returns a `QueryTaskFuture` representing the asynchronous evaluation of the expression
    /// provided by the constructor of the inherited classes. Results of the asynchronous
    /// evaluation of the expression are passed to the given `callback`.
    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:335 EvaluateExpression.eval
    fn eval(&self, callback: &dyn Callback<T>) -> QueryTaskFuture<()>;

    /// Attempts to cancel execution of expression evaluation task.
    ///
    /// Please note that `gracefully_cancel()` is a no-op implementation for
    /// non-SkyQueryEnvironment-descended environments.
    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:343 EvaluateExpression.gracefullyCancel
    fn gracefully_cancel(&self) -> bool;

    /// Returns `true` if the underlying future is cancelled but not via `gracefully_cancel`.
    /// Clients are advised to propagate such cancellations instead of recovering from them,
    /// because they probably indicate that query evaluation was interrupted.
    // Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:350 EvaluateExpression.isUngracefullyCancelled
    fn is_ungracefully_cancelled(&self) -> bool;
}

/// A mutable thread-safe set that uses proper equality semantics for `T`.
/// `QueryExpression`/`QueryFunction` implementations should use `ThreadSafeMutableSet<T>` when
/// they need a set-like data structure for `T`.
///
/// PORT: Java's `Set<T>` methods take `&self` (the implementations are concurrent collections);
/// iteration is a snapshot in the set's iteration order (`to_vec`).
// Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:533 QueryEnvironment.ThreadSafeMutableSet
pub trait ThreadSafeMutableSet<T> {
    /// Java `Set.add`.
    fn add(&self, element: T) -> bool;

    /// Java `Set.contains`.
    fn contains(&self, element: &T) -> bool;

    /// Java `Set.remove`.
    fn remove(&self, element: &T) -> bool;

    /// Java `Set.size`.
    fn size(&self) -> i32;

    /// The elements in Java iteration order (`iterator()`).
    fn to_vec(&self) -> Vec<T>;

    /// Java `AbstractCollection.isEmpty`.
    fn is_empty(&self) -> bool {
        self.size() == 0
    }

    /// Java `AbstractCollection.addAll`.
    fn add_all(&self, elements: &[T])
    where
        T: Clone,
    {
        for e in elements {
            self.add(e.clone());
        }
    }

    /// Java `AbstractCollection.retainAll`: iterates this set and removes each element `other`
    /// does not contain.
    fn retain_all(&self, other: &dyn ThreadSafeMutableSet<T>) -> bool {
        let mut modified = false;
        for e in self.to_vec() {
            if !other.contains(&e) {
                self.remove(&e);
                modified = true;
            }
        }
        modified
    }
}

/// Settings for the query engine. See [`QueryEnvironment::is_setting_enabled`].
// Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:624 QueryEnvironment.Setting
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum Setting {
    /// Whether to evaluate tests() expressions in strict mode. If `is_setting_enabled` returns
    /// true for this setting, then the tests() expression will give an error when expanding tests
    /// suites, if the test suite contains any non-test targets.
    TestsExpressionStrict,

    /// Do not consider implicit deps (any label that was not explicitly specified in the BUILD
    /// file) when traversing dependency edges.
    NoImplicitDeps,

    /// Do not consider non-target dependencies when traversing dependency edges.
    OnlyTargetDeps,

    /// Do not consider nodep attributes when traversing dependency edges.
    NoNodepDeps,

    /// Include aspect-generated output. No-op for query, which always follows aspects.
    IncludeAspects,

    /// Include configured aspect targets in cquery output.
    ExplicitAspects,
}

/// An adapter interface giving access to properties of T. There are four types of targets:
/// rules, package groups, source files, and generated files. Of these, only rules can have
/// attributes.
// Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:656 QueryEnvironment.TargetAccessor
pub trait TargetAccessor<T> {
    /// Returns the target type represented as a string of the form `<type> rule` or
    /// `package group` or `source file` or `generated file`. This is widely used for target
    /// filtering, so implementations must use the Blaze rule class naming scheme.
    fn get_target_kind(&self, target: &T) -> String;

    /// Returns the full label of the target as a string, e.g. `//some:target`.
    fn get_label(&self, target: &T) -> String;

    /// Returns the label of the target's package as a string, e.g. `//some/package`.
    fn get_package(&self, target: &T) -> String;

    /// Returns whether the given target is a rule.
    fn is_rule(&self, target: &T) -> bool;

    /// Returns whether the given rule is executable with 'bazel run'.
    fn is_executable_non_test_rule(&self, target: &T) -> bool;

    /// Returns whether the given target is a test target. If this returns true, then `is_rule`
    /// must also return true for the target.
    fn is_test_rule(&self, target: &T) -> bool;

    /// Returns whether the given target is a test suite target. If this returns true, then
    /// `is_rule` must also return true for the target, but `is_test_rule` must return false; test
    /// suites are not test rules, and vice versa.
    fn is_test_suite(&self, target: &T) -> bool;

    /// If the attribute of the given name on the given target is a label or label list, then this
    /// method returns the list of corresponding target instances. Otherwise returns an empty list.
    /// If an error occurs during resolution, it throws a `QueryException` using the caller and
    /// error message prefix.
    ///
    /// Panics (Java `IllegalArgumentException`) if target is not a rule (according to `is_rule`).
    fn get_prerequisites(
        &self,
        caller: &QueryExpression,
        target: &T,
        attr_name: &str,
        error_msg_prefix: &str,
    ) -> Result<Vec<T>, QueryError>;

    /// If the attribute of the given name on the given target is a string list, then this method
    /// returns it.
    fn get_string_list_attr(&self, target: &T, attr_name: &str) -> Vec<String>;

    /// If the attribute of the given name on the given target is a string, then this method
    /// returns it (Java may return null).
    fn get_string_attr(&self, target: &T, attr_name: &str) -> Option<String>;

    /// Returns the given attribute represented as a list of strings. For "normal" attributes,
    /// this should just be a list of size one containing the attribute's value. For configurable
    /// attributes, there should be one entry for each possible value the attribute may take.
    ///
    /// Note that for backwards compatibility, tristate and boolean attributes are returned as int
    /// using the values `0, 1` and `-1`. If there is no such attribute, this method returns an
    /// empty list.
    fn get_attr_as_string(&self, target: &T, attr_name: &str) -> Vec<String>;

    /// Returns the set of package specifications the given target is visible from, represented as
    /// `QueryVisibility`s (Java `ImmutableSet`, in its insertion order).
    fn get_visibility<'a>(
        &'a self,
        caller: &QueryExpression,
        from: &T,
    ) -> Result<Vec<Arc<dyn QueryVisibility<T> + 'a>>, QueryError>;
}

/// Helper for `transitive_load_files`. Encapsulates the differences between the different
/// `QueryEnvironment` implementations.
// Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:566 QueryEnvironment.TransitiveLoadFilesHelper
pub trait TransitiveLoadFilesHelper<T> {
    fn get_pkg_id(&self, target: &T) -> PackageIdentifier;

    /// `visitor` is Java's `BazelModuleContext.LoadGraphVisitor<QueryException,
    /// InterruptedException>`: it returns true if the visitation should recurse into the loads of
    /// the given file.
    fn visit_loads(
        &self,
        original_target: &T,
        visitor: &mut dyn FnMut(&Label) -> Result<bool, QueryError>,
    ) -> Result<(), QueryError>;

    fn get_build_file_target(&self, original_target: &T) -> Result<T, InterruptedException>;

    fn get_load_file_target(
        &self,
        original_target: &T,
        bzl_label: &Label,
    ) -> Result<T, InterruptedException>;

    fn maybe_get_build_file_target_for_load_file_target(
        &self,
        original_target: &T,
        bzl_label: &Label,
    ) -> Result<Option<T>, QueryError>;
}

/// List of the default query functions.
// Java: com/google/devtools/build/lib/query2/engine/QueryEnvironment.java:741 QueryEnvironment.DEFAULT_QUERY_FUNCTIONS
// PORT: one `add` per function, as Java's ImmutableList.Builder.
#[allow(clippy::vec_init_then_push)]
pub fn default_query_functions<T: Clone + 'static>() -> Vec<Arc<dyn QueryFunction<T>>> {
    let mut functions: Vec<Arc<dyn QueryFunction<T>>> = Vec::new();
    functions.push(Arc::new(AllPathsFunction::new()));
    functions.push(Arc::new(AttrFunction::new()));
    functions.push(Arc::new(BuildFilesFunction::new()));
    functions.push(Arc::new(DepsFunction::new()));
    functions.push(Arc::new(ExecutablesFunction::new()));
    functions.push(Arc::new(FilterFunction::new()));
    functions.push(Arc::new(KindFunction::new()));
    functions.push(Arc::new(LabelsFunction::new()));
    functions.push(Arc::new(LoadFilesFunction::new()));
    functions.push(Arc::new(RdepsFunction::new()));
    functions.push(Arc::new(SamePkgDirectRdepsFunction::new()));
    functions.push(Arc::new(SiblingsFunction::new()));
    functions.push(Arc::new(SomeFunction::new()));
    functions.push(Arc::new(SomePathFunction::new()));
    functions.push(Arc::new(TestsFunction::new()));
    functions.push(Arc::new(VisibleFunction::new()));
    functions
}
