//! `blaze mod`: `bazel/commands/ModCommand.java` (`exec` / `execInternal`). The subcommand logic
//! (argument validation, `ModExecutor` and the output formatters) is
//! `blaze_bzlmod::modcommand::mod_command`. `RepoOutputFormatter` (`mod show_repo`) is here
//! because it needs the repository rules (blaze_repo_rules), the attribute formatter
//! (blaze_query) and the protos, which all depend on blaze_bzlmod.

use std::collections::HashMap;
use std::io::Write;
use std::sync::Arc;

use blaze_bzlmod::modcommand::invalid_argument_exception::ModCommandCode;
use blaze_bzlmod::modcommand::mod_command::report_and_create_failure_result;
use blaze_bzlmod::modcommand::mod_command::{
    ModCommandResult, ShowRepoBackend, exec_after_evaluation, exec_before_evaluation,
    exec_dump_repo_mapping, report_evaluation_error,
};
use blaze_bzlmod::modcommand::output_formatters::Printer;
use blaze_bzlmod::skyframe::keys::{BazelModTidyKey, BazelModTidyValue};
use blaze_bzlmod::skyframe::{BazelDepGraphKey, BazelModuleInspectorKey};
use blaze_bzlmod_model::AttributeValue;
use blaze_cmdline::{RepositoryMapping, RepositoryName};
use blaze_events::{Event, EventHandler, ExtendedEventHandler};
use blaze_optdefs::command::loading_phase_threads_option::LoadingPhaseThreadsOption;
use blaze_optdefs::misc::mod_options::{ModOptions, ModSubcommand, OutputFormat};
use blaze_options::OptionScalar;
use blaze_packages::target::LabelPrinter;
use blaze_packages::types::{AttrValue, Type};
use blaze_protos::blaze_query::Repository;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};
use blaze_protos::java::JavaMessage;
use blaze_protos::json::JsonFormat;
use blaze_repo_rules::repository::repo_definition::{
    RepoDefinition, RepoDefinitionKey, RepoDefinitionValue,
};
use blaze_shell::{CommandBuilder, CommandExecutionError};
use blaze_sky_loading::repos::RepositoryMappingKey;
use blaze_sky_top::skyframe_executor::SkyframeExecutor;
use blaze_skyframe::SkyKey;
use blaze_util::string_encoding::internal_to_unicode;

use crate::command::{BlazeCommand, BlazeCommandResult, CommandAnnotation};
use crate::command_environment::CommandEnvironment;
use crate::command_registry::MOD_COMMAND;
use crate::commands::query::{AfterCommand, create_skyframe_executor, reporter};
use crate::exit_code::DetailedExitCode;
use crate::options_result::OptionsParsingResult;

/// Queries the Bzlmod external dependency graph.
// Java: com/google/devtools/build/lib/bazel/commands/ModCommand.java:119 ModCommand
pub struct ModCommand;

impl BlazeCommand for ModCommand {
    fn annotation(&self) -> &'static CommandAnnotation {
        &MOD_COMMAND
    }

    // PORT: `exec` posts `NoBuildEvent` before and `NoBuildRequestFinishedEvent` after
    // `execInternal`; the UI's subscribers (`noBuild`, `noBuildFinished`) are called here, the
    // build event stream's are not (no build event stream subscriber of `mod` is ported).
    // Java: com/google/devtools/build/lib/bazel/commands/ModCommand.java:124 ModCommand.exec
    fn exec(
        &self,
        env: &mut CommandEnvironment,
        options: &OptionsParsingResult,
    ) -> BlazeCommandResult {
        // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:777 UiEventHandler.noBuild
        if let Some(handler) = env.get_ui_event_handler() {
            handler.no_build(true);
        }
        let result = mod_exec_internal(env, options);
        // Java: com/google/devtools/build/lib/runtime/UiEventHandler.java:788 UiEventHandler.noBuildFinished
        if let Some(handler) = env.get_ui_event_handler() {
            handler.no_build_finished();
        }
        result
    }
}

// Java: com/google/devtools/build/lib/bazel/commands/ModCommand.java:216 ModCommand.execInternal
fn mod_exec_internal(
    env: &mut CommandEnvironment,
    options: &OptionsParsingResult,
) -> BlazeCommandResult {
    let reporter = reporter(env);
    let mod_options: ModOptions = options.get_options().expect("ModOptions");
    let residue = options.get_residue();
    let request = match exec_before_evaluation(&residue, &mod_options, reporter.as_ref()) {
        Ok(r) => r,
        Err(r) => return to_command_result(r),
    };
    let subcommand = request.subcommand;
    let repo_mapping_keys: Vec<RepositoryMappingKey> = request
        .repo_mapping_repo_names
        .iter()
        .cloned()
        .map(RepositoryMappingKey::key)
        .collect();
    let threads = options
        .get_options::<LoadingPhaseThreadsOption>()
        .map(|o| o.threads.max(1) as usize)
        .unwrap_or(blaze_sky_top::skyframe_executor::DEFAULT_THREAD_COUNT);

    // env.syncPackageLoading(options)
    let skyframe_executor = match create_skyframe_executor(env, None) {
        Ok(e) => e,
        Err(result) => return result,
    };
    let _after_command = AfterCommand(&skyframe_executor);

    // ImmutableSet.Builder<SkyKey>: duplicates collapse, first occurrence wins.
    let mut keys: Vec<SkyKey> = Vec::new();
    if subcommand == ModSubcommand::DUMP_REPO_MAPPING {
        for k in &repo_mapping_keys {
            let key = SkyKey::new(k.clone());
            if !keys.contains(&key) {
                keys.push(key);
            }
        }
    } else if subcommand == ModSubcommand::TIDY {
        keys.push(SkyKey::new(BazelModTidyKey));
    } else {
        keys.push(SkyKey::new(BazelDepGraphKey));
        keys.push(SkyKey::new(BazelModuleInspectorKey));
    }
    let evaluation_result = match skyframe_executor.evaluate(
        &keys,
        /* keep_going= */ true,
        threads,
        reporter.clone(),
    ) {
        Ok(r) => r,
        Err(_) => return interrupted_result(env, "mod command interrupted: null"),
    };
    if evaluation_result.has_error() {
        // PORT: the CyclesReporter (BzlmodRepoCycleReporter, BzlLoadCycleReporter) is not
        // ported; cycles report the generic message.
        let message = evaluation_result
            .get_first_error()
            .and_then(|e| e.get_exception())
            .map(|e| e.to_string());
        return to_command_result(report_evaluation_error(
            reporter.as_ref(),
            message.as_deref(),
        ));
    }

    let out_err = env.get_reporter_out_err().clone();
    let mut out = crate::commands::query::ReporterStdout(out_err.get_output_stream().clone());

    // Handle commands that do not require BazelModuleInspectorValue.
    if subcommand == ModSubcommand::DUMP_REPO_MAPPING {
        let repo_mapping_values: Vec<_> = repo_mapping_keys
            .iter()
            .map(|k| {
                evaluation_result
                    .get(k)
                    .expect("RepositoryMappingValue without error")
            })
            .collect();
        let mappings: Vec<Option<&RepositoryMapping>> = repo_mapping_values
            .iter()
            .map(|v| {
                if v.is_not_found() {
                    None
                } else {
                    Some(v.repository_mapping().expect("repository mapping"))
                }
            })
            .collect();
        let result = exec_dump_repo_mapping(
            &request.repo_mapping_repo_names,
            &mappings,
            &mod_options,
            reporter.as_ref(),
            &mut out,
        );
        return to_command_result(result);
    } else if subcommand == ModSubcommand::TIDY {
        // tidy doesn't take extra arguments.
        if !request.args.is_empty() {
            return to_command_result(report_and_create_failure_result(
                reporter.as_ref(),
                "the 'tidy' command doesn't take extra arguments",
                ModCommandCode::TOO_MANY_ARGUMENTS,
            ));
        }
        let mod_tidy_value = evaluation_result
            .get(&BazelModTidyKey)
            .expect("BazelModTidyValue without error");
        return run_tidy(env, reporter.as_ref(), &mod_tidy_value);
    }

    let dep_graph = evaluation_result
        .get(&BazelDepGraphKey)
        .expect("BazelDepGraphValue without error");
    let inspector = evaluation_result
        .get(&BazelModuleInspectorKey)
        .expect("BazelModuleInspectorValue without error");
    let mut show_repo_backend = RepoDefinitions {
        skyframe_executor: &skyframe_executor,
        threads,
        event_handler: reporter.clone(),
        values: HashMap::new(),
    };
    let result = exec_after_evaluation(
        subcommand,
        &request.args,
        &mod_options,
        &dep_graph,
        &inspector,
        reporter.as_ref(),
        &mut out,
        Some(&mut show_repo_backend),
    );
    let _ = out.flush();
    if let ModCommandResult::Interrupted { message } = result {
        return interrupted_detailed_exit_code(message);
    }
    to_command_result(result)
}

// Java: com/google/devtools/build/lib/bazel/commands/ModCommand.java:690 ModCommand.runTidy
fn run_tidy(
    env: &CommandEnvironment,
    reporter: &dyn ExtendedEventHandler,
    mod_tidy_value: &BazelModTidyValue,
) -> BlazeCommandResult {
    // ImmutableListMultimap: keys in first-insertion order, values in order.
    let mut all_commands_per_file: indexmap::IndexMap<&blaze_vfs::PathFragment, Vec<&str>> =
        indexmap::IndexMap::new();
    for fixup in &mod_tidy_value.fixups {
        for (path, commands) in &fixup.module_file_path_to_buildozer_commands {
            all_commands_per_file
                .entry(path)
                .or_default()
                .extend(commands.iter().map(String::as_str));
        }
    }
    let mut buildozer_input = String::new();
    for module_file_path in &mod_tidy_value.module_file_paths {
        buildozer_input.push_str("//");
        buildozer_input.push_str(&module_file_path.to_string());
        buildozer_input.push_str(":all|");
        for command in all_commands_per_file
            .get(module_file_path)
            .into_iter()
            .flatten()
        {
            buildozer_input.push_str(command);
            buildozer_input.push('|');
        }
        buildozer_input.push_str("format\n");
    }

    let stdin_bytes = blaze_util::string_encoding::latin1_encode_lossy(&buildozer_input);
    let mut stdin: &[u8] = &stdin_bytes;
    let mut command_builder = CommandBuilder::new(env.get_client_env());
    command_builder.set_working_dir(
        env.get_workspace()
            .expect("the command runs in a workspace"),
    );
    command_builder.add_arg(&mod_tidy_value.buildozer.get_path_string());
    command_builder.add_arg("-f");
    command_builder.add_arg("-");
    let executed = command_builder
        .build()
        .execute_async_with_input(&mut stdin, /* kill_subprocess_on_interrupt= */ true)
        .map_err(|e| CommandExecutionError::Command(e.into()))
        .and_then(|mut future| future.get());
    if let Err(e) = executed {
        let mut suffix = String::new();
        let message = match &e {
            CommandExecutionError::Command(command_exception) => {
                if let Some(abnormal_termination_exception) =
                    command_exception.as_abnormal_termination()
                {
                    if abnormal_termination_exception
                        .get_result()
                        .termination_status()
                        .get_raw_exit_code()
                        == 3
                    {
                        // Buildozer exits with exit code 3 if it didn't make any changes.
                        return report_and_create_tidy_result(reporter, mod_tidy_value);
                    }
                    suffix = format!(
                        ":\n{}",
                        blaze_util::string_encoding::latin1_decode(
                            &abnormal_termination_exception.get_result().get_stderr()
                        )
                    );
                }
                command_exception.get_message().to_owned()
            }
            // InterruptedException.getMessage() is null.
            CommandExecutionError::Interrupted(_) => "null".to_owned(),
        };
        return to_command_result(report_and_create_failure_result(
            reporter,
            &format!("Unexpected error while running buildozer: {message}{suffix}"),
            ModCommandCode::BUILDOZER_FAILED,
        ));
    }

    for fixup_event in &mod_tidy_value.fixups {
        reporter.handle(&Event::info(fixup_event.get_success_message()));
    }

    report_and_create_tidy_result(reporter, mod_tidy_value)
}

// Java: com/google/devtools/build/lib/bazel/commands/ModCommand.java:738 ModCommand.reportAndCreateTidyResult
fn report_and_create_tidy_result(
    reporter: &dyn ExtendedEventHandler,
    mod_tidy_value: &BazelModTidyValue,
) -> BlazeCommandResult {
    if mod_tidy_value.errors.is_empty() {
        BlazeCommandResult::success()
    } else {
        let n = mod_tidy_value.errors.len();
        to_command_result(report_and_create_failure_result(
            reporter,
            &format!(
                "Failed to process {} extension{} due to errors.",
                n,
                if n == 1 { "" } else { "s" }
            ),
            ModCommandCode::ERROR_DURING_GRAPH_INSPECTION,
        ))
    }
}

/// The `InterruptedException` branches of `ModCommand.execInternal`.
fn interrupted_result(env: &CommandEnvironment, error_message: &str) -> BlazeCommandResult {
    env.get_reporter().handle(&Event::error(error_message));
    interrupted_detailed_exit_code(error_message.to_owned())
}

// Java: com/google/devtools/build/lib/util/InterruptedFailureDetails.java:30 InterruptedFailureDetails.detailedExitCode
fn interrupted_detailed_exit_code(error_message: String) -> BlazeCommandResult {
    BlazeCommandResult::detailed_exit_code(DetailedExitCode::of_exit_code(
        crate::exit_code::ExitCode::INTERRUPTED,
        FailureDetail {
            message: error_message,
            category: Some(Category::Interrupted(failure_details::Interrupted {
                code: failure_details::interrupted::Code::Interrupted as i32,
            })),
        },
    ))
}

/// `ModCommand.reportAndCreateFailureResult`'s `BlazeCommandResult` (the message was reported).
// Java: com/google/devtools/build/lib/bazel/commands/ModCommand.java:813 ModCommand.createFailureResult
fn to_command_result(result: ModCommandResult) -> BlazeCommandResult {
    match result {
        ModCommandResult::Success => BlazeCommandResult::success(),
        ModCommandResult::Failure { message, code, .. } => {
            BlazeCommandResult::detailed_exit_code(DetailedExitCode::of(FailureDetail {
                message,
                category: Some(Category::ModCommand(failure_details::ModCommand {
                    code: failure_details::mod_command::Code::from_str_name(code)
                        .unwrap_or(failure_details::mod_command::Code::ModCommandUnknown)
                        as i32,
                })),
            }))
        }
        ModCommandResult::Interrupted { message } => interrupted_detailed_exit_code(message),
    }
}

/// The `SHOW_REPO` evaluation of `ModCommand.execInternal` (`targetRepoDefinitions`) and the
/// `RepoOutputFormatter` that `ModExecutor.showRepo` prints them with.
struct RepoDefinitions<'a> {
    skyframe_executor: &'a SkyframeExecutor,
    threads: usize,
    event_handler: Arc<dyn ExtendedEventHandler>,
    values: HashMap<RepositoryName, Arc<RepoDefinitionValue>>,
}

impl ShowRepoBackend for RepoDefinitions<'_> {
    // Java: com/google/devtools/build/lib/bazel/commands/ModCommand.java:543 ModCommand.execInternal
    fn evaluate_repo_definitions(
        &mut self,
        repos: &[RepositoryName],
    ) -> Result<Option<Option<String>>, ()> {
        let sky_keys: Vec<SkyKey> = repos
            .iter()
            .map(|r| SkyKey::new(RepoDefinitionKey::key(r.clone())))
            .collect();
        // prepareAndGet(skyKeys, evaluationContext): the context has the command's parallelism
        // and reporter, and keepGoing false.
        let result = self
            .skyframe_executor
            .evaluate(
                &sky_keys,
                /* keep_going= */ false,
                self.threads,
                self.event_handler.clone(),
            )
            .map_err(|_| ())?;
        if result.has_error() {
            return Ok(Some(
                result
                    .get_first_error()
                    .and_then(|e| e.get_exception())
                    .map(|e| e.to_string()),
            ));
        }
        for repo in repos {
            let value = result
                .get(&RepoDefinitionKey::key(repo.clone()))
                .expect("RepoDefinitionValue without error");
            self.values.insert(repo.clone(), value);
        }
        Ok(None)
    }

    fn is_not_found(&self, repo: &RepositoryName) -> bool {
        matches!(*self.values[repo], RepoDefinitionValue::NotFound)
    }

    fn print(
        &self,
        printer: &mut Printer<'_>,
        output_format: OutputFormat,
        key: &str,
        repo: &RepositoryName,
    ) {
        RepoOutputFormatter {
            printer,
            output_format,
        }
        .print(key, &self.values[repo]);
    }
}

/// Outputs repository definitions for `mod show_repo`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/RepoOutputFormatter.java:36 RepoOutputFormatter
struct RepoOutputFormatter<'p, 'o> {
    /// The `PrintWriter`; its `output_stream()` is the `OutputStream` under it.
    printer: &'p mut Printer<'o>,
    output_format: OutputFormat,
}

impl RepoOutputFormatter<'_, '_> {
    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/RepoOutputFormatter.java:51 RepoOutputFormatter.print
    fn print(&mut self, key: &str, repo_definition: &RepoDefinitionValue) {
        match self.output_format {
            OutputFormat::TEXT => self.print_starlark(key, repo_definition),
            OutputFormat::STREAMED_JSONPROTO | OutputFormat::STREAMED_PROTO => {
                // In proto output formats, we only print repo definitions, not overrides.
                if let RepoDefinitionValue::Found(repo_def_value) = repo_definition {
                    if self.output_format == OutputFormat::STREAMED_JSONPROTO {
                        self.print_proto_json(key, repo_def_value);
                    } else {
                        self.print_streamed_proto(key, repo_def_value);
                    }
                }
            }
            other => panic!(
                "IllegalArgumentException: Unknown output format: {}",
                other.java_to_string()
            ),
        }
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/RepoOutputFormatter.java:66 RepoOutputFormatter.printStarlark
    fn print_starlark(&mut self, key: &str, repo_definition: &RepoDefinitionValue) {
        if let RepoDefinitionValue::Found(repo_def_value) = repo_definition {
            self.printer.print(&format!("## {key}:\n"));
            self.print_starlark_definition(repo_def_value);
        }
        if let RepoDefinitionValue::RepoOverride(repo_path) = repo_definition {
            self.printer.print(&format!(
                "## {key}:\nBuiltin or overridden repo located at: {repo_path}\n\n"
            ));
        }
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/RepoOutputFormatter.java:78 RepoOutputFormatter.printStarlark
    fn print_starlark_definition(&mut self, repo_definition: &RepoDefinition) {
        let repo_rule = &repo_definition.repo_rule;
        let p = &mut *self.printer;
        p.print("load(\"");
        p.print(&bzl_file_label_canonical_form(repo_definition));
        p.print("\", \"");
        p.print(repo_rule.id.rule_name());
        p.print("\")\n");
        p.print(repo_rule.id.rule_name());
        p.print("(\n");
        p.print("  name = \"");
        p.print(&repo_definition.name);
        p.print("\",\n");
        if let Some(original_name) = &repo_definition.original_name {
            p.print("  _original_name = \"");
            p.print(original_name);
            p.print("\",\n");
        }
        for (name, value) in repo_definition.attr_values.attributes() {
            p.print("  ");
            p.print(name);
            p.print(" = ");
            p.print(&blaze_repo_rules::attr_values::starlark_repr(value));
            p.print(",\n");
        }
        p.print(")\n");
        // TODO: record and print the call stack for the repo definition itself?
        p.print("\n");
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/RepoOutputFormatter.java:104 RepoOutputFormatter.printStreamedProto
    fn print_streamed_proto(&mut self, key: &str, repo_definition: &RepoDefinition) {
        let serialized = serialize_repo_definition_as_proto(key, repo_definition);
        let mut bytes = Vec::new();
        serialized.write_delimited_to(&mut bytes);
        // Ignore IOException like PrintWriter.
        let _ = self.printer.output_stream().write_all(&bytes);
    }

    // Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/RepoOutputFormatter.java:113 RepoOutputFormatter.printProtoJson
    fn print_proto_json(&mut self, key: &str, repo_definition: &RepoDefinition) {
        let serialized = serialize_repo_definition_as_proto(key, repo_definition);
        match JsonFormat::printer()
            .omitting_insignificant_whitespace()
            .print(&serialized)
        {
            Ok(json) => self.printer.println(&json),
            Err(e) => panic!("IllegalArgumentException: {}", e.get_message()),
        }
    }
}

/// `repoRule.id().bzlFileLabel().getUnambiguousCanonicalForm()`.
fn bzl_file_label_canonical_form(repo_definition: &RepoDefinition) -> String {
    match repo_definition.repo_rule.id.bzl_file_label() {
        Some(label) => label.get_unambiguous_canonical_form(),
        None => panic!("java.lang.NullPointerException: repo rule without a .bzl file label"),
    }
}

// Java: com/google/devtools/build/lib/bazel/bzlmod/modcommand/RepoOutputFormatter.java:122 RepoOutputFormatter.serializeRepoDefinitionAsProto
fn serialize_repo_definition_as_proto(key: &str, repo_definition: &RepoDefinition) -> Repository {
    let repo_rule = &repo_definition.repo_rule;

    let mut pb_builder = Repository {
        canonical_name: Some(u(&repo_definition.name)),
        repo_rule_name: Some(u(repo_rule.id.rule_name())),
        repo_rule_bzl_label: Some(u(&bzl_file_label_canonical_form(repo_definition))),
        ..Default::default()
    };

    // TODO: record and print the call stack for the repo definition itself?

    if key.starts_with('@') {
        if !key.starts_with("@@") {
            pb_builder.apparent_name = Some(u(key));
        }
    } else {
        pb_builder.module_key = Some(u(key));
    }
    if let Some(original_name) = &repo_definition.original_name {
        pb_builder.original_name = Some(u(original_name));
    }

    for (attr_name, &index) in &repo_rule.attribute_indices {
        let attr_definition = &repo_rule.attributes[index];

        let stored = repo_definition.attr_values.get(attr_name);
        let explicitly_specified = stored.is_some();
        let converted;
        let attr_value: Option<&AttrValue> = match stored {
            Some(value) => {
                converted = stored_attr_value(attr_definition.get_type(), value);
                Some(&converted)
            }
            None => attr_definition.get_default_value_unchecked(),
        };
        // AttributeFormatter.getAttributeProto(Attribute, ...) names the attribute
        // `attr.getName()` (`$foo` for a private `_foo`).
        let serialized_attribute = blaze_query::compat::attribute_formatter::get_attribute_proto(
            attr_definition.get_name(),
            attr_definition.get_type(),
            attr_value,
            explicitly_specified,
            /* encode_boolean_and_tri_state_as_integer_and_string= */ true,
            /* source_aspect= */ None,
            /* include_attribute_source_aspects= */ false,
            &LabelPrinter::legacy(),
        );
        pb_builder.attribute.push(serialized_attribute);
    }

    pb_builder
}

/// `StringEncoding.internalToUnicode` as an owned string (proto string fields).
fn u(s: &str) -> String {
    internal_to_unicode(s).into_owned()
}

/// The value `AttributeFormatter.writeAttributeValueToBuilder` reads from a stored attribute
/// value (Java hands it the stored Starlark object: a `StarlarkInt`, `String`, `Boolean`,
/// `Label`, `StarlarkList` or `Dict`, which it casts according to the attribute's type).
// PORT: Blaze's attribute formatter takes the converted form; the stored values were type-checked
// by `RepoRule.instantiate`, so each kind maps to the converted form of the attribute's type.
fn stored_attr_value(type_: Type, value: &AttributeValue) -> AttrValue {
    fn string(v: &AttributeValue) -> String {
        match v {
            AttributeValue::String(s) => s.clone(),
            other => panic!("ClassCastException: {other:?} is not a String"),
        }
    }
    fn label(v: &AttributeValue) -> blaze_cmdline::Label {
        match v {
            AttributeValue::Label(l) => l.clone(),
            other => panic!("ClassCastException: {other:?} is not a Label"),
        }
    }
    fn int(v: &AttributeValue) -> i32 {
        match v {
            // StarlarkInt.toIntUnchecked
            AttributeValue::Int(i) => *i as i32,
            other => panic!("ClassCastException: {other:?} is not a StarlarkInt"),
        }
    }
    fn list(v: &AttributeValue) -> &[AttributeValue] {
        match v {
            AttributeValue::List(items) => items,
            other => panic!("ClassCastException: {other:?} is not a Collection"),
        }
    }
    fn dict(v: &AttributeValue) -> &[(AttributeValue, AttributeValue)] {
        match v {
            AttributeValue::Dict(entries) => entries,
            other => panic!("ClassCastException: {other:?} is not a Map"),
        }
    }
    match type_ {
        Type::Integer => AttrValue::Int(int(value)),
        Type::String | Type::StringNoIntern => AttrValue::String(string(value)),
        Type::Boolean => match value {
            AttributeValue::Bool(b) => AttrValue::Bool(*b),
            other => panic!("ClassCastException: {other:?} is not a Boolean"),
        },
        Type::Label | Type::NodepLabel | Type::DormantLabel | Type::GenqueryScopeType => {
            AttrValue::Label(label(value))
        }
        Type::StringList => AttrValue::StringList(list(value).iter().map(string).collect()),
        Type::StringSet => AttrValue::StringSet(list(value).iter().map(string).collect()),
        Type::IntegerList => AttrValue::IntList(list(value).iter().map(int).collect()),
        Type::LabelList
        | Type::NodepLabelList
        | Type::DormantLabelList
        | Type::GenqueryScopeTypeList => {
            AttrValue::LabelList(list(value).iter().map(label).collect())
        }
        Type::StringDict => AttrValue::StringDict(
            dict(value)
                .iter()
                .map(|(k, v)| (string(k), string(v)))
                .collect(),
        ),
        Type::StringListDict => AttrValue::StringListDict(
            dict(value)
                .iter()
                .map(|(k, v)| (string(k), list(v).iter().map(string).collect()))
                .collect(),
        ),
        Type::LabelDictUnary => AttrValue::LabelDictUnary(
            dict(value)
                .iter()
                .map(|(k, v)| (string(k), label(v)))
                .collect(),
        ),
        Type::LabelKeyedStringDict => AttrValue::LabelKeyedStringDict(
            dict(value)
                .iter()
                .map(|(k, v)| (label(k), string(v)))
                .collect(),
        ),
        Type::LabelListDict => AttrValue::LabelListDict(
            dict(value)
                .iter()
                .map(|(k, v)| (string(k), list(v).iter().map(label).collect()))
                .collect(),
        ),
        other => blaze_util::unported!(
            "RepoOutputFormatter.serializeRepoDefinitionAsProto",
            "stored repo rule attribute of type {other:?}"
        ),
    }
}
