//! The Skyframe-independent parts of `com.google.devtools.build.lib.bazel.commands.ModCommand`.
//! OWNER: agent B. The caller (a `BlazeCommand` or the dev binary) parses options, calls
//! [`exec_before_evaluation`] (residue parsing and argument validation), evaluates
//! `BazelDepGraphValue.KEY` and `BazelModuleInspectorValue.KEY` with keep_going (reporting a
//! failed evaluation with [`report_evaluation_error`]), and then calls [`exec_after_evaluation`].

use std::borrow::Cow;
use std::io::Write;

use blaze_bzlmod_model::{BazelDepGraphValue, ModuleExtensionId, ModuleKey};
use blaze_cmdline::{RepositoryMapping, RepositoryName};
use blaze_events::{Event, ExtendedEventHandler};
use blaze_optdefs::misc::mod_options::{
    ModOptions, ModSubcommand, ModSubcommandConverter, OutputFormat,
};
use blaze_options::{Converter, OptionScalar};
use indexmap::{IndexMap, IndexSet};

use super::extension_arg::{ExtensionArg, extension_arg_list_to_string};
use super::invalid_argument_exception::{InvalidArgumentException, ModCommandCode};
use super::maybe_complete_set::MaybeCompleteSet;
use super::mod_executor::ModExecutor;
use super::module_arg::{ModuleArg, module_arg_list_to_string};
use crate::inspector::BazelModuleInspectorValue;

/// `env.getRuntime().getProductName()`: the `--product_name` startup option lowercased
/// (`BlazeRuntime.newRuntime`); the Bazel client passes `Bazel`.
pub const PRODUCT_NAME: &str = "bazel";

/// What a mod subcommand returns (`BlazeCommandResult`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ModCommandResult {
    Success,
    /// `reportAndCreateFailureResult`: the message was already reported as an ERROR event.
    Failure {
        message: String,
        /// `FailureDetails.ModCommand.Code` name, e.g. "INVALID_ARGUMENTS".
        code: &'static str,
        /// The process exit code Bazel uses for this failure.
        exit_code: i32,
    },
}

/// `ModCommand.validateArgs`; the error is `(message, code name)`.
// Java: com/google/devtools/build/lib/bazel/commands/ModCommand.java:141 ModCommand.validateArgs
pub fn validate_args(
    subcommand: ModSubcommand,
    mod_options: &ModOptions,
    args: &[String],
) -> Result<(), (String, &'static str)> {
    validate_args_internal(subcommand, mod_options, args)
        .map_err(|e| (e.get_message().to_string(), e.get_code().name()))
}

/// `ModCommand.validateArgs` with the Java exception type.
// Java: com/google/devtools/build/lib/bazel/commands/ModCommand.java:141 ModCommand.validateArgs
pub fn validate_args_internal(
    subcommand: ModSubcommand,
    mod_options: &ModOptions,
    args: &[String],
) -> Result<(), InvalidArgumentException> {
    // Validate output format.
    match subcommand {
        ModSubcommand::SHOW_REPO => match mod_options.output_format {
            OutputFormat::TEXT
            | OutputFormat::STREAMED_JSONPROTO
            | OutputFormat::STREAMED_PROTO => {} // supported
            _ => {
                return Err(InvalidArgumentException::with_code(
                    format!(
                        "Invalid --output '{}' for the 'show_repo' subcommand. Only 'text', \
                         'streamed_jsonproto', and 'streamed_proto' are supported.",
                        mod_options.output_format.java_to_string()
                    ),
                    ModCommandCode::INVALID_ARGUMENTS,
                ));
            }
        },
        ModSubcommand::SHOW_EXTENSION => {
            if mod_options.output_format != OutputFormat::TEXT {
                return Err(InvalidArgumentException::with_code(
                    format!(
                        "Invalid --output '{}' for the 'show_extension' subcommand. Only 'text' \
                         is supported.",
                        mod_options.output_format.java_to_string()
                    ),
                    ModCommandCode::INVALID_ARGUMENTS,
                ));
            }
        }
        sub if sub.is_graph() => match mod_options.output_format {
            OutputFormat::TEXT | OutputFormat::JSON | OutputFormat::GRAPH => {} // supported
            _ => {
                return Err(InvalidArgumentException::with_code(
                    format!(
                        "Invalid --output '{}' for the '{}' subcommand. Only 'text', 'json', and \
                         'graph' are supported.",
                        mod_options.output_format.java_to_string(),
                        sub.java_to_string()
                    ),
                    ModCommandCode::INVALID_ARGUMENTS,
                ));
            }
        },
        // We don't validate other subcommands yet since they are less confusing.
        _ => {}
    }

    if subcommand == ModSubcommand::SHOW_REPO {
        let mut selected_modes = 0;
        if mod_options.all_repos {
            selected_modes += 1;
        }
        if mod_options.all_visible_repos {
            selected_modes += 1;
        }
        if !args.is_empty() {
            selected_modes += 1;
        }
        if selected_modes > 1 {
            return Err(InvalidArgumentException::with_code(
                "the 'show_repo' command requires exactly one of --all_repos, --all_visible_repos, \
                 or a list of repo arguments",
                ModCommandCode::TOO_MANY_ARGUMENTS,
            ));
        }
    } else {
        if mod_options.all_repos {
            return Err(InvalidArgumentException::with_code(
                format!(
                    "the '{}' command doesn't take the --all_repos option",
                    subcommand.java_to_string()
                ),
                ModCommandCode::INVALID_ARGUMENTS,
            ));
        }
        if mod_options.all_visible_repos {
            return Err(InvalidArgumentException::with_code(
                format!(
                    "the '{}' command doesn't take the --all_visible_repos option",
                    subcommand.java_to_string()
                ),
                ModCommandCode::INVALID_ARGUMENTS,
            ));
        }
    }
    Ok(())
}

/// `ModCommand.reportAndCreateFailureResult`: reports the (possibly extended) message as an
/// ERROR event and returns the failure.
// Java: com/google/devtools/build/lib/bazel/commands/ModCommand.java:799 ModCommand.reportAndCreateFailureResult
pub fn report_and_create_failure_result(
    reporter: &dyn ExtendedEventHandler,
    message: &str,
    detailed_code: ModCommandCode,
) -> ModCommandResult {
    let full_message = match detailed_code {
        ModCommandCode::MISSING_ARGUMENTS
        | ModCommandCode::TOO_MANY_ARGUMENTS
        | ModCommandCode::INVALID_ARGUMENTS => format!(
            "{}{} Type '{} help mod' for syntax and help.",
            message,
            if message.ends_with('.') { "" } else { "." },
            PRODUCT_NAME
        ),
        _ => message.to_string(),
    };
    reporter.handle(&Event::error(full_message.clone()));
    create_failure_result(full_message, detailed_code)
}

// Java: com/google/devtools/build/lib/bazel/commands/ModCommand.java:813 ModCommand.createFailureResult
fn create_failure_result(message: String, detailed_code: ModCommandCode) -> ModCommandResult {
    ModCommandResult::Failure {
        message,
        code: detailed_code.name(),
        exit_code: detailed_code.exit_code(),
    }
}

/// What [`exec_before_evaluation`] hands to the Skyframe evaluation.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ModCommandRequest {
    pub subcommand: ModSubcommand,
    /// The residue after the subcommand.
    pub args: Vec<String>,
    /// For `dump_repo_mapping`: the repositories whose `RepositoryMappingValue`s to evaluate.
    pub repo_mapping_repo_names: Vec<RepositoryName>,
}

/// The part of `ModCommand.execInternal` before the Skyframe evaluation: subcommand parsing,
/// `validateArgs` and the `dump_repo_mapping` argument checks.
// Java: com/google/devtools/build/lib/bazel/commands/ModCommand.java:216 ModCommand.execInternal
pub fn exec_before_evaluation(
    residue: &[String],
    mod_options: &ModOptions,
    reporter: &dyn ExtendedEventHandler,
) -> Result<ModCommandRequest, ModCommandResult> {
    if residue.is_empty() {
        let error_message = format!(
            "No subcommand specified, choose one of : {}.",
            ModSubcommand::print_values()
        );
        return Err(report_and_create_failure_result(
            reporter,
            &error_message,
            ModCommandCode::MOD_COMMAND_UNKNOWN,
        ));
    }

    // The first element in the residue must be the subcommand, and then comes a list of
    // arguments.
    let subcommand_str = &residue[0];
    let subcommand = match ModSubcommandConverter.convert(Some(subcommand_str.as_str()), None) {
        Ok(Some(v)) => v.get::<ModSubcommand>(),
        Ok(None) | Err(_) => {
            let error_message = format!(
                "Invalid subcommand, choose one from : {}.",
                ModSubcommand::print_values()
            );
            return Err(report_and_create_failure_result(
                reporter,
                &error_message,
                ModCommandCode::MOD_COMMAND_UNKNOWN,
            ));
        }
    };
    let args: Vec<String> = residue[1..].to_vec();

    // Validate and parse args as early as possible, so we don't have to
    // wait for Skyframe evaluations to happen before failing due to a simple error.
    if let Err(e) = validate_args_internal(subcommand, mod_options, &args) {
        return Err(report_and_create_failure_result(
            reporter,
            e.get_message(),
            e.get_code(),
        ));
    }

    let mut repo_mapping_repo_names = Vec::new();
    if subcommand == ModSubcommand::DUMP_REPO_MAPPING {
        if args.is_empty() {
            // Make this case an error so that we are free to add a mode that emits all mappings
            // in a single JSON object later.
            return Err(report_and_create_failure_result(
                reporter,
                "No repository name(s) specified",
                ModCommandCode::INVALID_ARGUMENTS,
            ));
        }
        for arg in &args {
            match RepositoryName::create(arg) {
                Ok(r) => repo_mapping_repo_names.push(r),
                Err(e) => {
                    return Err(report_and_create_failure_result(
                        reporter,
                        e.get_message(),
                        ModCommandCode::INVALID_ARGUMENTS,
                    ));
                }
            }
        }
    }
    Ok(ModCommandRequest {
        subcommand,
        args,
        repo_mapping_repo_names,
    })
}

/// The `evaluationResult.hasError()` branch of `ModCommand.execInternal` (after any cycles were
/// reported): `message` is the error's exception message, if there is one.
// Java: com/google/devtools/build/lib/bazel/commands/ModCommand.java:296 ModCommand.execInternal
pub fn report_evaluation_error(
    reporter: &dyn ExtendedEventHandler,
    message: Option<&str>,
) -> ModCommandResult {
    let message = message.unwrap_or("Unexpected error during module graph evaluation.");
    report_and_create_failure_result(reporter, message, ModCommandCode::MOD_COMMAND_UNKNOWN)
}

/// The part of `ModCommand.execInternal` after the Skyframe evaluation, from "Extract and check
/// the --base_module argument" on: resolves args, runs `ModExecutor`, prints to `out` (the
/// reporter's output stream).
// Java: com/google/devtools/build/lib/bazel/commands/ModCommand.java:370 ModCommand.execInternal
pub fn exec_after_evaluation(
    subcommand: ModSubcommand,
    args: &[String],
    mod_options: &ModOptions,
    dep_graph_value: &BazelDepGraphValue,
    module_inspector: &BazelModuleInspectorValue,
    reporter: &dyn ExtendedEventHandler,
    out: &mut dyn Write,
) -> ModCommandResult {
    // Handle commands that do not require BazelModuleInspectorValue.
    match subcommand {
        ModSubcommand::DUMP_REPO_MAPPING => {
            blaze_util::unported!(
                "ModCommand.execInternal (dump_repo_mapping: RepositoryMappingValue)"
            )
        }
        ModSubcommand::TIDY => {
            blaze_util::unported!("ModCommand.runTidy")
        }
        _ => {}
    }
    let mut mod_options = mod_options.clone();

    // Extract and check the --base_module argument first to use it when parsing the other args.
    // Can only be a TargetModule or a repoName relative to the ROOT.
    let base_module_arg = ModuleArg::from_option(&mod_options.base_module);
    let Some(root_module) = module_inspector.dep_graph.get(&ModuleKey::root()) else {
        panic!("java.lang.NullPointerException");
    };
    let base_module_key: ModuleKey = match (|| -> Result<ModuleKey, InvalidArgumentException> {
        let keys = base_module_arg.resolve_to_module_keys(
            &module_inspector.modules_index,
            &module_inspector.dep_graph,
            &module_inspector.module_key_to_canonical_names,
            &root_module.deps,
            &root_module.unused_deps,
            false,
            false,
        )?;
        if keys.len() > 1 {
            return Err(InvalidArgumentException::with_code(
                format!(
                    "The --base_module option can only specify exactly one module version, \
                     choose one of: {}.",
                    keys.iter()
                        .map(|k| k.to_string())
                        .collect::<Vec<_>>()
                        .join(", ")
                ),
                ModCommandCode::INVALID_ARGUMENTS,
            ));
        }
        match keys.len() {
            1 => Ok(keys.into_iter().next().unwrap()),
            _ => panic!("java.util.NoSuchElementException"),
        }
    })() {
        Ok(k) => k,
        Err(e) => {
            return report_and_create_failure_result(
                reporter,
                &format!(
                    "In --base_module {} option: {} (Note that unused modules cannot be used here)",
                    base_module_arg,
                    e.get_message()
                ),
                ModCommandCode::INVALID_ARGUMENTS,
            );
        }
    };

    // The args can have different types depending on the subcommand, so create multiple
    // containers which can be filled accordingly.
    let mut args_as_modules: Option<IndexSet<ModuleKey>> = None;
    let mut args_as_extensions: Option<Vec<ModuleExtensionId>> = None;
    let mut _args_as_repos: Option<IndexMap<String, RepositoryName>> = None;

    let Some(base_module) = module_inspector.dep_graph.get(&base_module_key) else {
        panic!("java.lang.NullPointerException");
    };
    let base_module_mapping = dep_graph_value.get_full_repo_mapping(&base_module_key);
    let parsed: Result<(), InvalidArgumentException> = (|| {
        match subcommand {
            ModSubcommand::GRAPH => {
                // GRAPH doesn't take extra arguments.
                if !args.is_empty() {
                    return Err(InvalidArgumentException::with_code(
                        "the 'graph' command doesn't take extra arguments",
                        ModCommandCode::TOO_MANY_ARGUMENTS,
                    ));
                }
            }
            ModSubcommand::SHOW_REPO => {
                _args_as_repos = Some(get_repos_to_show(
                    &mod_options,
                    module_inspector,
                    dep_graph_value,
                    &base_module_mapping,
                    args,
                )?);
            }
            ModSubcommand::SHOW_EXTENSION => {
                let mut extensions: Vec<ModuleExtensionId> = Vec::new();
                for arg in args {
                    let resolved = match ExtensionArg::convert(arg) {
                        Err(e) => Err(e.get_message().to_string()),
                        Ok(ext) => ext
                            .resolve_to_extension_id(
                                &module_inspector.modules_index,
                                &module_inspector.dep_graph,
                                &module_inspector.module_key_to_canonical_names,
                                &base_module.deps,
                                &base_module.unused_deps,
                            )
                            .map_err(|e| e.get_message().to_string()),
                    };
                    match resolved {
                        Ok(id) => add_to_sorted_set(&mut extensions, id),
                        Err(message) => {
                            return Err(InvalidArgumentException::with_cause(
                                format!("In extension argument {arg}: {message}"),
                                ModCommandCode::INVALID_ARGUMENTS,
                                message,
                            ));
                        }
                    }
                }
                args_as_extensions = Some(extensions);
            }
            _ => {
                let mut keys_builder: IndexSet<ModuleKey> = IndexSet::new();
                for arg in args {
                    let resolved = match ModuleArg::convert(arg) {
                        Err(e) => Err(e.get_message().to_string()),
                        Ok(module_arg) => module_arg
                            .resolve_to_module_keys(
                                &module_inspector.modules_index,
                                &module_inspector.dep_graph,
                                &module_inspector.module_key_to_canonical_names,
                                &base_module.deps,
                                &base_module.unused_deps,
                                mod_options.include_unused,
                                /* warn_unused= */ true,
                            )
                            .map_err(|e| e.get_message().to_string()),
                    };
                    match resolved {
                        Ok(keys) => keys_builder.extend(keys),
                        Err(message) => {
                            return Err(InvalidArgumentException::with_code(
                                format!("In module argument {arg}: {message}"),
                                ModCommandCode::INVALID_ARGUMENTS,
                            ));
                        }
                    }
                }
                args_as_modules = Some(keys_builder);
            }
        }
        Ok(())
    })();
    if let Err(e) = parsed {
        return report_and_create_failure_result(reporter, e.get_message(), e.get_code());
    }

    // Extract and check the --from and --extension_usages argument
    let modules_from: Vec<ModuleArg> = mod_options
        .modules_from
        .iter()
        .map(ModuleArg::from_option)
        .collect();
    let from_keys = match module_arg_list_to_keys(
        &modules_from,
        module_inspector,
        &base_module.deps,
        &base_module.unused_deps,
        mod_options.include_unused,
    ) {
        Ok(k) => k,
        Err(e) => {
            return report_and_create_failure_result(
                reporter,
                &format!(
                    "In --from {} option: {}",
                    module_arg_list_to_string(&modules_from),
                    e.get_message()
                ),
                ModCommandCode::INVALID_ARGUMENTS,
            );
        }
    };

    let extension_usages: Vec<ModuleArg> = mod_options
        .extension_usages
        .iter()
        .map(ModuleArg::from_option)
        .collect();
    let usage_keys = match module_arg_list_to_keys(
        &extension_usages,
        module_inspector,
        &base_module.deps,
        &base_module.unused_deps,
        mod_options.include_unused,
    ) {
        Ok(k) => k,
        Err(e) => {
            return report_and_create_failure_result(
                reporter,
                &format!(
                    "In --extension_usages {} option: {} (Note that unused modules cannot be used \
                     here)",
                    module_arg_list_to_string(&extension_usages),
                    e.get_message()
                ),
                ModCommandCode::INVALID_ARGUMENTS,
            );
        }
    };

    // Extract and check the --extension_filter argument
    let mut filter_extensions: Option<MaybeCompleteSet<ModuleExtensionId>> = None;
    if subcommand.is_graph()
        && let Some(extension_filter) = &mod_options.extension_filter
    {
        if extension_filter.is_empty() {
            filter_extensions = Some(MaybeCompleteSet::complete_set());
        } else {
            let extension_filter: Vec<ExtensionArg> = extension_filter
                .iter()
                .map(ExtensionArg::from_option)
                .collect();
            match extension_arg_list_to_ids(
                &extension_filter,
                module_inspector,
                &base_module.deps,
                &base_module.unused_deps,
            ) {
                Ok(ids) => filter_extensions = Some(MaybeCompleteSet::copy_of(&ids)),
                Err(e) => {
                    return report_and_create_failure_result(
                        reporter,
                        &format!(
                            "In --extension_filter {} option: {}",
                            extension_arg_list_to_string(&extension_filter),
                            e.get_message()
                        ),
                        ModCommandCode::INVALID_ARGUMENTS,
                    );
                }
            }
        }
    }

    if subcommand == ModSubcommand::SHOW_REPO {
        blaze_util::unported!("ModCommand.execInternal (show_repo: RepoDefinitionValue)");
    }

    // Workaround to allow different default value for DEPS and EXPLAIN, and also use
    // Integer.MAX_VALUE instead of the exact number string.
    if mod_options.depth < 1 {
        mod_options.depth = match subcommand {
            ModSubcommand::EXPLAIN => 1,
            ModSubcommand::DEPS => 2,
            _ => i32::MAX,
        };
    }

    let mut mod_executor = ModExecutor::with_extensions(
        Cow::Borrowed(&module_inspector.dep_graph),
        Cow::Borrowed(&dep_graph_value.extension_usages_table),
        Cow::Borrowed(&module_inspector.extension_to_repo_internal_names),
        filter_extensions,
        mod_options,
        out,
    );

    let executed: Result<(), InvalidArgumentException> = match subcommand {
        ModSubcommand::GRAPH => {
            mod_executor.graph(&from_keys);
            Ok(())
        }
        ModSubcommand::DEPS => {
            mod_executor.graph(args_as_modules.as_ref().unwrap());
            Ok(())
        }
        ModSubcommand::PATH => {
            mod_executor.path(&from_keys, args_as_modules.as_ref().unwrap());
            Ok(())
        }
        ModSubcommand::ALL_PATHS | ModSubcommand::EXPLAIN => {
            mod_executor.all_paths(&from_keys, args_as_modules.as_ref().unwrap());
            Ok(())
        }
        ModSubcommand::SHOW_REPO => {
            mod_executor.show_repo();
            Ok(())
        }
        ModSubcommand::SHOW_EXTENSION => {
            mod_executor.show_extension(args_as_extensions.as_deref().unwrap(), &usage_keys)
        }
        other => panic!(
            "IllegalStateException: Unexpected subcommand: {}",
            other.java_to_string()
        ),
    };
    if let Err(e) = executed {
        return report_and_create_failure_result(
            reporter,
            e.get_message(),
            ModCommandCode::INVALID_ARGUMENTS,
        );
    }

    if module_inspector.errors.is_empty() {
        ModCommandResult::Success
    } else {
        let n = module_inspector.errors.len();
        report_and_create_failure_result(
            reporter,
            &format!(
                "Results may be incomplete as {} extension{} failed.",
                n,
                if n == 1 { "" } else { "s" }
            ),
            ModCommandCode::ERROR_DURING_GRAPH_INSPECTION,
        )
    }
}

/// `ImmutableSortedSet.Builder<ModuleExtensionId>(LEXICOGRAPHIC_COMPARATOR).add(id)`: keeps the
/// first of equal elements.
fn add_to_sorted_set(set: &mut Vec<ModuleExtensionId>, id: ModuleExtensionId) {
    match set.binary_search_by(|e| ModuleExtensionId::lexicographic_cmp(e, &id)) {
        Ok(_) => {}
        Err(pos) => set.insert(pos, id),
    }
}

// Java: com/google/devtools/build/lib/bazel/commands/ModCommand.java:625 ModCommand.getReposToShow
fn get_repos_to_show(
    mod_options: &ModOptions,
    module_inspector: &BazelModuleInspectorValue,
    dep_graph_value: &BazelDepGraphValue,
    base_module_mapping: &RepositoryMapping,
    args: &[String],
) -> Result<IndexMap<String, RepositoryName>, InvalidArgumentException> {
    // ImmutableMap.Builder.buildKeepingLast: the last value wins, at the key's first position.
    let mut target_to_repo_name: IndexMap<String, RepositoryName> = IndexMap::new();

    if mod_options.all_repos {
        // Module repos.
        for repo_name in module_inspector.module_key_to_canonical_names.values() {
            if repo_name.is_main() {
                // The main repo can't be inspected.
                continue;
            }
            target_to_repo_name.insert(repo_name.get_name_with_at(), repo_name.clone());
        }

        // Extension repos.
        for (extension_id, internal_names) in &module_inspector.extension_to_repo_internal_names {
            // Java concatenates a missing (null) unique name as "null".
            let extension_unique_name: &str = dep_graph_value
                .extension_unique_names
                .get(extension_id)
                .map(String::as_str)
                .unwrap_or("null");

            for internal_name in internal_names {
                // Java: com/google/devtools/build/lib/bazel/bzlmod/SingleExtensionValue.java:72 SingleExtensionValue.repositoryName
                let repo_name = RepositoryName::create_unvalidated(&format!(
                    "{extension_unique_name}+{internal_name}"
                ));
                target_to_repo_name.insert(repo_name.get_name_with_at(), repo_name);
            }
        }
    } else if mod_options.all_visible_repos {
        for (key, value) in base_module_mapping.entries() {
            if value.is_main() {
                // The main repo can't be inspected.
                continue;
            }
            target_to_repo_name.insert(format!("@{key}"), value.clone());
        }
    } else {
        // Resolve explicitly specified repos.
        for arg in args {
            let resolved = match ModuleArg::convert(arg) {
                Err(e) => Err(e.get_message().to_string()),
                Ok(module_arg) => module_arg
                    .resolve_to_repo_names(
                        &module_inspector.modules_index,
                        &module_inspector.dep_graph,
                        &module_inspector.module_key_to_canonical_names,
                        base_module_mapping,
                    )
                    .map_err(|e| e.get_message().to_string()),
            };
            match resolved {
                Ok(m) => target_to_repo_name.extend(m),
                Err(message) => {
                    return Err(InvalidArgumentException::with_cause(
                        format!(
                            "In repo argument {arg}: {message} (Note that unused modules cannot be \
                             used here)"
                        ),
                        ModCommandCode::INVALID_ARGUMENTS,
                        message,
                    ));
                }
            }
        }
    }
    Ok(target_to_repo_name)
}

/// Collects a list of [`ModuleArg`] into a set of `ModuleKey`s.
// Java: com/google/devtools/build/lib/bazel/commands/ModCommand.java:753 ModCommand.moduleArgListToKeys
fn module_arg_list_to_keys(
    arg_list: &[ModuleArg],
    module_inspector: &BazelModuleInspectorValue,
    base_module_deps: &IndexMap<String, ModuleKey>,
    base_module_unused_deps: &IndexMap<String, ModuleKey>,
    include_unused: bool,
) -> Result<IndexSet<ModuleKey>, InvalidArgumentException> {
    let mut all_target_keys: IndexSet<ModuleKey> = IndexSet::new();
    for module_arg in arg_list {
        all_target_keys.extend(module_arg.resolve_to_module_keys(
            &module_inspector.modules_index,
            &module_inspector.dep_graph,
            &module_inspector.module_key_to_canonical_names,
            base_module_deps,
            base_module_unused_deps,
            include_unused,
            true,
        )?);
    }
    Ok(all_target_keys)
}

// Java: com/google/devtools/build/lib/bazel/commands/ModCommand.java:777 ModCommand.extensionArgListToIds
fn extension_arg_list_to_ids(
    args: &[ExtensionArg],
    module_inspector: &BazelModuleInspectorValue,
    base_module_deps: &IndexMap<String, ModuleKey>,
    base_module_unused_deps: &IndexMap<String, ModuleKey>,
) -> Result<Vec<ModuleExtensionId>, InvalidArgumentException> {
    let mut extensions_builder: Vec<ModuleExtensionId> = Vec::new();
    for arg in args {
        let id = arg.resolve_to_extension_id(
            &module_inspector.modules_index,
            &module_inspector.dep_graph,
            &module_inspector.module_key_to_canonical_names,
            base_module_deps,
            base_module_unused_deps,
        )?;
        add_to_sorted_set(&mut extensions_builder, id);
    }
    Ok(extensions_builder)
}
