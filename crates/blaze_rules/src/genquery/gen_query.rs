//! Port of `rules/genquery/GenQuery.java`: the `genquery` rule's configured target factory, its
//! `QueryResultAction` and the Skyframe-backed target pattern preloader.

use std::any::Any;
use std::cell::{Cell, RefCell};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::io::{self, Write};
use std::rc::Rc;
use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_actions::actions::action::{
    AbstractAction, Action, ComputeKeyError, InputMetadataProvider,
};
use blaze_actions::actions::action_key_context::ActionKeyContext;
use blaze_actions::actions::action_owner::ActionOwner;
use blaze_actions::analysis::actions::abstract_file_write_action::{
    self, AbstractFileWriteAction, DeterministicWriter, NewDeterministicWriterError,
    new_abstract_file_write_action,
};
use blaze_actions::analysis::runfiles::RunfilesBuilder;
use blaze_analysis::configured_rule_class_provider::{
    CreateConfiguredTargetError, RuleConfiguredTargetFactory,
};
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::providers::file_provider::RunfilesProvider;
use blaze_analysis::providers::output_group_info::VALIDATION_TRANSITIVE;
use blaze_analysis::rule::rule_configured_target_builder::RuleConfiguredTargetBuilder;
use blaze_analysis::rule::rule_context::RuleContext;
use blaze_analysis::rule::rule_error_consumer::RuleErrorConsumer;
use blaze_cmdline::{
    Label, PackageIdentifier, SignedTargetPattern, TargetParsingException, TargetPatternParser,
    TargetPatternsCode,
};
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_events::{Event, EventHandler, ExtendedEventHandler};
use blaze_optdefs::command::keep_going_option::KeepGoingOption;
use blaze_optdefs::command::query_options::{OrderOutput, QueryOptions};
use blaze_options::{OptionsClass, OptionsParser, TriState};
use blaze_packages::attribute::AttributeMap;
use blaze_packages::exceptions::NoSuchPackageException;
use blaze_packages::target::Target;
use blaze_packages::types::{AttrValue, Type};
use blaze_query::common::abstract_blaze_query_environment::{
    AbstractBlazeQueryEnvironment, EvaluateQueryError, LabelFilter,
};
use blaze_query::common::query_util_targets::new_lexicographically_sorted_target_aggregator;
use blaze_query::pkgcache::{PreloadTargetPatternsError, TargetPatternPreloader};
use blaze_query::query::aspectresolvers::aspect_resolver;
use blaze_query::query::blaze_query_environment::BlazeQueryEnvironment;
use blaze_query::query::graphless_blaze_query_environment::GraphlessBlazeQueryEnvironment;
use blaze_query::query::output::output_formatter::OutputStream;
use blaze_query::query::output::query_output_utils;
use blaze_query::query::output::{output_formatters, query_options as query_options_util};
use blaze_query_engine::query_util::AggregateAllCallback;
use blaze_query_engine::{
    DigraphQueryEvalResult, InterruptedException, QueryEvalResult, QueryExpression,
    ThreadSafeOutputFormatterCallback,
};
use blaze_sky_loading::package_function::PackageKey;
use blaze_sky_loading::target_pattern_function::{FilteringPolicy, TargetPatternKey};
use blaze_util::Fingerprint;
use blaze_vfs::{DigestHashFunction, PathFragment};
use indexmap::{IndexMap, IndexSet};

use super::gen_query_output_stream::{GenQueryOutputStream, GenQueryResult};
use super::gen_query_package_provider::{GenQueryPackageProvider, construct_package_map};

/// An implementation of the 'genquery' rule.
// Java: com/google/devtools/build/lib/rules/genquery/GenQuery.java:91 GenQuery
pub struct GenQuery;

impl RuleConfiguredTargetFactory for GenQuery {
    // Java: com/google/devtools/build/lib/rules/genquery/GenQuery.java:96 GenQuery.create
    fn create(
        &self,
        rule_context: &mut RuleContext,
    ) -> Result<Option<ConfiguredTarget>, CreateConfiguredTargetError> {
        let rule_context: &RuleContext = rule_context;
        let output_artifact = rule_context.create_output_artifact();

        // The query string
        let query = string_attr(rule_context, "expression");

        // Label.RepoContext.of(ruleContext.getRepository(), repositoryMapping)
        // PORT: Blaze's label converter takes the repository mapping as its conversion context
        // (blaze_optdefs::fragment::converters); neither options class has a label-typed option.
        let mut options_parser = OptionsParser::builder()
            .options_classes(&[QueryOptions::class(), KeepGoingOption::class()])
            .allow_residue(false)
            .with_conversion_context(Arc::new(
                rule_context
                    .get_rule()
                    .get_package_metadata()
                    .repository_mapping
                    .clone(),
            ))
            .build()
            .unwrap_or_else(|e| panic!("{}", e.get_message()));
        let opts: Vec<String> = string_list_attr(rule_context, "opts");
        let opts_refs: Vec<&str> = opts.iter().map(|s| s.as_str()).collect();
        if let Err(e) = options_parser.parse(&opts_refs) {
            rule_context.attribute_error(
                "opts",
                &format!("error while parsing query options: {}", e.get_message()),
            );
            return Ok(None);
        }

        // Parsed query options
        let mut query_options: QueryOptions = options_parser
            .get_options::<QueryOptions>()
            .expect("QueryOptions");
        // If you change the list of options here, also change the documentation of genquery.opts
        // in GenQueryRule.java .
        if options_parser
            .get_options::<KeepGoingOption>()
            .expect("KeepGoingOption")
            .keep_going
        {
            rule_context.attribute_error("opts", "option --keep_going is not allowed");
            return Ok(None);
        }
        if !query_options.universe_scope.is_empty() {
            rule_context.attribute_error("opts", "option --universe_scope is not allowed");
            return Ok(None);
        }
        if options_parser.contains_explicit_option("order_results") {
            rule_context.attribute_error("opts", "option --order_results is not allowed");
            return Ok(None);
        }
        if options_parser.contains_explicit_option("noorder_results") {
            rule_context.attribute_error("opts", "option --noorder_results is not allowed");
            return Ok(None);
        }
        if options_parser.contains_explicit_option("order_output") {
            rule_context.attribute_error("opts", "option --order_output is not allowed");
            return Ok(None);
        }
        if options_parser.contains_explicit_option("experimental_graphless_query") {
            rule_context.attribute_error(
                "opts",
                "option --experimental_graphless_query is not allowed",
            );
            return Ok(None);
        }
        // Genquery should always use AUTO, while build isn't affected by query options, .
        query_options.use_graphless_query = TriState::AUTO;

        // force relative_locations to true so it has a deterministic output across machines.
        query_options.relative_locations = true;

        if !options_parser.contains_explicit_option("nodep_deps") {
            // Have GenQuery *not* include "nodep" deps by default. This is an unfortunate
            // divergence from `query` which is necessary to maintain legacy behavior.
            // TODO(b/123122592): Complete the migration and remove this divergence.
            query_options.include_no_dep_deps = false;
        }

        // PORT: skipped, profiling ("GenQuery.executeQuery").
        let scope: Vec<Label> = match rule_context
            .attributes()
            .get("scope", Type::GenqueryScopeTypeList)
        {
            Some(AttrValue::LabelList(scope)) => scope,
            None => Vec::new(),
            Some(other) => panic!("genquery scope: unexpected value {other:?}"),
        };
        let result = execute_query(
            rule_context,
            &mut query_options,
            &scope,
            &query,
            output_artifact
                .get_path()
                .get_file_system()
                .get_digest_function(),
        )?;
        let Some(result) = result else {
            return Ok(None);
        };
        if rule_context.has_errors() {
            return Ok(None);
        }

        // PORT: skipped, logging ("Genquery %s had large output %s").
        rule_context.register_action(Arc::new(QueryResultAction::new(
            rule_context.get_action_owner(),
            output_artifact.clone(),
            result,
        )));

        let files_to_build: NestedSet<Artifact> =
            NestedSetBuilder::create(Order::STABLE_ORDER, [output_artifact]);
        let mut runfiles = RunfilesBuilder::new(rule_context.get_workspace_name());
        runfiles.add_transitive_artifacts(&files_to_build);
        let mut builder = RuleConfiguredTargetBuilder::new(rule_context);
        builder
            .set_files_to_build(files_to_build)
            .add_provider(RunfilesProvider::simple(runfiles.build()))
            .add_output_group(
                VALIDATION_TRANSITIVE,
                &NestedSetBuilder::empty_set(Order::STABLE_ORDER),
            );
        Ok(builder.build()?)
    }
}

fn string_attr(rule_context: &RuleContext, name: &str) -> String {
    match rule_context.attributes().get(name, Type::String) {
        Some(AttrValue::String(s)) => s,
        other => panic!("genquery {name}: unexpected value {other:?}"),
    }
}

fn string_list_attr(rule_context: &RuleContext, name: &str) -> Vec<String> {
    match rule_context.attributes().get(name, Type::StringList) {
        Some(AttrValue::StringList(l)) => l,
        None => Vec::new(),
        other => panic!("genquery {name}: unexpected value {other:?}"),
    }
}

fn bool_attr(rule_context: &RuleContext, name: &str) -> bool {
    match rule_context.attributes().get(name, Type::Boolean) {
        Some(AttrValue::Bool(b)) => b,
        other => panic!("genquery {name}: unexpected value {other:?}"),
    }
}

/// DO NOT USE! We should get rid of this method: errors reported directly to this object don't
/// set the error flag in `ConfiguredTarget`.
// Java: com/google/devtools/build/lib/rules/genquery/GenQuery.java:210 GenQuery.getEventHandler
fn get_event_handler(rule_context: &RuleContext) -> Arc<dyn ExtendedEventHandler> {
    rule_context
        .get_analysis_environment()
        .get_event_handler()
        .clone()
}

/// `Ok(None)` where Java returns null.
// Java: com/google/devtools/build/lib/rules/genquery/GenQuery.java:214 GenQuery.executeQuery
fn execute_query(
    rule_context: &RuleContext,
    query_options: &mut QueryOptions,
    scope: &[Label],
    query: &str,
    hash_function: DigestHashFunction,
) -> Result<Option<GenQueryResult>, CreateConfiguredTargetError> {
    let package_provider = match blaze_analysis::skyframe_env::get_skyframe_env(|env| {
        construct_package_map(env, scope)
    }) {
        Err(blaze_skyframe::Interrupted) => {
            return Err(CreateConfiguredTargetError::Interrupted);
        }
        Ok(Ok(None)) => return Ok(None),
        Ok(Ok(Some(provider))) => provider,
        Ok(Err(e)) => {
            rule_context.rule_error(e.get_message());
            return Ok(None);
        }
    };

    do_query(
        query_options,
        package_provider,
        Arc::new(SkyframeEnvTargetPatternEvaluator::default()),
        query,
        rule_context,
        hash_function,
    )
}

/// The callback the query evaluates into (Java `AggregateAllOutputFormatterCallback<Target, ?>`).
enum TargetsCallback {
    Lexicographic(blaze_query::common::query_util_targets::LexicographicallySortedTargetAggregator),
    Ordered(
        Box<
            dyn blaze_query_engine::query_util::AggregateAllOutputFormatterCallback<
                    Target,
                    Vec<Target>,
                >,
        >,
    ),
}

impl TargetsCallback {
    fn as_callback(&self) -> &dyn ThreadSafeOutputFormatterCallback<Target> {
        match self {
            TargetsCallback::Lexicographic(c) => c,
            TargetsCallback::Ordered(c) => &**c,
        }
    }

    fn get_result(&self) -> Vec<Target> {
        match self {
            TargetsCallback::Lexicographic(c) => c.get_result(),
            TargetsCallback::Ordered(c) => c.get_result(),
        }
    }
}

/// An `EventHandler` view of the analysis environment's handler.
struct ForwardingEventHandler(Arc<dyn ExtendedEventHandler>);

impl EventHandler for ForwardingEventHandler {
    fn handle(&self, event: &Event) {
        self.0.handle(event);
    }
}

// Java: com/google/devtools/build/lib/rules/genquery/GenQuery.java:244 GenQuery.doQuery
fn do_query(
    query_options: &mut QueryOptions,
    package_provider: Arc<GenQueryPackageProvider>,
    preloader: Arc<SkyframeEnvTargetPatternEvaluator>,
    query: &str,
    rule_context: &RuleContext,
    hash_function: DigestHashFunction,
) -> Result<Option<GenQueryResult>, CreateConfiguredTargetError> {
    let settings = query_options_util::to_settings(query_options);

    let formatters = output_formatters::get_default_formatters();
    let Some(formatter) =
        output_formatters::get_formatter(&formatters, &query_options.output_format)
    else {
        rule_context.rule_error(&format!(
            "Invalid output format '{}'. Valid values are: {}",
            query_options.output_format,
            output_formatters::formatter_names(&formatters)
        ));
        return Ok(None);
    };
    let graphless_query = output_formatters::is_streamed_formatter(formatter);
    if graphless_query {
        query_options.order_output = OrderOutput::NO;
    } else {
        // Force results to be deterministic.
        query_options.order_output = OrderOutput::FULL;
    }

    // QUERY_ENVIRONMENT_FACTORY.create(...)
    // Java: com/google/devtools/build/lib/query2/QueryEnvironmentFactory.java:41 QueryEnvironmentFactory.create
    let rule = rule_context.get_rule();
    let target_parser = TargetPatternParser::new(
        PathFragment::empty_fragment(),
        rule_context.get_repository().clone(),
        rule.get_package_metadata().repository_mapping.clone(),
    );
    let label_filter = LabelFilter::Predicate(package_provider.get_valid_target_predicate());
    let event_handler = get_event_handler(rule_context);
    let strict = bool_attr(rule_context, "strict");
    let label_printer = query_options_util::get_label_printer_legacy(
        query_options,
        rule_context.get_starlark_semantics(),
    );
    // canUseSkyQuery(orderedResults = !graphlessQuery, UniverseScope.EMPTY, ...) is false.
    let query_environment: Box<dyn AbstractBlazeQueryEnvironment> = if graphless_query {
        Box::new(GraphlessBlazeQueryEnvironment::new(
            /* query_transitive_package_preloader= */ None,
            package_provider.clone(),
            package_provider.clone(),
            preloader.clone(),
            target_parser,
            /* keep_going= */ false,
            strict,
            // Use a single thread to prevent race conditions causing nondeterministic output
            // (b/127644784). All the packages are already loaded at this point, so there is no
            // need to start up multiple threads anyway.
            /* loading_phase_threads= */
            1,
            label_filter,
            event_handler.clone(),
            settings,
            /* extra_functions= */ Vec::new(),
            label_printer,
        ))
    } else {
        Box::new(BlazeQueryEnvironment::new(
            /* query_transitive_package_preloader= */ None,
            package_provider.clone(),
            package_provider.clone(),
            preloader.clone(),
            target_parser,
            /* keep_going= */ false,
            strict,
            /* loading_phase_threads= */ 1,
            label_filter,
            event_handler.clone(),
            settings,
            /* extra_functions= */ Vec::new(),
            label_printer,
        ))
    };

    let expr = match QueryExpression::parse(query, query_environment.as_query_environment()) {
        Ok(expr) => expr,
        Err(e) => {
            rule_context.rule_error(&format!("query syntax error: {}", e.get_message()));
            return Ok(None);
        }
    };
    if let Err(e) = formatter.verify_compatible(&*query_environment, &expr) {
        rule_context.rule_error(&format!("query failed: {}", e.get_message()));
        return Ok(None);
    }
    let targets = if graphless_query && !expr.is_top_level_some_path_function() {
        TargetsCallback::Lexicographic(new_lexicographically_sorted_target_aggregator())
    } else {
        TargetsCallback::Ordered(
            blaze_query_engine::query_util::new_ordered_aggregate_all_output_formatter_callback(
                query_environment.as_query_environment(),
            ),
        )
    };
    let query_result: QueryEvalResult =
        match query_environment.evaluate_query(&expr, targets.as_callback()) {
            Ok(result) => result,
            Err(_) if preloader.restart.get() => {
                // Do not emit errors for skyframe restarts. They make output of the
                // ConfiguredTargetFunction inconsistent from run to run, and make detecting
                // legitimate errors more difficult.
                return Ok(None);
            }
            Err(EvaluateQueryError::Query(e)) => {
                rule_context.rule_error(&format!("query failed: {}", e.get_message()));
                return Ok(None);
            }
            Err(EvaluateQueryError::Interrupted(_)) => {
                return Err(CreateConfiguredTargetError::Interrupted);
            }
            // catch (IOException e) { throw new RuntimeException(e); }
            Err(EvaluateQueryError::Io(e)) => panic!("RuntimeException: {e}"),
        };

    let compressed_output_requested = bool_attr(rule_context, "compressed_output");
    let output_stream = Rc::new(RefCell::new(GenQueryOutputStream::new(
        compressed_output_requested,
    )));
    let result: Vec<Target> = targets.get_result();
    let digraph_result = query_environment
        .get_result_graph()
        .map(|graph| DigraphQueryEvalResult::new(query_result.clone(), graph));
    let output: OutputStream = output_stream.clone();
    let output_result = query_output_utils::output(
        query_options,
        digraph_result.as_ref(),
        &result,
        formatter,
        output,
        aspect_resolver::create_resolver(
            query_options.aspect_deps,
            package_provider.clone(),
            event_handler.clone(),
        ),
        Some(Arc::new(ForwardingEventHandler(event_handler.clone()))),
        hash_function,
        query_environment.get_label_printer(),
    );
    match output_result {
        Ok(()) => {}
        // ClosedByInterruptException: throw new InterruptedException(e.getMessage())
        Err(blaze_query_engine::OutputFormatterError::Interrupted(_)) => {
            return Err(CreateConfiguredTargetError::Interrupted);
        }
        // catch (IOException e) { throw new RuntimeException(e); }
        Err(blaze_query_engine::OutputFormatterError::Io(e)) => panic!("RuntimeException: {e}"),
    }
    let mut stream = output_stream.borrow_mut();
    stream
        .close()
        .unwrap_or_else(|e| panic!("RuntimeException: {e}"));
    Ok(Some(stream.get_result()))
}

// Java: com/google/devtools/build/lib/rules/genquery/GenQuery.java:357 GenQuery.QueryResultAction
pub struct QueryResultAction {
    base: AbstractAction,
    result: GenQueryResult,
}

impl QueryResultAction {
    // Java: com/google/devtools/build/lib/rules/genquery/GenQuery.java:361 GenQuery.QueryResultAction.<init>
    fn new(owner: ActionOwner, output: Artifact, result: GenQueryResult) -> QueryResultAction {
        QueryResultAction {
            base: new_abstract_file_write_action(
                owner,
                NestedSetBuilder::empty_set(Order::STABLE_ORDER),
                output,
            ),
            result,
        }
    }
}

impl fmt::Debug for QueryResultAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("QueryResultAction")
            .field("base", &self.base)
            .field("size", &self.result.size())
            .finish_non_exhaustive()
    }
}

impl Action for QueryResultAction {
    fn abstract_action(&self) -> &AbstractAction {
        &self.base
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    // Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:96 AbstractFileWriteAction.getMnemonic
    fn get_mnemonic(&self) -> String {
        abstract_file_write_action::MNEMONIC.to_string()
    }

    // Java: com/google/devtools/build/lib/rules/genquery/GenQuery.java:371 GenQuery.QueryResultAction.computeKey
    fn compute_key(
        &self,
        _action_key_context: &ActionKeyContext,
        _input_metadata_provider: Option<&dyn InputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        self.result.fingerprint(fp);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:101 AbstractFileWriteAction.getRawProgressMessage
    fn get_raw_progress_message(&self) -> Option<String> {
        abstract_file_write_action::raw_progress_message(self)
    }

    // Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:125 AbstractFileWriteAction.getExecProperties
    fn get_exec_properties(&self) -> IndexMap<String, String> {
        abstract_file_write_action::exec_properties()
    }
}

impl AbstractFileWriteAction for QueryResultAction {
    // Java: com/google/devtools/build/lib/rules/genquery/GenQuery.java:366 GenQuery.QueryResultAction.newDeterministicWriter
    fn new_deterministic_writer(
        &self,
    ) -> Result<Box<dyn DeterministicWriter + '_>, NewDeterministicWriterError> {
        // GenQueryResultWriter
        Ok(Box::new(GenQueryResultWriter(&self.result)))
    }
}

// Java: com/google/devtools/build/lib/rules/genquery/GenQuery.java:503 GenQuery.GenQueryResultWriter
struct GenQueryResultWriter<'a>(&'a GenQueryResult);

impl DeterministicWriter for GenQueryResultWriter<'_> {
    // Java: com/google/devtools/build/lib/rules/genquery/GenQuery.java:511 GenQuery.GenQueryResultWriter.writeTo
    fn write_to(&self, out: &mut dyn Write) -> io::Result<()> {
        self.0.write_to(out)
    }

    // Java: com/google/devtools/build/lib/rules/genquery/GenQuery.java:516 GenQuery.GenQueryResultWriter.getBytes
    fn get_bytes(&self) -> io::Result<Vec<u8>> {
        self.0.get_bytes()
    }
}

/// Provide target pattern evaluation to the query operations using Skyframe dep lookup.
///
/// PORT: Java's `SkyframeRestartQueryException` (a `RuntimeException` that unwinds the query
/// evaluation) is the `restart` flag plus an `InterruptedException` from the preloader; `doQuery`
/// checks the flag first, as Java's first `catch`.
// Java: com/google/devtools/build/lib/rules/genquery/GenQuery.java:383 GenQuery.SkyframeEnvTargetPatternEvaluator
#[derive(Default)]
struct SkyframeEnvTargetPatternEvaluator {
    restart: Cell<bool>,
}

impl SkyframeEnvTargetPatternEvaluator {
    fn skyframe_restart(&self) -> PreloadTargetPatternsError {
        self.restart.set(true);
        PreloadTargetPatternsError::Interrupted(InterruptedException::new())
    }

    // Java: com/google/devtools/build/lib/rules/genquery/GenQuery.java:390 GenQuery.SkyframeEnvTargetPatternEvaluator.getExistingTarget
    fn get_existing_target(
        label: &Label,
        packages: &HashMap<PackageIdentifier, Arc<blaze_packages::package::Package>>,
    ) -> Target {
        match packages[label.get_package_identifier()].get_target(label.get_name()) {
            Ok(t) => t.clone(),
            // Unexpected since the label was part of the TargetPatternValue.
            Err(e) => panic!("IllegalStateException: {}", e.get_message()),
        }
    }

    // Java: com/google/devtools/build/lib/rules/genquery/GenQuery.java:488 GenQuery.SkyframeEnvTargetPatternEvaluator.checkValidPatternType
    fn check_valid_pattern_type(
        pattern: &str,
        parser: &TargetPatternParser,
    ) -> Result<(), TargetParsingException> {
        use blaze_cmdline::target_pattern::TargetPatternType;
        let type_ = parser.parse(pattern)?.get_type();
        if type_ == TargetPatternType::PathAsTarget {
            return Err(TargetParsingException::new(
                format!("couldn't determine target from filename '{pattern}'"),
                TargetPatternsCode::CannotDetermineTargetFromFilename,
            ));
        } else if type_ == TargetPatternType::TargetsBelowDirectory {
            return Err(TargetParsingException::new(
                format!("recursive target patterns are not permitted: '{pattern}'"),
                TargetPatternsCode::RecursiveTargetPatternsNotAllowed,
            ));
        }
        Ok(())
    }
}

impl TargetPatternPreloader for SkyframeEnvTargetPatternEvaluator {
    // Java: com/google/devtools/build/lib/rules/genquery/GenQuery.java:400 GenQuery.SkyframeEnvTargetPatternEvaluator.preloadTargetPatterns
    fn preload_target_patterns(
        &self,
        _event_handler: &Arc<dyn ExtendedEventHandler>,
        target_parser: &TargetPatternParser,
        patterns: &[String],
        keep_going: bool,
    ) -> Result<IndexMap<String, Vec<Target>>, PreloadTargetPatternsError> {
        blaze_util::check_argument!(!keep_going);
        blaze_util::check_argument!(target_parser.get_relative_directory().is_empty());
        let mut ok = true;
        // Maps.newHashMapWithExpectedSize: PORT: the returned map keeps `patterns`' order
        // (BlazeQueryEnvironment reads it by pattern).
        let mut preloaded_patterns: IndexMap<String, Vec<Target>> = IndexMap::new();
        // ImmutableMap.Builder<TargetPatternKey, String>: buildOrThrow fails on a repeated key.
        let mut pattern_keys: IndexMap<TargetPatternKey, String> = IndexMap::new();
        for pattern in patterns {
            Self::check_valid_pattern_type(pattern, target_parser)?;
            let key = TargetPatternKey::key(
                SignedTargetPattern::parse(pattern, target_parser)?,
                FilteringPolicy::NoFilter,
            );
            if pattern_keys.insert(key, pattern.clone()).is_some() {
                panic!("IllegalArgumentException: Multiple entries with same key: {pattern}");
            }
        }
        // HashSet<SkyKey>: iterated only to look the packages up.
        let mut package_keys: IndexSet<PackageIdentifier> = IndexSet::new();
        let mut resolved_labels_map: HashMap<String, Vec<Label>> = HashMap::new();
        let looked_up = blaze_analysis::skyframe_env::get_skyframe_env(
            |env| -> Result<Result<(), TargetParsingException>, blaze_skyframe::Interrupted> {
                for (key, pattern) in &pattern_keys {
                    match env.get_value_or_throw::<TargetPatternKey, TargetParsingException>(
                        key.clone(),
                    )? {
                        Err(e) => return Ok(Err(e)),
                        Ok(None) => ok = false,
                        Ok(Some(pattern_value)) => {
                            let resolved_labels: Vec<Label> =
                                pattern_value.get_targets().iter().cloned().collect();
                            // Iterables.concat(resolvedLabels.getTargets(),
                            // resolvedLabels.getFilteredTargets()): the filtered set of a
                            // TargetPatternValue is always empty (TargetPatternFunction.compute
                            // only adds targets, and removes the filtered targets of a set that
                            // is empty), so Blaze's value keeps the targets only.
                            // Java: com/google/devtools/build/lib/skyframe/TargetPatternFunction.java:132 TargetPatternFunction.compute
                            for label in &resolved_labels {
                                package_keys.insert(label.get_package_identifier().clone());
                            }
                            resolved_labels_map.insert(pattern.clone(), resolved_labels);
                        }
                    }
                }
                Ok(Ok(()))
            },
        );
        match looked_up {
            Err(blaze_skyframe::Interrupted) => {
                return Err(PreloadTargetPatternsError::Interrupted(
                    InterruptedException::new(),
                ));
            }
            Ok(Err(e)) => return Err(PreloadTargetPatternsError::TargetParsing(e)),
            Ok(Ok(())) => {}
        }
        if !ok {
            return Err(self.skyframe_restart());
        }
        let mut packages: HashMap<PackageIdentifier, Arc<blaze_packages::package::Package>> =
            HashMap::new();
        let looked_up = blaze_analysis::skyframe_env::get_skyframe_env(
            |env| -> Result<(), blaze_skyframe::Interrupted> {
                // packageKeys is not mutated, the iteration order is the same.
                for pkg_name in &package_keys {
                    match env.get_value_or_throw::<PackageKey, NoSuchPackageException>(
                        PackageKey::key(pkg_name.clone()),
                    )? {
                        Ok(None) => {
                            ok = false;
                            continue;
                        }
                        Ok(Some(package_value)) => {
                            packages.insert(pkg_name.clone(), package_value.get_package().clone());
                        }
                        Err(_nspe) => continue,
                    }
                }
                Ok(())
            },
        );
        if looked_up.is_err() {
            return Err(PreloadTargetPatternsError::Interrupted(
                InterruptedException::new(),
            ));
        }
        if !ok {
            return Err(self.skyframe_restart());
        }
        for pattern in patterns {
            let Some(resolved_labels) = resolved_labels_map.get(pattern) else {
                continue;
            };
            // CompactHashSet: PORT: insertion order (the order of the pattern's resolution).
            let mut builder: Vec<Target> = Vec::new();
            let mut seen: HashSet<Label> = HashSet::new();
            for label in resolved_labels {
                if seen.insert(label.clone()) {
                    builder.push(Self::get_existing_target(label, &packages));
                }
            }
            preloaded_patterns.insert(pattern.clone(), builder);
        }
        Ok(preloaded_patterns)
    }
}

/// Registers the factory under its Java class simple name.
pub fn register_factory(
    builder: &mut blaze_analysis::configured_rule_class_provider::ConfiguredRuleClassProviderBuilder,
) {
    builder.add_factory("GenQuery", Arc::new(GenQuery));
}

// Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:60 AbstractFileWriteAction.execute
impl blaze_exec::actions::ExecutableAction for QueryResultAction {
    fn execute(
        &self,
        ctx: &dyn blaze_exec::exec::action_execution_context::ActionExecutionContext,
    ) -> Result<blaze_exec::actions::ActionResult, blaze_exec::actions::ActionExecutionException>
    {
        blaze_exec::analysis::actions::abstract_file_write_action::execute_file_write(
            self,
            ctx,
            || {
                self.new_deterministic_writer().map_err(|e| match e {
                    NewDeterministicWriterError::Interrupted => {
                        blaze_exec::actions::ExecException::Interrupted
                    }
                    NewDeterministicWriterError::Exec(m) => blaze_util::unported!(
                        "AbstractFileWriteAction.newDeterministicWriter",
                        "unexpected ExecException from QueryResultAction: {}",
                        m
                    ),
                })
            },
        )
    }
}
