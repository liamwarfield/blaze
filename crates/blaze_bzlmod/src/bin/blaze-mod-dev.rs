//! Development driver for `bazel mod` until `blaze` (W1) hosts the command.
//!
//! Usage: `blaze-mod-dev [--workspace DIR] [--output_base DIR] [--install_base DIR]
//!         mod [options...] <subcommand> [args...]`
//!
//! It does what `ModCommand.execInternal` + `BazelLockFileModule.afterCommand` do in one Bazel
//! command: parse the options, register the bzlmod SkyFunctions, evaluate `BAZEL_DEP_GRAPH` and
//! `BAZEL_MODULE_INSPECTION` with keep_going, run the subcommand, write the lockfile.
// PORT: dev tool, not a Bazel class. The real wiring is BlazeCommandDispatcher -> ModCommand.

use std::io::Write;
use std::path::PathBuf;
use std::sync::Arc;

use blaze_bzlmod::modcommand::mod_command::{
    ModCommandResult, exec_after_evaluation, exec_before_evaluation, report_evaluation_error,
};
use blaze_bzlmod::registry_fetcher::LocalDownloadManagerFetcher;
use blaze_bzlmod::skyframe::{
    BazelDepGraphKey, BazelModuleInspectorKey, BzlmodConfig, after_command,
    register_bzlmod_functions, register_lockfile_only_extension_functions,
};
use blaze_events::{Event, EventHandler, EventKind, ExtendedEventHandler, Postable};
use blaze_optdefs::command::build_language_options::BuildLanguageOptions;
use blaze_optdefs::command::loading_phase_threads_option::LoadingPhaseThreadsOption;
use blaze_optdefs::command::ui_options::UiOptions;
use blaze_optdefs::misc::mod_options::ModOptions;
use blaze_optdefs::misc::repository_options::RepositoryOptions;
use blaze_options::{OptionsClass, OptionsParser, PriorityCategory};
use blaze_skyframe::{EvaluationContext, EvaluatorBuilder, SkyKey};
use blaze_starlark::StarlarkSemantics;
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
        match event.get_location() {
            Some(l) => eprintln!("{kind}: {l}: {}", event.get_message()),
            None => eprintln!("{kind}: {}", event.get_message()),
        }
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
        "usage: blaze-mod-dev [--workspace DIR] [--output_base DIR] [--install_base DIR] \
         mod [options...] <subcommand> [args...]"
    );
    std::process::exit(2)
}

fn main() {
    let code = blaze_starlark::eval::run_on_evaluation_thread(real_main);
    std::process::exit(code);
}

fn real_main() -> i32 {
    let mut args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().is_some_and(|a| a == "help" || a == "--help") {
        usage();
    }
    let cwd = std::env::current_dir().expect("cwd");
    let mut workspace = cwd.clone();
    let mut output_base: Option<PathBuf> = None;
    let mut install_base = PathBuf::from(format!(
        "{}/.cache/blaze/oracle-out/ur/install/58cf1ecbff441bac762b0050d6da21e9",
        std::env::var("HOME").unwrap_or_default()
    ));
    while let Some(a) = args.first().cloned() {
        let take = |args: &mut Vec<String>| -> PathBuf {
            args.remove(0);
            if args.is_empty() {
                usage();
            }
            PathBuf::from(args.remove(0))
        };
        match a.as_str() {
            "--workspace" => workspace = take(&mut args),
            "--output_base" => output_base = Some(take(&mut args)),
            "--install_base" => install_base = take(&mut args),
            _ => break,
        }
    }
    if args.first().map(String::as_str) != Some("mod") {
        usage();
    }
    args.remove(0);
    let workspace = std::fs::canonicalize(&workspace).unwrap_or(workspace);
    let output_base = output_base.unwrap_or_else(|| workspace.join(".blaze-mod-dev-ob"));
    std::fs::create_dir_all(&output_base).expect("create output base");

    let reporter: Arc<StderrHandler> = Arc::new(StderrHandler);

    // Options of the mod command (ModCommand's @Command options and the common ones it inherits
    // that the bzlmod functions read).
    let mut parser = OptionsParser::builder()
        .options_classes(&[
            ModOptions::class(),
            RepositoryOptions::class(),
            BuildLanguageOptions::class(),
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
    let residue = parser.get_residue();

    let request = match exec_before_evaluation(&residue, &mod_options, reporter.as_ref()) {
        Ok(r) => r,
        Err(r) => return exit_code(&r),
    };

    // PORT: BuildLanguageOptions.toStarlarkSemantics is not wired yet; Bazel's defaults.
    let semantics = StarlarkSemantics::default();
    let client_env: IndexMap<String, String> = std::env::vars().collect();
    let repo_cache = match &repo_options.repository_cache {
        Some(p) if p.get_path_string().is_empty() => None,
        Some(p) => Some(PathBuf::from(p.get_path_string())),
        // Java: RepositoryOptions default `{--output_user_root}/cache/repos/v1`.
        None => output_base.parent().map(|p| p.join("cache/repos/v1")),
    };
    let fetcher = Arc::new(LocalDownloadManagerFetcher::new(
        repo_cache,
        reporter.clone() as Arc<dyn ExtendedEventHandler>,
        repo_options.repository_downloader_retries,
    ));
    let config = match BzlmodConfig::from_repository_options(
        &repo_options,
        &workspace,
        &cwd,
        &output_base,
        &install_base,
        semantics,
        client_env.clone(),
        client_env,
        fetcher,
    ) {
        Ok(c) => Arc::new(c),
        Err(e) => {
            reporter.handle(&Event::error(e.message.clone()));
            return 2;
        }
    };
    let lockfile_mode = config.lockfile_mode;

    let builder = register_bzlmod_functions(EvaluatorBuilder::new(), config);
    let builder = register_lockfile_only_extension_functions(builder);
    let evaluator = builder.build();
    let ctx = EvaluationContext {
        keep_going: true,
        event_handler: reporter.clone(),
        ..EvaluationContext::default()
    };
    let roots = [
        SkyKey::new(BazelDepGraphKey),
        SkyKey::new(BazelModuleInspectorKey),
    ];
    let result = match evaluator.evaluate(&roots, &ctx) {
        Ok(r) => r,
        Err(_) => {
            reporter.handle(&Event::error("mod command interrupted: null"));
            return 8;
        }
    };
    let code = if result.has_error() {
        let message = result
            .get_first_error()
            .and_then(|e| e.get_exception())
            .map(|e| e.to_string());
        exit_code(&report_evaluation_error(
            reporter.as_ref(),
            message.as_deref(),
        ))
    } else {
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
        );
        out.flush().ok();
        exit_code(&r)
    };
    after_command(&evaluator, &workspace, &output_base, lockfile_mode);
    code
}

fn exit_code(r: &ModCommandResult) -> i32 {
    match r {
        ModCommandResult::Success => 0,
        ModCommandResult::Failure { exit_code, .. } => *exit_code,
    }
}
