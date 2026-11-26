//! Ports of `skyframe/BzlmodRepoCycleReporter.java`, `skyframe/BzlLoadCycleReporter.java`,
//! `repository/RequestRepositoryInformationEvent.java` and the subscriber half of
//! `bazel/repository/starlark/StarlarkRepositoryDebugModule.java`.
//!
//! PORT: the two reporters belong to the `skyframe` package (`blaze_sky_top`, next to
//! `TargetCycleReporter`); they name key types of `blaze_bzlmod`, `blaze_repo_rules` and
//! `blaze_sky_loading` that `blaze_sky_top` does not all depend on, so they are kept here, with
//! their only Blaze user (`mod`, `crate::commands::mod_command`). `SkyframeExecutor`'s own cycles
//! reporter does not register them (blaze_sky_top's `CyclesReporter` fails loudly on their
//! cycles).

use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use blaze_bzlmod::skyframe::keys::{ModuleFileKey, SingleExtensionEvalKey, SingleExtensionKey};
use blaze_events::{Event, EventHandler as _, ExtendedEventHandler, Postable};
use blaze_repo_rules::repository::repository_fetch_function::StarlarkRepositoryDefinitionLocationEvent;
use blaze_sky_loading::bzl_load::BzlLoadKey;
use blaze_sky_loading::file::FileKey;
use blaze_sky_loading::file_state::FileStateKey;
use blaze_sky_loading::package_function::PackageKey;
use blaze_sky_loading::package_lookup::PackageLookupKey;
use blaze_sky_loading::repos::{RepositoryDirectoryKey, RepositoryMappingKey};
use blaze_sky_loading::starlark_builtins::StarlarkBuiltinsKey;
use blaze_sky_top::cycles_reporter::{SingleCycleReporter, print_cycle};
use blaze_skyframe::{CycleInfo, SkyKey};
use parking_lot::Mutex;

/// `SkyFunctions.isSkyFunction(name)`.
fn is_sky_function(key: &SkyKey, name: &str) -> bool {
    key.function_name().0 == name
}

/// Asks the `StarlarkRepositoryDebugModule` to report where the named repository was defined.
// Java: com/google/devtools/build/lib/repository/RequestRepositoryInformationEvent.java:22 RequestRepositoryInformationEvent
#[derive(Debug, Clone)]
pub struct RequestRepositoryInformationEvent {
    name: String,
}

impl RequestRepositoryInformationEvent {
    pub fn new(name: String) -> Self {
        RequestRepositoryInformationEvent { name }
    }

    pub fn get_name(&self) -> &str {
        &self.name
    }
}

impl Postable for RequestRepositoryInformationEvent {}

/// `requestRepoDefinitions` of both reporters.
// Java: com/google/devtools/build/lib/skyframe/BzlmodRepoCycleReporter.java:83 BzlmodRepoCycleReporter.requestRepoDefinitions
fn request_repo_definitions(event_handler: &Arc<dyn ExtendedEventHandler>, repos: &[SkyKey]) {
    for repo in repos {
        if let Some(key) = repo.downcast::<RepositoryDirectoryKey>() {
            event_handler.post(Arc::new(RequestRepositoryInformationEvent::new(
                key.0.get_name().to_string(),
            )));
        }
    }
}

/// Reports cycles introduced by module extensions and .bzl files where they are declared.
// Java: com/google/devtools/build/lib/skyframe/BzlmodRepoCycleReporter.java:38 BzlmodRepoCycleReporter
pub struct BzlmodRepoCycleReporter;

impl SingleCycleReporter for BzlmodRepoCycleReporter {
    // Java: com/google/devtools/build/lib/skyframe/BzlmodRepoCycleReporter.java:95 BzlmodRepoCycleReporter.maybeReportCycle
    fn maybe_report_cycle(
        &self,
        _top_level_key: &SkyKey,
        cycle_info: &CycleInfo,
        already_reported: bool,
        event_handler: &Arc<dyn ExtendedEventHandler>,
    ) -> bool {
        let cycle = cycle_info.get_cycle();
        if already_reported {
            return true;
        }
        let is = |k: &SkyKey, names: &[&str]| names.iter().any(|n| is_sky_function(k, n));

        // This cycle reporter is aimed to handle cycles between any chaining of general .bzl
        // files, extension-generated repositories, extension evaluations, and the .bzl files
        // where they are declared. The state machine that describes this kind of cycles is:
        //    ________________________
        //   V                        |
        // PACKAGE -> REPOSITORY -> EXT -> BZL_LOAD -
        //                   ^                   ^   \
        //                   |___________________|___|
        // TODO(andreisolo): Figure out how to detect and print this kind of cycles more
        // specifically.
        if cycle.iter().all(|k| {
            is(
                k,
                &[
                    "REPOSITORY_DIRECTORY",
                    "PACKAGE_LOOKUP",
                    "REPO_DEFINITION",
                    "SINGLE_EXTENSION_EVAL",
                    "SINGLE_EXTENSION",
                    "BZL_LOAD",
                    "CONTAINING_PACKAGE_LOOKUP",
                    "REPOSITORY_MAPPING",
                    "MODULE_EXTENSION_REPO_MAPPING_ENTRIES",
                    "PACKAGE",
                    "BAZEL_MODULE_RESOLUTION",
                    "BAZEL_DEP_GRAPH",
                    "MODULE_FILE",
                    "FILE",
                    "FILE_STATE",
                ],
            )
        }) && cycle.iter().any(|k| {
            is(
                k,
                &[
                    "REPOSITORY_DIRECTORY",
                    "REPO_DEFINITION",
                    "SINGLE_EXTENSION_EVAL",
                ],
            )
        }) {
            let mut cycle_message = String::from(
                "Circular definition of repositories generated by module extensions or files in \
                 external repositories:",
            );
            let repos: Vec<SkyKey> = cycle
                .iter()
                .filter(|k| {
                    is(
                        k,
                        &[
                            "REPOSITORY_DIRECTORY",
                            "SINGLE_EXTENSION_EVAL",
                            "BZL_LOAD",
                            "REPOSITORY_MAPPING",
                            "BAZEL_MODULE_RESOLUTION",
                            "BAZEL_DEP_GRAPH",
                            "MODULE_FILE",
                        ],
                    )
                })
                .cloned()
                .collect();
            let printer = |input: &SkyKey| -> String {
                if let Some(key) = input.downcast::<RepositoryDirectoryKey>() {
                    key.0.to_string()
                } else if let Some(key) = input.downcast::<SingleExtensionEvalKey>() {
                    format!("module extension {}", key.argument())
                } else if let Some(key) = input.downcast::<SingleExtensionKey>() {
                    format!("module extension {}", key.argument())
                } else if let Some(key) = input.downcast::<RepositoryMappingKey>() {
                    format!("repository mapping of {}", key.repo_name())
                } else if is_sky_function(input, "BAZEL_MODULE_RESOLUTION") {
                    "module resolution".to_string()
                } else if is_sky_function(input, "BAZEL_DEP_GRAPH") {
                    "module dependency graph".to_string()
                } else if let Some(key) = input.downcast::<ModuleFileKey>() {
                    // ModuleFileValue.Key is its own argument (a record: its toString).
                    format!("module file of {:?}", key)
                } else if let Some(key) = input.downcast::<FileKey>() {
                    format!("file {}", key.argument())
                } else if let Some(key) = input.downcast::<FileStateKey>() {
                    format!("file state {}", key.argument())
                } else {
                    // Preconditions.checkArgument(input.argument() instanceof BzlLoadValue.Key)
                    match input.downcast::<BzlLoadKey>() {
                        Some(key) => key.get_label().to_string(),
                        None => panic!("IllegalArgumentException: {:?}", input),
                    }
                }
            };
            print_cycle(&repos, &mut cycle_message, &printer, &|_| false);
            event_handler.handle(&Event::error(cycle_message));
            // To help debugging, request that the information be printed about where the
            // respective repositories were defined.
            request_repo_definitions(event_handler, &repos);
            return true;
        } else if cycle.iter().any(|k| is_sky_function(k, "BZL_LOAD")) {
            let file_label = cycle
                .iter()
                .filter(|k| is_sky_function(k, "BZL_LOAD"))
                .next_back()
                .and_then(|k| k.downcast::<BzlLoadKey>())
                .expect("ClassCastException: BzlLoadValue.Key")
                .get_label()
                .clone();
            let error_message = if cycle[0]
                .downcast::<StarlarkBuiltinsKey>()
                .is_some_and(|k| k.with_autoloads)
            {
                // We know `fileLabel` is the last .bzl visited in the cycle. We also know that
                // BzlLoadFunction triggered the cycle by requesting StarlarkBuiltinsValue
                // w/autoloads. We know that we're not in builtins .bzls, because they don't
                // request w/autoloads. Thus, `fileLabel` is a .bzl transitively depended on by
                // an autoload.
                format!(
                    "Cycle caused by autoloads, failed to load .bzl file '{}'.\nAdd '{}' to \
                     --repositories_without_autoloads or disable autoloads by setting \
                     '--incompatible_autoload_externally='\nMore information on \
                     https://github.com/bazelbuild/bazel/issues/23043.\n",
                    file_label,
                    file_label.get_repository().get_name()
                )
            } else {
                format!(
                    "Failed to load .bzl file '{}': possible dependency cycle detected.\n",
                    file_label
                )
            };
            event_handler.handle(&Event::error(error_message));
            return true;
        } else if cycle.iter().any(|k| is_sky_function(k, "PACKAGE_LOOKUP")) {
            let pkg = cycle
                .iter()
                .filter(|k| is_sky_function(k, "PACKAGE_LOOKUP"))
                .next_back()
                .and_then(|k| k.downcast::<PackageLookupKey>())
                .expect("ClassCastException: PackageIdentifier")
                .0
                .clone();
            event_handler.handle(&Event::error(format!(
                "cannot load package '{}': possible dependency cycle detected.\n",
                pkg
            )));
            return true;
        }
        false
    }
}

/// Reports cycles of recursive import of Starlark files.
// Java: com/google/devtools/build/lib/skyframe/BzlLoadCycleReporter.java:33 BzlLoadCycleReporter
pub struct BzlLoadCycleReporter;

impl BzlLoadCycleReporter {
    fn is_bzlmod_extension(key: &SkyKey) -> bool {
        is_sky_function(key, "SINGLE_EXTENSION") || is_sky_function(key, "SINGLE_EXTENSION_EVAL")
    }
}

impl SingleCycleReporter for BzlLoadCycleReporter {
    // Java: com/google/devtools/build/lib/skyframe/BzlLoadCycleReporter.java:63 BzlLoadCycleReporter.maybeReportCycle
    fn maybe_report_cycle(
        &self,
        _top_level_key: &SkyKey,
        cycle_info: &CycleInfo,
        already_reported: bool,
        event_handler: &Arc<dyn ExtendedEventHandler>,
    ) -> bool {
        let path_to_cycle = cycle_info.get_path_to_cycle();
        let cycle = cycle_info.get_cycle();
        let Some(last_path_element) = path_to_cycle.last() else {
            return false;
        };
        if already_reported {
            return true;
        }
        let is_package = |k: &SkyKey| is_sky_function(k, "PACKAGE");
        let is_bzl_load = |k: &SkyKey| is_sky_function(k, "BZL_LOAD");

        if cycle.iter().all(is_bzl_load)
            // The last element before the cycle has to be a PackageFunction, a .bzl file, or an
            // extension eval.
            && (is_package(last_path_element)
                || is_bzl_load(last_path_element)
                || Self::is_bzlmod_extension(last_path_element))
        {
            let printer = |input: &SkyKey| -> String {
                if let Some(key) = input.downcast::<BzlLoadKey>() {
                    return key.get_label().to_string();
                }
                if let Some(key) = input.downcast::<PackageKey>() {
                    return format!("{}/BUILD", key.0);
                }
                // Preconditions.checkArgument(input.argument() instanceof ModuleExtensionId)
                if let Some(key) = input.downcast::<SingleExtensionEvalKey>() {
                    return format!("module extension {}", key.argument());
                }
                match input.downcast::<SingleExtensionKey>() {
                    Some(key) => format!("module extension {}", key.argument()),
                    None => panic!("IllegalArgumentException: {:?}", input),
                }
            };

            let mut cycle_message = String::from("cycle detected in extension files: ");

            // go back the path that lead to the cycle till we found the BUILD file or extension
            // eval that lead to the circular load,
            let mut start_index = path_to_cycle.len() - 1;
            while start_index > 0
                && (is_package(&path_to_cycle[start_index - 1])
                    || is_bzl_load(&path_to_cycle[start_index - 1])
                    || Self::is_bzlmod_extension(&path_to_cycle[start_index - 1]))
            {
                start_index -= 1;
            }
            for key in &path_to_cycle[start_index..] {
                cycle_message.push_str("\n    ");
                cycle_message.push_str(&printer(key));
            }
            print_cycle(cycle, &mut cycle_message, &printer, &|_| false);
            // TODO(bazel-team): it would be nice to pass the Location of the load Statement in
            // the BUILD file.
            event_handler.handle(&Event::error(cycle_message));
            return true;
        } else if cycle.iter().all(|k| {
            is_sky_function(k, "PACKAGE_LOOKUP") || is_sky_function(k, "REPOSITORY_DIRECTORY")
        }) {
            let mut cycle_message = String::from("Circular definition of repositories:");
            let repos: Vec<SkyKey> = cycle
                .iter()
                .filter(|k| is_sky_function(k, "REPOSITORY_DIRECTORY"))
                .cloned()
                .collect();
            let printer = |input: &SkyKey| -> String {
                match input.downcast::<RepositoryDirectoryKey>() {
                    Some(key) => key.0.to_string(),
                    None => panic!("UnsupportedOperationException"),
                }
            };
            print_cycle(&repos, &mut cycle_message, &printer, &|_| false);
            event_handler.handle(&Event::error(cycle_message));
            // To help debugging, request that the information be printed about where the
            // respective repositories were defined.
            request_repo_definitions(event_handler, &repos);
            return true;
        }
        false
    }
}

/// The event-bus half of `StarlarkRepositoryDebugModule`: reports back the place an external
/// repository was defined, if requested by some error involving that repository.
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryDebugModule.java:31 StarlarkRepositoryDebugModule
pub struct StarlarkRepositoryDebugSubscriber {
    repository_definitions: Mutex<HashMap<String, String>>,
    reported: Mutex<HashSet<String>>,
    reporter: Arc<blaze_events::Reporter>,
}

impl StarlarkRepositoryDebugSubscriber {
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryDebugModule.java:37 StarlarkRepositoryDebugModule.beforeCommand
    pub fn new(reporter: Arc<blaze_events::Reporter>) -> Self {
        StarlarkRepositoryDebugSubscriber {
            repository_definitions: Mutex::new(HashMap::new()),
            reported: Mutex::new(HashSet::new()),
            reporter,
        }
    }
}

impl blaze_events::EventBus for StarlarkRepositoryDebugSubscriber {
    fn post(&self, event: Arc<dyn Postable>) {
        let any: &dyn std::any::Any = &*event;
        // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryDebugModule.java:53 StarlarkRepositoryDebugModule.definitionLocation
        if let Some(e) = any.downcast_ref::<StarlarkRepositoryDefinitionLocationEvent>() {
            self.repository_definitions
                .lock()
                .insert(e.name.clone(), e.definition_information.clone());
        // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryDebugModule.java:59 StarlarkRepositoryDebugModule.requestDefinition
        } else if let Some(e) = any.downcast_ref::<RequestRepositoryInformationEvent>() {
            let to_report = if self.reported.lock().insert(e.get_name().to_string()) {
                self.repository_definitions
                    .lock()
                    .get(e.get_name())
                    .cloned()
            } else {
                None
            };
            if let Some(to_report) = to_report {
                self.reporter.handle(&Event::info(to_report));
            }
        }
    }
}
