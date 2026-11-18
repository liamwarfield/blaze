//! W4 development driver: computes the configurations a corpus command creates (top-level,
//! test-trimmed, exec, no-config) and prints them in `bazel config --output=json` form, one per
//! line, prefixed with its role. Used by `scripts/w4/compare-config`.
//!
//! PORT: this is not a Bazel command. Until the dispatcher runs `cquery`/`config`, it parses the
//! command line the way the client and `BlazeOptionHandler` do (workspace `.bazelrc` only, as
//! with `--nohome_rc --nosystem_rc`), injects the module flag aliases like
//! `BlazeCommandDispatcher`, and takes two inputs Blaze cannot compute yet:
//! - `--mod-graph`: a `mod graph --output=json` (module names, versions) for `getFlagAliases`;
//! - `--platforms`: platform facts (`scripts/w4/oracle-platforms`) for the PLATFORM seam.
//!
//! The exec platform is the resolved host platform (toolchain resolution is not ported; for the
//! corpus the only execution platform is `--host_platform`).

use std::collections::BTreeMap;
use std::sync::Arc;

use blaze_build_config::bazel_rule_class_provider::BazelRuleClassProvider;
use blaze_build_config::build_configuration_value::{BuildConfigurationValue, GlobalStateProvider};
use blaze_build_config::config_output::{ConfigurationForOutput, json_write_configuration};
use blaze_build_config::flag_aliases::{FlagAliasModule, flag_alias_args, get_flag_aliases};
use blaze_build_config::function_transition_util::{
    exec_transition_patch, exec_transition_required_fragments,
};
use blaze_build_config::output_directories::ConfigDirectories;
use blaze_build_config::platform::{OraclePlatforms, PlatformValueProvider};
use blaze_build_config::top_level::{
    TopLevelConfigurationInputs, create_build_options, create_top_level_configuration,
    starlark_semantics_from,
};
use blaze_build_config::transitions::{EMPTY_OPTIONS, restrict, test_trimming_trim};
use blaze_cmdline::{Label, RepositoryName};
use blaze_events::{Event, EventHandler, ExtendedEventHandler};
use blaze_options::rc::handler::{BlazeOptionHandler, Command, StoredEventHandler};
use blaze_options::rc::option_processor::{
    ClientEnvironment, OptionProcessor, RecordingStartupArgsProcessor, StartupFlagRegistry,
};
use blaze_options::{OptionsClass, OptionsParser, PriorityCategory};
use blaze_vfs::Path;
use indexmap::IndexMap;

const USAGE: &str = "\
usage: blaze-config-dev --workspace DIR --output-base DIR --mod-graph FILE --platforms FILE
                        [--command cquery|build|test|info] [-- COMMAND_ARGS...]
       blaze-config-dev help

Prints one line per configuration: '<role> <config --output=json>' for the roles top, trimmed,
exec and noconfig; then 'make_env <json>' and 'dirs <json>' of the top-level configuration.";

static BUILD: Command = Command {
    name: "build",
    inherits_options_from: &[],
};
static TEST: Command = Command {
    name: "test",
    inherits_options_from: &[&BUILD],
};
static CQUERY: Command = Command {
    name: "cquery",
    inherits_options_from: &[&TEST],
};
static INFO: Command = Command {
    name: "info",
    inherits_options_from: &[&BUILD],
};

struct PrintingHandler;

impl EventHandler for PrintingHandler {
    fn handle(&self, event: &Event) {
        eprintln!("{:?}", event);
    }
    fn as_extended_event_handler(&self) -> Option<&dyn ExtendedEventHandler> {
        Some(self)
    }
}

impl ExtendedEventHandler for PrintingHandler {
    fn post(&self, _event: Arc<dyn blaze_events::Postable>) {}
}

fn options_classes(
    rule_class_provider: &BazelRuleClassProvider,
) -> Vec<blaze_options::definition::OptionsClassRef> {
    use blaze_optdefs::command::*;
    let mut classes: Vec<_> = vec![
        common_command_options::CommonCommandOptions::class(),
        blaze_options::rc::client_options::ClientOptions::class(),
        build_request_options::BuildRequestOptions::class(),
        execution_options::ExecutionOptions::class(),
        build_language_options::BuildLanguageOptions::class(),
        package_options::PackageOptions::class(),
        cquery_options::CqueryOptions::class(),
        ui_options::UiOptions::class(),
        test_summary_options::TestSummaryOptions::class(),
    ];
    for c in rule_class_provider.get_options_classes() {
        classes.push(c.options_class());
    }
    classes
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.is_empty() || args[0] == "help" || args[0] == "--help" {
        println!("{}", USAGE);
        return;
    }
    let mut workspace = None;
    let mut output_base = None;
    let mut mod_graph = None;
    let mut platforms_file = None;
    let mut command_name = "cquery".to_string();
    let mut command_args: Vec<String> = Vec::new();
    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--workspace" => {
                workspace = Some(args[i + 1].clone());
                i += 2;
            }
            "--output-base" => {
                output_base = Some(args[i + 1].clone());
                i += 2;
            }
            "--mod-graph" => {
                mod_graph = Some(args[i + 1].clone());
                i += 2;
            }
            "--platforms" => {
                platforms_file = Some(args[i + 1].clone());
                i += 2;
            }
            "--command" => {
                command_name = args[i + 1].clone();
                i += 2;
            }
            "--" => {
                command_args = args[i + 1..].to_vec();
                break;
            }
            other => {
                eprintln!("unknown argument: {}\n{}", other, USAGE);
                std::process::exit(2);
            }
        }
    }
    let workspace = workspace.expect("--workspace");
    let output_base = output_base.expect("--output-base");
    let command: &'static Command = match command_name.as_str() {
        "build" => &BUILD,
        "test" => &TEST,
        "cquery" => &CQUERY,
        "info" => &INFO,
        other => panic!("unsupported command {}", other),
    };

    // Client: rc files -> --default_override arguments.
    let mut processor = OptionProcessor::new(
        StartupFlagRegistry::bazel(),
        "/etc/bazel.bazelrc",
        "9.3.2",
        ClientEnvironment {
            cwd: workspace.clone(),
            home: std::env::var("HOME").unwrap_or_default(),
            ..ClientEnvironment::default()
        },
    );
    let mut client_args = vec![
        "bazel".to_string(),
        "--nohome_rc".to_string(),
        "--nosystem_rc".to_string(),
        command_name.clone(),
    ];
    client_args.extend(command_args);
    let mut startup = RecordingStartupArgsProcessor { flags: Vec::new() };
    processor
        .parse_options(client_args, &workspace, &workspace, &mut startup)
        .unwrap_or_else(|(_, e)| panic!("client: {}", e));
    // The command name, then the rc and command line arguments.
    let mut server_args = vec![processor.get_command()];
    server_args.extend(processor.get_command_arguments());

    if std::env::var("BLAZE_W4_DEBUG").is_ok() {
        eprintln!("server args: {:?}", server_args);
    }
    // Server: BlazeOptionHandler.
    let rule_class_provider = Arc::new(BazelRuleClassProvider::create());
    let classes = options_classes(&rule_class_provider);
    let mut parser = OptionsParser::builder()
        .options_classes(&classes)
        .allow_residue(true)
        .build()
        .unwrap_or_else(|e| panic!("{}", e.get_message()));
    let mut events = StoredEventHandler::default();
    {
        let mut handler = BlazeOptionHandler::new(
            "bazel",
            command,
            [
                "build", "test", "cquery", "info", "config", "query", "aquery",
            ],
            classes.clone(),
            &mut parser,
        );
        let (result, _) =
            handler.parse_options_and_get_config_definitions(&server_args, &mut events);
        if let Err(e) = result {
            panic!("options: {}", e.message);
        }
    }
    for e in events.get_events() {
        eprintln!("option event: {:?}", e);
    }

    // BlazeCommandDispatcher: inject MODULE.bazel flag aliases for commands that analyze.
    if command_name != "info" {
        let modules = read_mod_graph(&mod_graph.expect("--mod-graph"));
        let remove_py = option_bool(&parser, "incompatible_remove_ctx_py_fragment");
        let remove_bazel_py = option_bool(&parser, "incompatible_remove_ctx_bazel_py_fragment");
        let aliases = get_flag_aliases(&modules, remove_py, remove_bazel_py);
        parser
            .parse_with_priority(
                PriorityCategory::RC_FILE,
                Some("module resolution"),
                flag_alias_args(&aliases),
            )
            .unwrap_or_else(|e| panic!("{}", e.get_message()));
    }

    let oracle_platforms = Arc::new(
        OraclePlatforms::load(std::path::Path::new(&platforms_file.expect("--platforms")))
            .unwrap_or_else(|e| panic!("{}", e)),
    );
    let platforms: Arc<dyn PlatformValueProvider> = oracle_platforms.clone();
    let semantics = Arc::new(starlark_semantics_from(&parser));
    let fs = blaze_vfs::UnixFileSystem::create(blaze_vfs::DigestHashFunction::Sha256);
    let directories = ConfigDirectories {
        exec_root_base: Path::create(
            blaze_vfs::PathFragment::create(&format!("{}/execroot", output_base)),
            &fs,
        ),
        product_name: "bazel".to_string(),
    };
    let build_options = create_build_options(&rule_class_provider, &parser);
    let top = create_top_level_configuration(
        TopLevelConfigurationInputs {
            directories,
            rule_class_provider: rule_class_provider.clone(),
            starlark_semantics: semantics.clone(),
            package_path_roots: vec![Path::create(
                blaze_vfs::PathFragment::create(&workspace),
                &fs,
            )],
            platforms: platforms.clone(),
            event_handler: Arc::new(PrintingHandler),
        },
        build_options,
        true,
    )
    .unwrap_or_else(|e| panic!("{}", e.get_message()));

    let print = |role: &str, config: &BuildConfigurationValue| {
        let out = ConfigurationForOutput::get_configuration_for_output(
            config,
            rule_class_provider.get_fragment_registry(),
        );
        print!("{} {}", role, json_write_configuration(&out));
    };
    print("top", &top.configuration);

    // Non-test targets: TestTrimmingTransition on the top-level configuration.
    let trimmed_options = test_trimming_trim(top.configuration.get_options());
    let trimmed = top
        .skyframe
        .get_configuration(Arc::new(trimmed_options), true)
        .unwrap_or_else(|e| panic!("{}", e.get_message()));
    print("trimmed", &trimmed);

    // cfg = "exec" dependencies of non-test targets, then trimmed by their rule transition.
    let exec_platform = resolved_host_platform(&top.configuration, &oracle_platforms);
    let exec_transition = blaze_build_config::starlark_transition::load_starlark_exec_transition(
        Some(trimmed.get_options()),
        &semantics,
    )
    .unwrap_or_else(|e| panic!("{}", e))
    .expect("Starlark exec transition");
    let exec_options = exec_transition_patch(
        Some(&exec_platform),
        &exec_transition,
        &restrict(exec_transition_required_fragments(), trimmed.get_options()),
        &semantics,
        &PrintingHandler,
    );
    let exec = top
        .skyframe
        .get_configuration(Arc::new(test_trimming_trim(&exec_options)), true)
        .unwrap_or_else(|e| panic!("{}", e.get_message()));
    print("exec", &exec);

    // Platform and constraint targets: NoConfigTransition.
    let noconfig = top
        .skyframe
        .get_configuration(Arc::new(EMPTY_OPTIONS.clone()), true)
        .unwrap_or_else(|e| panic!("{}", e.get_message()));
    print("noconfig", &noconfig);

    let make_env: BTreeMap<String, String> = top
        .configuration
        .get_make_environment()
        .into_iter()
        .collect();
    println!("make_env {}", serde_json::to_string(&make_env).unwrap());
    let main = blaze_cmdline::repository_name::MAIN.clone();
    let dirs: BTreeMap<&str, String> = [
        (
            "bazel-bin",
            top.configuration
                .get_bin_directory(&main)
                .get_root_path()
                .to_string(),
        ),
        (
            "bazel-genfiles",
            top.configuration
                .get_genfiles_directory(&main)
                .get_root_path()
                .to_string(),
        ),
        (
            "bazel-testlogs",
            top.configuration
                .get_test_logs_directory(&main)
                .get_root_path()
                .to_string(),
        ),
    ]
    .into_iter()
    .collect();
    println!("dirs {}", serde_json::to_string(&dirs).unwrap());
}

/// The platform the host platform label resolves to (`PlatformInfo.label()`), via the PLATFORM
/// seam of the configuration evaluator.
fn resolved_host_platform(top: &BuildConfigurationValue, platforms: &OraclePlatforms) -> Label {
    use blaze_optdefs::fragment::platform_options::PlatformOptions;
    let host = top
        .get_options()
        .get::<PlatformOptions>()
        .expect("PlatformOptions")
        .host_platform
        .0
        .clone();
    // PORT: the registered execution platforms are [--host_platform] for the corpus; the exec
    // platform label is the resolved platform's label.
    platforms
        .get(&host)
        .map(|p| p.label.clone())
        .unwrap_or_else(|| panic!("no resolved platform for {}", host))
}

fn option_bool(parser: &OptionsParser, name: &str) -> bool {
    parser
        .get_option_value_description(name)
        .and_then(|d| d.get_value())
        .and_then(|v| v.downcast_ref::<bool>().copied())
        .unwrap_or_else(|| {
            parser
                .get_option_description(name)
                .map(|d| {
                    d.get_option_definition()
                        .get_default_value(None)
                        .and_then(|v| v.downcast_ref::<bool>().copied())
                        .unwrap_or(false)
                })
                .unwrap_or(false)
        })
}

/// Reads `mod graph --output=json` into the modules of the dependency graph (BFS order, no
/// duplicates). Canonical repo names: `name+`, or `name+version` when several versions of a
/// module are in the graph (`BazelDepGraphFunction.computeCanonicalRepoName`).
fn read_mod_graph(path: &str) -> Vec<FlagAliasModule> {
    let text = std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {}", path, e));
    let root: serde_json::Value = serde_json::from_str(&text).expect("mod graph JSON");
    let mut seen: IndexMap<String, (String, String)> = IndexMap::new();
    let mut queue = std::collections::VecDeque::new();
    queue.push_back(&root);
    while let Some(node) = queue.pop_front() {
        let key = node["key"].as_str().unwrap_or_default().to_string();
        if node.get("unexpanded").and_then(|u| u.as_bool()) == Some(true) && seen.contains_key(&key)
        {
            continue;
        }
        if !seen.contains_key(&key) {
            seen.insert(
                key.clone(),
                (
                    node["name"].as_str().unwrap_or_default().to_string(),
                    node["version"].as_str().unwrap_or_default().to_string(),
                ),
            );
        }
        if let Some(deps) = node.get("dependencies").and_then(|d| d.as_array()) {
            for d in deps {
                queue.push_back(d);
            }
        }
    }
    let mut versions_per_name: BTreeMap<String, usize> = BTreeMap::new();
    for (name, _) in seen.values() {
        *versions_per_name.entry(name.clone()).or_default() += 1;
    }
    seen.into_iter()
        .filter(|(k, _)| k != "<root>")
        .map(|(_, (name, version))| {
            let key = blaze_bzlmod_model::module_key::ModuleKey::new(
                name.clone(),
                blaze_bzlmod_model::Version::parse(&version).expect("version"),
            );
            let canonical_repo_name: RepositoryName = if versions_per_name[&name] > 1 {
                key.get_canonical_repo_name_with_version()
            } else {
                key.get_canonical_repo_name_without_version()
            };
            FlagAliasModule {
                name,
                version,
                canonical_repo_name,
                // PORT: `mod graph` does not print flag_alias() calls; the corpus modules
                // declare none (checked by scripts/w4/compare-config).
                flag_aliases: IndexMap::new(),
            }
        })
        .collect()
}
