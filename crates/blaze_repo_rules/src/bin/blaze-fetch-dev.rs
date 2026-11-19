//! Development driver for `bazel fetch --repo=...` until `blaze` (the runtime) hosts the fetch
//! command.
//!
//! Usage: `blaze-fetch-dev [--workspace DIR] [--output_base DIR] [--install_base DIR]
//!         fetch [options...] [--repo=@@name ...] [--dev_extension=<lockfile id> ...]`
//!
//! `blaze-fetch-dev [--workspace DIR] [--output_base DIR] [--install_base DIR] mod [options...]
//! <subcommand> [args...]` runs `bazel mod` (like `blaze-mod-dev`) on this evaluator, so that
//! non-registry overrides (`archive_override`, `git_override`) are really fetched.
//!
//! `--dev_extension` (not a Bazel option) also evaluates the module extension with that lockfile
//! id (e.g. `@@rules_python+//python/uv:uv.bzl%uv`), so that the lockfile can hold the same
//! extensions as one written by a whole Bazel build.
//!
//! It does what one Bazel `fetch` command does for the given repos: parse the options, set up the
//! repository machinery (`BazelRepositoryModule.beforeCommand`: download manager, repository and
//! repo contents caches, repo env, process wrapper), register the bzlmod, loading and repository
//! SkyFunctions, evaluate REPOSITORY_DIRECTORY for every `--repo` (keep_going), report failures,
//! and write the lockfile (`BazelLockFileModule.afterCommand`).
//!
//! Hook for the runtime: the `fetch` / `sync` command (`FetchCommand`) does the same with
//! `BlazeCommandDispatcher`'s options and reporter; it calls
//! [`blaze_repo_rules::registration`]'s `bazel_starlark_environment`, `register_repo_rules_functions`,
//! `inject_repo_precomputed` and `BzlmodExternalRepos` exactly as below.
// PORT: dev tool, not a Bazel class.

use std::path::PathBuf;
use std::sync::Arc;

use blaze_bzlmod::skyframe::after_command;
use blaze_cmdline::RepositoryName;
use blaze_events::{Event, EventHandler, EventKind, ExtendedEventHandler, Postable};
use blaze_optdefs::command::build_language_options::BuildLanguageOptions;
use blaze_optdefs::command::common_command_options::CommonCommandOptions;
use blaze_optdefs::command::fetch_options::FetchOptions;
use blaze_optdefs::misc::repository_options::RepositoryOptions;
use blaze_options::{OptionsClass, OptionsParser, PriorityCategory};
use blaze_repo_rules::registration::RepoPrecomputedValues;
use blaze_repo_rules::setup::{FetchSetup, create_evaluator};
use blaze_sky_loading::repos::RepositoryDirectoryKey;
use blaze_skyframe::{EvaluationContext, SkyKey};
use blaze_starlark::StarlarkSemantics;
use blaze_util::string_encoding::{
    internal_to_platform, latin1_encode_lossy, platform_to_internal,
};
use indexmap::IndexMap;

/// Prints events the way Bazel's UI does for `--color=no --curses=no`.
struct StderrHandler;

impl EventHandler for StderrHandler {
    fn handle(&self, event: &Event) {
        let kind = match event.get_kind() {
            EventKind::Error => "ERROR",
            EventKind::Warning => "WARNING",
            EventKind::Info => "INFO",
            EventKind::Debug => "DEBUG",
            _ => return,
        };
        let line = match event.get_location() {
            Some(l) => format!("{kind}: {l}: {}\n", event.get_message()),
            None => format!("{kind}: {}\n", event.get_message()),
        };
        // Internal strings leave through the Latin-1 console encoding.
        let _ = std::io::Write::write_all(&mut std::io::stderr(), &latin1_encode_lossy(&line));
    }
    fn as_extended_event_handler(&self) -> Option<&dyn ExtendedEventHandler> {
        Some(self)
    }
}

impl ExtendedEventHandler for StderrHandler {
    fn post(&self, _obj: Arc<dyn Postable>) {}
}

fn usage() -> ! {
    eprintln!(
        "usage: blaze-fetch-dev [--workspace DIR] [--output_base DIR] [--install_base DIR] \
         fetch [options...] [--repo=@@name ...]\n\
         Fetches the given repositories like `bazel fetch --repo=...` and writes \
         MODULE.bazel.lock."
    );
    std::process::exit(2)
}

fn main() {
    let code = blaze_starlark::eval::run_on_evaluation_thread(real_main);
    std::process::exit(code);
}

/// The client environment as internal strings.
fn client_env() -> IndexMap<String, String> {
    std::env::vars_os()
        .map(|(k, v)| {
            (
                platform_to_internal(&k).into_owned(),
                platform_to_internal(&v).into_owned(),
            )
        })
        .collect()
}

fn real_main() -> i32 {
    // The JVM hands Java its argv and environment as internal strings.
    let mut args: Vec<String> = std::env::args_os()
        .skip(1)
        .map(|a| platform_to_internal(&a).into_owned())
        .collect();
    if args.first().is_some_and(|a| a == "help" || a == "--help") {
        usage();
    }
    let cwd = std::env::current_dir().expect("cwd");
    let mut workspace = cwd.clone();
    let mut output_base: Option<PathBuf> = None;
    let mut install_base = PathBuf::from(std::env::var_os("HOME").unwrap_or_default())
        .join(".cache/blaze/oracle-out/ur/install/58cf1ecbff441bac762b0050d6da21e9");
    while let Some(a) = args.first().cloned() {
        let take = |args: &mut Vec<String>| -> PathBuf {
            args.remove(0);
            if args.is_empty() {
                usage();
            }
            PathBuf::from(internal_to_platform(&args.remove(0)).into_owned())
        };
        match a.as_str() {
            "--workspace" => workspace = take(&mut args),
            "--output_base" => output_base = Some(take(&mut args)),
            "--install_base" => install_base = take(&mut args),
            _ => break,
        }
    }
    let verb = args.first().cloned();
    if verb.as_deref() != Some("fetch") && verb.as_deref() != Some("mod") {
        usage();
    }
    args.remove(0);
    if verb.as_deref() == Some("mod") {
        return mod_main(args, &cwd, workspace, output_base, &install_base);
    }
    let mut dev_extensions = Vec::new();
    args.retain(|a| match a.strip_prefix("--dev_extension=") {
        Some(id) => {
            dev_extensions.push(id.to_owned());
            false
        }
        None => true,
    });
    let workspace = std::fs::canonicalize(&workspace).unwrap_or(workspace);
    let output_base = output_base.unwrap_or_else(|| workspace.join(".blaze-fetch-dev-ob"));
    std::fs::create_dir_all(&output_base).expect("create output base");
    let output_base = std::fs::canonicalize(&output_base).unwrap_or(output_base);

    let reporter: Arc<StderrHandler> = Arc::new(StderrHandler);

    let mut parser = OptionsParser::builder()
        .options_classes(&[
            FetchOptions::class(),
            RepositoryOptions::class(),
            BuildLanguageOptions::class(),
            CommonCommandOptions::class(),
        ])
        .allow_residue(true)
        .build()
        .unwrap_or_else(|e| e.throw());
    if let Err(e) = parser.parse_with_priority(PriorityCategory::COMMAND_LINE, None, args) {
        reporter.handle(&Event::error(e.get_message().to_string()));
        return 2;
    }
    let fetch_options: FetchOptions = parser.get_options().expect("FetchOptions");
    let repo_options: RepositoryOptions = parser.get_options().expect("RepositoryOptions");
    let common_options: CommonCommandOptions = parser.get_options().expect("CommonCommandOptions");

    // PORT: BuildLanguageOptions.toStarlarkSemantics is not wired yet; Bazel's defaults.
    let semantics = StarlarkSemantics::default();
    let client_env = client_env();
    let (repo_env, nonstrict_repo_env) = compute_repo_env(
        &client_env,
        &common_options,
        &platform_to_internal(workspace.as_os_str()),
    );

    let fetch = match create_evaluator(FetchSetup {
        workspace: &workspace,
        working_directory: &cwd,
        output_base: &output_base,
        install_base: &install_base,
        repo_options: &repo_options,
        starlark_semantics: semantics,
        client_env: client_env.clone(),
        repo_env,
        nonstrict_repo_env,
        precomputed: RepoPrecomputedValues {
            force_fetch: if fetch_options.force {
                uuid_string()
            } else {
                String::new()
            },
            force_fetch_configure: if fetch_options.configure {
                uuid_string()
            } else {
                String::new()
            },
            ..RepoPrecomputedValues::default()
        },
        reporter: reporter.clone(),
    }) {
        Ok(f) => f,
        Err(message) => {
            reporter.handle(&Event::error(message));
            return 2;
        }
    };
    let evaluator = &fetch.evaluator;
    let lockfile_mode = fetch.lockfile_mode;

    let mut roots = Vec::new();
    for repo in &fetch_options.repos {
        let name = repo
            .strip_prefix("@@")
            .or_else(|| repo.strip_prefix('@'))
            .unwrap_or(repo);
        roots.push(SkyKey::new(RepositoryDirectoryKey::key(
            RepositoryName::create_unvalidated(name),
        )));
    }
    for id in &dev_extensions {
        match blaze_bzlmod_model::ModuleExtensionId::parse_lockfile_form(id) {
            Ok(id) => roots.push(SkyKey::new(blaze_bzlmod::skyframe::SingleExtensionKey(id))),
            Err(e) => {
                reporter.handle(&Event::error(format!("bad --dev_extension {id}: {e}")));
                return 2;
            }
        }
    }
    let ctx = EvaluationContext {
        keep_going: true,
        event_handler: reporter.clone(),
        ..EvaluationContext::default()
    };
    let code = match evaluator.evaluate(&roots, &ctx) {
        Err(_) => {
            reporter.handle(&Event::error("fetch interrupted"));
            8
        }
        Ok(result) => {
            let mut failed = 0;
            for root in &roots {
                if let Some(e) = result.get_error(root) {
                    failed += 1;
                    let msg = e
                        .get_exception()
                        .map(|x| x.to_string())
                        .unwrap_or_else(|| format!("{e:?}"));
                    reporter.handle(&Event::error(format!("{root:?}: {msg}")));
                } else if let Some(v) = result.get_dyn(root)
                    && let Some(blaze_sky_loading::repos::RepositoryDirectoryValue::Failure {
                        error_msg,
                    }) = v.downcast_ref::<blaze_sky_loading::repos::RepositoryDirectoryValue>()
                {
                    failed += 1;
                    reporter.handle(&Event::error(format!("{root:?}: {error_msg}")));
                }
            }
            eprintln!(
                "INFO: fetched {} of {} repositories and extensions",
                roots.len() - failed,
                roots.len()
            );
            if failed > 0 { 1 } else { 0 }
        }
    };
    after_command(evaluator, &workspace, &output_base, lockfile_mode);
    code
}

/// `bazel mod` on the fetching evaluator: `ModCommand.execInternal` +
/// `BazelLockFileModule.afterCommand`, as `blaze-mod-dev` does with its local-only config.
// PORT: dev tool, not a Bazel class.
fn mod_main(
    args: Vec<String>,
    cwd: &std::path::Path,
    workspace: PathBuf,
    output_base: Option<PathBuf>,
    install_base: &std::path::Path,
) -> i32 {
    use blaze_bzlmod::modcommand::mod_command::{
        ModCommandResult, exec_after_evaluation, exec_before_evaluation, report_evaluation_error,
    };
    use blaze_bzlmod::skyframe::{BazelDepGraphKey, BazelModuleInspectorKey};
    use blaze_optdefs::command::loading_phase_threads_option::LoadingPhaseThreadsOption;
    use blaze_optdefs::command::ui_options::UiOptions;
    use blaze_optdefs::misc::mod_options::ModOptions;
    use std::io::Write;

    fn exit_code(r: &ModCommandResult) -> i32 {
        match r {
            ModCommandResult::Success => 0,
            ModCommandResult::Failure { exit_code, .. } => *exit_code,
            // ExitCode.INTERRUPTED
            ModCommandResult::Interrupted { .. } => 8,
        }
    }

    let workspace = std::fs::canonicalize(&workspace).unwrap_or(workspace);
    let output_base = output_base.unwrap_or_else(|| workspace.join(".blaze-fetch-dev-ob"));
    std::fs::create_dir_all(&output_base).expect("create output base");
    let output_base = std::fs::canonicalize(&output_base).unwrap_or(output_base);
    let reporter: Arc<StderrHandler> = Arc::new(StderrHandler);

    let mut parser = OptionsParser::builder()
        .options_classes(&[
            ModOptions::class(),
            RepositoryOptions::class(),
            BuildLanguageOptions::class(),
            CommonCommandOptions::class(),
            LoadingPhaseThreadsOption::class(),
            UiOptions::class(),
        ])
        .allow_residue(true)
        .build()
        .unwrap_or_else(|e| e.throw());
    if let Err(e) = parser.parse_with_priority(PriorityCategory::COMMAND_LINE, None, args) {
        reporter.handle(&Event::error(e.get_message().to_string()));
        return 2;
    }
    let mod_options: ModOptions = parser.get_options().expect("ModOptions");
    let repo_options: RepositoryOptions = parser.get_options().expect("RepositoryOptions");
    let common_options: CommonCommandOptions = parser.get_options().expect("CommonCommandOptions");
    let residue = parser.get_residue();
    let request = match exec_before_evaluation(&residue, &mod_options, reporter.as_ref()) {
        Ok(r) => r,
        Err(r) => return exit_code(&r),
    };
    if matches!(
        request.subcommand,
        blaze_optdefs::misc::mod_options::ModSubcommand::DUMP_REPO_MAPPING
            | blaze_optdefs::misc::mod_options::ModSubcommand::TIDY
            | blaze_optdefs::misc::mod_options::ModSubcommand::SHOW_REPO
    ) {
        // These subcommands are wired only in `blaze mod` (blaze_runtime).
        blaze_util::unported!(
            "ModCommand.execInternal",
            "{:?} in blaze-fetch-dev (use `blaze mod`)",
            request.subcommand
        );
    }

    // PORT: BuildLanguageOptions.toStarlarkSemantics is not wired yet; Bazel's defaults.
    let semantics = StarlarkSemantics::default();
    let client_env = client_env();
    let (repo_env, nonstrict_repo_env) = compute_repo_env(
        &client_env,
        &common_options,
        &platform_to_internal(workspace.as_os_str()),
    );
    let fetch = match create_evaluator(FetchSetup {
        workspace: &workspace,
        working_directory: cwd,
        output_base: &output_base,
        install_base,
        repo_options: &repo_options,
        starlark_semantics: semantics,
        client_env: client_env.clone(),
        repo_env,
        nonstrict_repo_env,
        precomputed: RepoPrecomputedValues::default(),
        reporter: reporter.clone(),
    }) {
        Ok(f) => f,
        Err(message) => {
            reporter.handle(&Event::error(message));
            return 2;
        }
    };
    let evaluator = &fetch.evaluator;
    let ctx = EvaluationContext {
        keep_going: true,
        event_handler: reporter.clone(),
        ..EvaluationContext::default()
    };
    let roots = [
        SkyKey::new(BazelDepGraphKey),
        SkyKey::new(BazelModuleInspectorKey),
    ];
    let code = match evaluator.evaluate(&roots, &ctx) {
        Err(_) => {
            reporter.handle(&Event::error("mod command interrupted: null"));
            8
        }
        Ok(result) if result.has_error() => {
            let message = result
                .get_first_error()
                .and_then(|e| e.get_exception())
                .map(|e| e.to_string());
            exit_code(&report_evaluation_error(
                reporter.as_ref(),
                message.as_deref(),
            ))
        }
        Ok(result) => {
            let dep_graph = result.get(&BazelDepGraphKey).expect("dep graph");
            let inspector = result.get(&BazelModuleInspectorKey).expect("inspector");
            let stdout = std::io::stdout();
            let mut out = stdout.lock();
            let r = exec_after_evaluation(
                request.subcommand,
                &request.args,
                &mod_options,
                &dep_graph,
                &inspector,
                reporter.as_ref(),
                &mut out,
                None,
            );
            out.flush().ok();
            exit_code(&r)
        }
    };
    after_command(evaluator, &workspace, &output_base, fetch.lockfile_mode);
    code
}

fn uuid_string() -> String {
    uuid::Uuid::new_v4().to_string()
}

/// Java `CommandEnvironment`'s repo env: the client env (only `PATH` with
/// `--experimental_strict_repo_env`) plus `--repo_env`.
// Java: com/google/devtools/build/lib/runtime/CommandEnvironment.java:342 CommandEnvironment.<init>
fn compute_repo_env(
    client_env: &IndexMap<String, String>,
    options: &CommonCommandOptions,
    bazel_workspace: &str,
) -> (IndexMap<String, String>, IndexMap<String, String>) {
    use blaze_options::converters::EnvVar;
    let mut repo_env: std::collections::BTreeMap<String, String> = if options.use_strict_repo_env {
        client_env
            .iter()
            .filter(|(k, _)| k.as_str() == "PATH")
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    } else {
        client_env
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect()
    };
    let mut nonstrict: std::collections::BTreeMap<String, String> = client_env
        .iter()
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    // PORT: --action_env (CoreOptions) is not parsed by this dev driver; its default is empty.
    for env_var in &options.repository_environment {
        match env_var {
            EnvVar::Set { name, value } => {
                let value = value.replace("%bazel_workspace%", bazel_workspace);
                repo_env.insert(name.clone(), value.clone());
                nonstrict.insert(name.clone(), value);
            }
            EnvVar::Inherit { name } => {
                if let Some(value) = client_env.get(name) {
                    repo_env.insert(name.clone(), value.clone());
                    nonstrict.insert(name.clone(), value.clone());
                }
            }
            EnvVar::Unset { name } => {
                repo_env.remove(name);
                nonstrict.remove(name);
            }
        }
    }
    (
        repo_env.into_iter().collect(),
        nonstrict.into_iter().collect(),
    )
}
