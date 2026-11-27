//! Port of `query2/cquery/StarlarkOutputFormatterCallback.java` (cquery `--output=starlark`),
//! with `ConfiguredTarget.getProvidersDictForQuery` of each configured target class (the
//! `providers()` function of the cquery dialect).

use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;

use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::configuredtargets::{
    get_default_info, target_from_starlark, target_to_starlark,
};
use blaze_analysis::provider_map::{ProviderInstance, ProviderMapKey, TransitiveInfoProviderMap};
use blaze_analysis::providers::file_provider::{
    FileProvider, FilesToRunProvider, StarlarkFileProvider,
};
use blaze_build_config::build_configuration_key::BuildConfigurationKey;
use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_build_config::function_transition_util::starlark_option_to_java;
use blaze_build_config::starlark_transition::{JavaObject, from_java};
use blaze_cmdline::label_constants::COMMAND_LINE_OPTION_PREFIX;
use blaze_events::{Event, ExtendedEventHandler};
use blaze_packages::rule_class::ProviderKey;
use blaze_packages::starlark_defs::builtin_providers::{
    DEFAULT_INFO, INSTRUMENTED_FILES_INFO, OUTPUT_GROUP_INFO,
};
use blaze_protos::failure_details::{configurable_query, query};
use blaze_query_engine::callback::{
    IoExceptionSlot, OutputFormatterError, output_formatter_callback_process,
};
use blaze_query_engine::{
    Callback, OutputFormatterCallback, QueryError, QueryException,
    ThreadSafeOutputFormatterCallback,
};
use blaze_sky_top::skyframe_executor::SkyframeExecutor;
use blaze_starlark::eval::{EvalOptions, frozen_into, with_evaluator};
use blaze_starlark::file_kind::{FileKind, FileOptions};
use blaze_starlark::loader::LoadMap;
use blaze_starlark::thread::{Phase, StarlarkEvents, ThreadContext};
use blaze_starlark::{EvalError, Location, StarlarkSemantics};
use indexmap::IndexMap;
use starlark::environment::{Globals, GlobalsBuilder, Module};
use starlark::eval::Evaluator;
use starlark::values::dict::AllocDict;
use starlark::values::{Heap, OwnedFrozenValue, Value, ValueLike};
use starlark_derive::starlark_module;

use crate::cquery::configured_target_query_environment::CqueryNode;
use crate::cquery::cquery_threadsafe_callback::CqueryThreadsafeCallback;
use crate::post_analysis_query_environment::NamedThreadSafeOutputFormatterCallback;

/// The configuration lookup the cquery dialect's `build_options` uses (Java
/// `CqueryThreadsafeCallback.getConfiguration`, which the inner class `CqueryDialectGlobals`
/// calls on its outer instance).
type ConfigurationLookup =
    Rc<dyn Fn(&BuildConfigurationKey) -> Option<Arc<BuildConfigurationValue>>>;

/// The thread payload through which the natives of `CqueryDialectGlobals` reach the formatter.
struct CqueryDialectPayload {
    get_configuration: ConfigurationLookup,
}

/// The default print handler of a `StarlarkThread` (`StarlarkThread.createTransient` sets none):
/// `print` writes `<caller location>: <message>` to the server's standard error.
// Java: net/starlark/java/eval/StarlarkThread.java:366 StarlarkThread.defaultPrintHandler
struct StderrPrint;

impl StarlarkEvents for StderrPrint {
    fn debug(&self, location: Option<&Location>, message: &str) {
        // System.err.println(thread.getCallerLocation() + ": " + msg); a builtin caller is
        // Location.BUILTIN ("<builtin>").
        let line = match location {
            Some(location) => format!("{location}: {message}"),
            None => format!("<builtin>: {message}"),
        };
        blaze_events::io::system_err_println(&line);
    }

    fn warn(&self, location: Option<&Location>, message: &str) {
        self.debug(location, message);
    }
}

/// The configured target of a `target` argument (Java: the parameter's declared type
/// `CqueryNode`; the call machinery rejects any other value).
fn cquery_node_param(func: &str, target: Value) -> starlark::Result<ConfiguredTarget> {
    match target_from_starlark(target) {
        Some(ct) => Ok(ct),
        None => Err(blaze_packages::starlark_defs::util::param_type_error(
            func,
            "target",
            target,
            "CqueryNode",
        )),
    }
}

fn payload_lookup(eval: &Evaluator) -> anyhow::Result<ConfigurationLookup> {
    let cx = ThreadContext::get(eval)?;
    let payload = cx.payload::<CqueryDialectPayload>().ok_or_else(|| {
        anyhow::anyhow!("internal error: cquery dialect function outside cquery --output=starlark")
    })?;
    Ok(payload.get_configuration.clone())
}

/// Java's inner class `CqueryDialectGlobals`.
// Java: com/google/devtools/build/lib/query2/cquery/StarlarkOutputFormatterCallback.java:59 StarlarkOutputFormatterCallback.CqueryDialectGlobals
#[starlark_module]
fn cquery_dialect_globals(builder: &mut GlobalsBuilder) {
    // Java: com/google/devtools/build/lib/query2/cquery/StarlarkOutputFormatterCallback.java:66 StarlarkOutputFormatterCallback.CqueryDialectGlobals.buildOptions
    fn build_options<'v>(
        #[starlark(require = pos)] target: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        let target = cquery_node_param("build_options", target)?;
        let get_configuration = payload_lookup(eval)?;
        let heap = eval.heap();
        let config = target
            .get_configuration_key()
            .and_then(|key| get_configuration(&key));

        let Some(config) = config else {
            // config is null for input file configured targets.
            return Ok(Value::new_none());
        };

        let build_options = config.get_options();
        // ImmutableMap.builder(): insertion order; buildOrThrow fails on a repeated key.
        let mut result: IndexMap<String, Value<'v>> = IndexMap::new();
        let mut put = |key: String, value: Value<'v>| {
            if result.insert(key.clone(), value).is_some() {
                panic!("IllegalArgumentException: Multiple entries with same key: {key}");
            }
        };

        // Add all build options from each native configuration fragment.
        for (option_class, options) in build_options.get_native_options() {
            for def in blaze_options::parser::OptionsParser::get_option_definitions(
                option_class.options_class(),
            ) {
                let option_name = def.get_option_name();
                let option_key = format!("{COMMAND_LINE_OPTION_PREFIX}{option_name}");

                let option_value = def.get_value(options);

                // fromJava is not a deep validity check.
                // It is not guaranteed to catch all errors,
                // nor does it specify how it reports the errors it does find.
                // Passing arbitrary Java values into the Starlark interpreter
                // is not safe.
                // TODO(cparsons,twigg): fix it: convert value by explicit cases.
                if let Some(v) =
                    from_java(&JavaObject::from_option_value(option_value.as_ref()), heap)
                {
                    put(option_key, v);
                }
                // else: optionValue is not a valid Starlark value, so skip this option.
                // (e.g. tristate; a map with null values)
            }
        }

        // Add Starlark options.
        for (label, value) in build_options.get_starlark_options() {
            let v = from_java(&starlark_option_to_java(value), heap).unwrap_or_else(|| {
                panic!("InvalidStarlarkValueException: Starlark option {label} = {value:?}")
            });
            put(label.to_string(), v);
        }
        // The method's ImmutableMap result becomes a Dict (Starlark.fromJava).
        Ok(heap.alloc(AllocDict(result)))
    }

    // Java: com/google/devtools/build/lib/query2/cquery/StarlarkOutputFormatterCallback.java:115 StarlarkOutputFormatterCallback.CqueryDialectGlobals.providers
    fn providers<'v>(
        #[starlark(require = pos)] target: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        let ct = cquery_node_param("providers", target)?;
        let semantics = ThreadContext::get(eval)?.semantics.clone();
        let module = eval.module();
        let heap = eval.heap();
        match get_providers_dict_for_query(&ct, module, &semantics) {
            None => Ok(Value::new_none()),
            Some(entries) => {
                let dict = heap.alloc(AllocDict(entries));
                // Dict.Builder.buildImmutable()
                blaze_starlark::dict::dict_immutable_copy_of(heap, dict)
            }
        }
    }
}

/// Java `ConfiguredTarget.getProvidersDictForQuery()`: provider names to their values, `None`
/// for the classes that do not override the interface's default (package groups, environment
/// groups).
// Java: com/google/devtools/build/lib/analysis/ConfiguredTarget.java:150 ConfiguredTarget.getProvidersDictForQuery
fn get_providers_dict_for_query<'v>(
    ct: &ConfiguredTarget,
    module: &Module<'v>,
    semantics: &StarlarkSemantics,
) -> Option<IndexMap<String, Value<'v>>> {
    match ct {
        // Java: com/google/devtools/build/lib/analysis/configuredtargets/RuleConfiguredTarget.java:332 RuleConfiguredTarget.getProvidersDictForQuery
        ConfiguredTarget::Rule(rule) => {
            Some(to_providers_dict_for_query(ct, rule.providers(), module))
        }
        // Java: com/google/devtools/build/lib/analysis/configuredtargets/MergedConfiguredTarget.java:376 MergedConfiguredTarget.getProvidersDictForQuery
        ConfiguredTarget::Merged(merged) => Some(to_providers_dict_for_query(
            ct,
            merged.get_providers_for_query(),
            module,
        )),
        // Java: com/google/devtools/build/lib/rules/AliasConfiguredTarget.java:241 AliasConfiguredTarget.getProvidersDictForQuery
        ConfiguredTarget::Alias(alias) => {
            get_providers_dict_for_query(alias.get_actual_no_follow(), module, semantics)
        }
        ConfiguredTarget::InputFile(_) => {
            Some(file_providers_dict_for_query(ct, module, semantics))
        }
        // Java: com/google/devtools/build/lib/analysis/configuredtargets/OutputFileConfiguredTarget.java:124 OutputFileConfiguredTarget.getProvidersDictForQuery
        ConfiguredTarget::OutputFile(output) => {
            let mut dict = file_providers_dict_for_query(ct, module, semantics);
            // addStarlarkProviderIfPresent(dict, InstrumentedFilesInfo.STARLARK_CONSTRUCTOR)
            // addStarlarkProviderIfPresent(dict, OutputGroupInfo.STARLARK_CONSTRUCTOR)
            // (the provider map holds what rawGetStarlarkProvider returns for both keys).
            for key in [INSTRUMENTED_FILES_INFO.key(), OUTPUT_GROUP_INFO.key()] {
                if let Some(info) = output.providers().get(&key) {
                    try_add_provider_for_query(
                        &mut dict,
                        &ProviderMapKey::Info(key.clone()),
                        Some(info_value(module, info)),
                    );
                }
            }
            // addNativeProviderFromRuleIfPresent(dict, RequiredConfigFragmentsProvider.class):
            // a RequiredConfigFragmentsProvider is not a Starlark value, so tryAddProviderForQuery
            // leaves it out.
            Some(dict)
        }
        // The interface default (PackageGroupConfiguredTarget and
        // EnvironmentGroupConfiguredTarget do not override it).
        ConfiguredTarget::PackageGroup(_) | ConfiguredTarget::EnvironmentGroup(_) => None,
    }
}

/// Java `FileConfiguredTarget.getProvidersDictForQuery`.
// Java: com/google/devtools/build/lib/analysis/configuredtargets/FileConfiguredTarget.java:113 FileConfiguredTarget.getProvidersDictForQuery
fn file_providers_dict_for_query<'v>(
    ct: &ConfiguredTarget,
    module: &Module<'v>,
    semantics: &StarlarkSemantics,
) -> IndexMap<String, Value<'v>> {
    let heap = module.heap();
    let mut dict = IndexMap::new();
    // tryAddProviderForQuery(dict, VisibilityProvider.class, this)
    dict.insert(
        "VisibilityProvider".to_owned(),
        target_to_starlark(ct, heap, semantics),
    );
    // createFileProvider() / createFilesToRunProvider(): the provider map of a file target holds
    // the providers these create.
    if let Some(p) = ct.providers().get_provider::<FileProvider>() {
        dict.insert(
            "FileProvider".to_owned(),
            heap.alloc(StarlarkFileProvider(Arc::new(p.clone()))),
        );
    }
    if let Some(p) = ct.providers().get_provider::<FilesToRunProvider>() {
        dict.insert("FilesToRunProvider".to_owned(), heap.alloc(p.clone()));
    }
    // DefaultInfo is not stored as a provider, but Starlark targets still observe it on
    // dependencies.
    try_add_provider_for_query(
        &mut dict,
        &ProviderMapKey::Info(DEFAULT_INFO.key()),
        Some(info_value(module, &get_default_info(ct))),
    );
    dict
}

/// Java `AbstractConfiguredTarget.toProvidersDictForQuery`.
// Java: com/google/devtools/build/lib/analysis/configuredtargets/AbstractConfiguredTarget.java:278 AbstractConfiguredTarget.toProvidersDictForQuery
fn to_providers_dict_for_query<'v>(
    ct: &ConfiguredTarget,
    providers: &TransitiveInfoProviderMap,
    module: &Module<'v>,
) -> IndexMap<String, Value<'v>> {
    let heap = module.heap();
    let mut dict = IndexMap::new();
    for (key, instance) in providers.entries() {
        let value = match instance {
            ProviderInstance::Info(v) | ProviderInstance::Legacy(v) => Some(info_value(module, v)),
            ProviderInstance::Native(p) => native_provider_value(&**p, heap),
        };
        try_add_provider_for_query(&mut dict, key, value);
    }
    // DefaultInfo is not stored as a provider, but Starlark targets still observe it on
    // dependencies.
    try_add_provider_for_query(
        &mut dict,
        &ProviderMapKey::Info(DEFAULT_INFO.key()),
        Some(info_value(module, &get_default_info(ct))),
    );
    dict
}

/// A frozen provider value on `heap`.
fn info_value<'v>(module: &Module<'v>, v: &OwnedFrozenValue) -> Value<'v> {
    frozen_into(module, v)
}

/// `Starlark.fromJava(providerInstance, null)` of a native `TransitiveInfoProvider`: the
/// provider itself when its class implements `StarlarkValue` (`FileProvider` through
/// `FileProviderApi`, `FilesToRunProvider` through `FilesToRunProviderApi`), else `None`
/// (`InvalidStarlarkValueException`).
// PORT: of the native providers Blaze ports, only these two classes implement StarlarkValue in
// Bazel 9.3.2 (the others implement TransitiveInfoProvider alone or are records).
fn native_provider_value<'v>(
    p: &dyn blaze_analysis::provider_map::TransitiveInfoProvider,
    heap: Heap<'v>,
) -> Option<Value<'v>> {
    if let Some(p) = p.as_any().downcast_ref::<FileProvider>() {
        return Some(heap.alloc(StarlarkFileProvider(Arc::new(p.clone()))));
    }
    if let Some(p) = p.as_any().downcast_ref::<FilesToRunProvider>() {
        return Some(heap.alloc(p.clone()));
    }
    None
}

/// Attempts to add a provider instance to `dict` under an unspecified stringification of the
/// given key. Takes no action if the provider instance is not a valid Starlark value.
// Java: com/google/devtools/build/lib/analysis/configuredtargets/AbstractConfiguredTarget.java:297 AbstractConfiguredTarget.tryAddProviderForQuery
fn try_add_provider_for_query<'v>(
    dict: &mut IndexMap<String, Value<'v>>,
    key: &ProviderMapKey,
    provider_instance: Option<Value<'v>>,
) {
    // The key may be of many types, but we need a string for the intended use.
    let key_as_string = match key {
        ProviderMapKey::Legacy(string) => string.clone(),
        ProviderMapKey::Info(ProviderKey::Starlark {
            extension_label,
            exported_name,
        }) => format!("{extension_label}%{exported_name}"),
        // BuiltinProvider.Key.toString(): the provider's name.
        ProviderMapKey::Info(ProviderKey::Builtin { name, .. }) => name.clone(),
        ProviderMapKey::Class(_) => {
            // aClass.getSimpleName(): the class of a native provider is only known from its
            // instance, which `native_provider_value` converted.
            match provider_instance {
                Some(v) => native_simple_class_name(v).to_owned(),
                None => return,
            }
        }
    };
    // This is OK. If this is not a valid StarlarkValue, we just leave it out of the map.
    if let Some(v) = provider_instance {
        // Dict.Builder.put: overwriting any previous entry with the same key.
        dict.insert(key_as_string, v);
    }
}

/// The Java simple class name of a native provider converted by [`native_provider_value`].
fn native_simple_class_name(v: Value) -> &'static str {
    if v.downcast_ref::<StarlarkFileProvider>().is_some() {
        "FileProvider"
    } else if v.downcast_ref::<FilesToRunProvider>().is_some() {
        "FilesToRunProvider"
    } else {
        unreachable!("native_provider_value converts only these classes")
    }
}

/// Starlark output formatter for cquery results. Each configured target will result in an
/// evaluation of the Starlark expression specified by `--expr`.
// Java: com/google/devtools/build/lib/query2/cquery/StarlarkOutputFormatterCallback.java:58 StarlarkOutputFormatterCallback
pub struct StarlarkOutputFormatterCallback<'a> {
    base: CqueryThreadsafeCallback<'a>,
    // Starlark function with single required parameter "target", a CqueryNode query result.
    format_fn: OwnedFrozenValue,
    starlark_semantics: Arc<StarlarkSemantics>,
    globals: Globals,
    skyframe_executor: &'a SkyframeExecutor,
    event_handler: Arc<dyn ExtendedEventHandler>,
}

/// The predeclared names of the cquery dialect: `CqueryDialectGlobals` and
/// `StarlarkGlobalsImpl.getUtilToplevelsForCquery()` (depset, json, proto, struct), on the
/// universe.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkGlobalsImpl.java:68 StarlarkGlobalsImpl.getUtilToplevelsForCquery
fn cquery_globals(semantics: &StarlarkSemantics) -> Globals {
    let mut b = blaze_starlark::env::universe_builder(semantics);
    cquery_dialect_globals(&mut b);
    // addCommonUtilToplevels
    blaze_starlark::depset::register_depset(&mut b);
    blaze_starlark::json::register_json(&mut b);
    blaze_packages::proto::register_proto(&mut b);
    blaze_starlark::structs::register_struct(&mut b);
    b.build()
}

fn to_event_location(l: &Location) -> blaze_events::Location {
    blaze_events::Location::new(l.file.clone(), l.line as i32, l.column as i32)
}

impl<'a> StarlarkOutputFormatterCallback<'a> {
    /// `workspace` is the directory a relative `--starlark:file` is read from (PORT: Java reads
    /// it relative to the server process's working directory, which Bazel's client sets to the
    /// workspace before it starts the server).
    // Java: com/google/devtools/build/lib/query2/cquery/StarlarkOutputFormatterCallback.java:128 StarlarkOutputFormatterCallback.<init>
    pub fn new(
        base: CqueryThreadsafeCallback<'a>,
        skyframe_executor: &'a SkyframeExecutor,
        starlark_semantics: &StarlarkSemantics,
        workspace: &blaze_vfs::Path,
    ) -> Result<Self, QueryException> {
        let options = base.options;
        let event_handler = base.event_handler.clone();
        let starlark_semantics = Arc::new(starlark_semantics.clone());

        let (file_name, source, exception_message_prefix): (String, String, &str);
        if !options.file.is_empty() {
            if !options.expr.is_empty() {
                return Err(QueryException::new(
                    None,
                    "You must not specify both --starlark:expr and --starlark:file",
                    query::Code::IllegalFlagCombination,
                ));
            }
            exception_message_prefix = "invalid --starlark:file: ";
            // ParserInput.readFile(options.file): `fromUTF8`, so the file's text is a Unicode
            // string (not an internal one, unlike BUILD and .bzl sources); a non-ASCII literal
            // leaves through the ISO-8859-1 streams as its Latin-1 byte or `?`.
            // Java: net/starlark/java/syntax/ParserInput.java:61 ParserInput.readFile
            match read_file(workspace, &options.file) {
                Ok(bytes) => {
                    file_name = options.file.clone();
                    source = blaze_util::string_encoding::java_new_string_utf8(&bytes);
                }
                Err(message) => {
                    return Err(QueryException::new(
                        None,
                        format!("{exception_message_prefix}failed to read {message}"),
                        query::Code::QueryFileReadFailure,
                    ));
                }
            }
        } else {
            exception_message_prefix = "invalid --starlark:expr: ";
            let expr = if options.expr.is_empty() {
                "str(target.label)"
            } else {
                options.expr.as_str()
            };
            // Validate that options.expr is a pure expression (for example, that it does not
            // attempt to escape its scope via unbalanced parens).
            let errors = blaze_starlark::syntax::java::parse_expression(
                "--starlark:expr",
                expr,
                blaze_starlark::syntax::java::JavaOptions::DEFAULT,
            );
            if !errors.is_empty() {
                return Err(QueryException::with_configurable_query_code(
                    None,
                    format!(
                        "{exception_message_prefix}{}",
                        blaze_starlark::error::syntax_errors_message(&errors)
                    ),
                    configurable_query::Code::StarlarkSyntaxError,
                ));
            }

            // Create a synthetic file that defines a function with single parameter "target",
            // whose body is provided by the user's expression. Dynamic errors will have the
            // wrong column.
            file_name = "--starlark:expr".to_owned();
            source = format!("def format(target): return ({expr})");
        }

        // StarlarkFile.parse(input, FileOptions.DEFAULT)
        let parsed = blaze_starlark::syntax::java::parse_file(
            &file_name,
            &source,
            blaze_starlark::syntax::java::JavaOptions::DEFAULT,
        );
        if !parsed.errors.is_empty() {
            Event::replay_syntax_errors_on(
                &*event_handler,
                parsed
                    .errors
                    .iter()
                    .map(|e| (to_event_location(&e.location), e.message.clone())),
            );
        }

        let globals = cquery_globals(&starlark_semantics);
        // Starlark.execFile(input, FileOptions.DEFAULT, module, thread), with a transient thread
        // (default print handler).
        let names: Vec<String> = globals.names().map(|n| n.as_str().to_owned()).collect();
        let no_guard = |_: &str| None;
        let exec = blaze_starlark::eval::compile_and_resolve(
            FileKind::Bzl,
            FileOptions::DEFAULT,
            &file_name,
            source,
            names,
            &no_guard,
        )
        .and_then(|compiled| {
            let ctx = ThreadContext::new(
                Phase::Other,
                starlark_semantics.clone(),
                Arc::new(StderrPrint),
            );
            blaze_starlark::eval::exec_module(
                compiled,
                &globals,
                &LoadMap::new(),
                &ctx,
                &EvalOptions::default(),
            )
        });
        let module = match exec {
            Ok(module) => module,
            Err(e) => return Err(starlark_exception(exception_message_prefix, &e)),
        };

        let format_fn = module.get_option("format").ok().flatten();
        let Some(format_fn) = format_fn else {
            return Err(QueryException::with_configurable_query_code(
                None,
                format!("{exception_message_prefix}file does not define 'format'"),
                configurable_query::Code::FormatFunctionError,
            ));
        };
        // instanceof StarlarkFunction: a `def` (or lambda); builtins have another type.
        let type_ = format_fn.value().to_value().get_type();
        if type_ != "function" {
            return Err(QueryException::with_configurable_query_code(
                None,
                format!("{exception_message_prefix}got {type_} for 'format', want function"),
                configurable_query::Code::FormatFunctionError,
            ));
        }
        let (parameter_names, _) = blaze_packages::starlark_defs::attr_module::function_parameters(
            format_fn.value().to_value(),
        );
        if parameter_names.len() != 1 {
            return Err(QueryException::with_configurable_query_code(
                None,
                format!("{exception_message_prefix}'format' function must take exactly 1 argument"),
                configurable_query::Code::FormatFunctionError,
            ));
        }

        Ok(StarlarkOutputFormatterCallback {
            base,
            format_fn,
            starlark_semantics,
            globals,
            skyframe_executor,
            event_handler,
        })
    }
}

/// The `QueryException` of a failed `Starlark.execFile`: a `SyntaxError.Exception` gives its
/// message (`STARLARK_SYNTAX_ERROR`), an `EvalException` its message with stack
/// (`STARLARK_EVAL_ERROR`).
fn starlark_exception(prefix: &str, e: &EvalError) -> QueryException {
    match &e.kind {
        blaze_starlark::error::EvalErrorKind::Syntax(_) => {
            QueryException::with_configurable_query_code(
                None,
                format!("{prefix}{}", e.message),
                configurable_query::Code::StarlarkSyntaxError,
            )
        }
        blaze_starlark::error::EvalErrorKind::Eval
        | blaze_starlark::error::EvalErrorKind::StackOverflow => {
            QueryException::with_configurable_query_code(
                None,
                format!("{prefix}{}", e.message_with_stack()),
                configurable_query::Code::StarlarkEvalError,
            )
        }
        blaze_starlark::error::EvalErrorKind::Cancelled
        | blaze_starlark::error::EvalErrorKind::Panic => {
            panic!("{}", e.message_with_stack())
        }
    }
}

/// `Files.readAllBytes(Paths.get(file))`; the error is the `IOException`'s message.
fn read_file(workspace: &blaze_vfs::Path, file: &str) -> Result<Vec<u8>, String> {
    let resolved = if file.starts_with('/') {
        file.to_owned()
    } else {
        format!("{}/{}", workspace.get_path_string(), file)
    };
    match std::fs::read(blaze_util::string_encoding::internal_to_platform(&resolved)) {
        Ok(bytes) => Ok(bytes),
        // NoSuchFileException(file) and AccessDeniedException(file): the message is the path as
        // given.
        Err(e)
            if matches!(
                e.kind(),
                std::io::ErrorKind::NotFound | std::io::ErrorKind::PermissionDenied
            ) =>
        {
            Err(file.to_owned())
        }
        Err(e) => blaze_util::unported!(
            "ParserInput.readFile",
            "IOException message for {}: {}",
            file,
            e
        ),
    }
}

impl NamedThreadSafeOutputFormatterCallback<CqueryNode> for StarlarkOutputFormatterCallback<'_> {
    // Java: com/google/devtools/build/lib/query2/cquery/StarlarkOutputFormatterCallback.java:217 StarlarkOutputFormatterCallback.getName
    fn get_name(&self) -> String {
        "starlark".to_owned()
    }
}

impl ThreadSafeOutputFormatterCallback<CqueryNode> for StarlarkOutputFormatterCallback<'_> {}

impl Callback<CqueryNode> for StarlarkOutputFormatterCallback<'_> {
    fn process(&self, partial_result: &[CqueryNode]) -> Result<(), QueryError> {
        output_formatter_callback_process(self, partial_result)
    }
}

impl OutputFormatterCallback<CqueryNode> for StarlarkOutputFormatterCallback<'_> {
    // Java: com/google/devtools/build/lib/query2/cquery/StarlarkOutputFormatterCallback.java:222 StarlarkOutputFormatterCallback.processOutput
    fn process_output(&self, partial_result: &[CqueryNode]) -> Result<(), OutputFormatterError> {
        // CqueryThreadsafeCallback.getConfiguration, as the inner class CqueryDialectGlobals
        // calls it. PORT: the thread payload must own its data, so the lookup is a map of the
        // configurations in the graph (every configuration a configured target of the graph
        // has); Java looks the key up in Skyframe, which finds the same value.
        let mut configurations: HashMap<
            BuildConfigurationKey,
            Option<Arc<BuildConfigurationValue>>,
        > = HashMap::new();
        for key in self.skyframe_executor.get_transitive_configuration_keys() {
            if let Some(config_key) = key.downcast::<BuildConfigurationKey>() {
                let value = self.base.get_configuration(Some(config_key));
                configurations.insert(config_key.clone(), value);
            }
        }
        let configurations = Rc::new(configurations);
        for target in partial_result {
            let CqueryNode::ConfiguredTarget(ct) = target else {
                blaze_util::unported!(
                    "StarlarkOutputFormatterCallback.processOutput",
                    "an aspect node ({}) as the format function's argument",
                    target.get_label()
                );
            };
            // PORT: Java calls the format function on a thread limited to 500,000 computation
            // steps (thread.setMaxExecutionSteps(500_000L)); starlark-rust does not count Java's
            // steps, so the limit is not enforced.
            let ctx = ThreadContext::new(
                Phase::Other,
                self.starlark_semantics.clone(),
                Arc::new(StderrPrint),
            );
            let configurations = configurations.clone();
            let lookup: ConfigurationLookup =
                Rc::new(
                    move |key: &BuildConfigurationKey| match configurations.get(key) {
                        Some(c) => c.clone(),
                        None => blaze_util::unported!(
                            "CqueryThreadsafeCallback.getConfiguration",
                            "configuration {:?} is not in the graph",
                            key
                        ),
                    },
                );
            let ctx = ctx.with_payload(CqueryDialectPayload {
                get_configuration: lookup,
            });
            let semantics = self.starlark_semantics.clone();
            let _ = &self.globals;
            // Invoke formatFn with `target` argument.
            let result = with_evaluator(&ctx, None, &EvalOptions::default(), |module, eval| {
                let f = frozen_into(module, &self.format_fn);
                let arg = target_to_starlark(ct, module.heap(), &semantics);
                let res = eval
                    .eval_function(f, &[arg], &[])
                    .map_err(|e| EvalError::from_starlark(&e))?;
                // Starlark.str(result, thread.getSemantics())
                Ok(res.to_str())
            });
            match result {
                Ok(s) => self.base.add_result(s),
                Err(ex) => {
                    if !matches!(
                        ex.kind,
                        blaze_starlark::error::EvalErrorKind::Eval
                            | blaze_starlark::error::EvalErrorKind::StackOverflow
                    ) {
                        panic!("{}", ex.message_with_stack());
                    }
                    self.event_handler.handle(&Event::error(format!(
                        "Starlark evaluation error for {}: {}",
                        target.get_original_label(),
                        ex.message_with_stack()
                    )));
                    continue;
                }
            }
        }
        Ok(())
    }

    fn close(&self, fail_fast: bool) -> Result<(), OutputFormatterError> {
        self.base
            .close(fail_fast)
            .map_err(OutputFormatterError::from)
    }

    fn io_exception_slot(&self) -> &IoExceptionSlot {
        &self.base.io_exception
    }
}
