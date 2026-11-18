//! `blaze info`.

pub mod info_item_handler;
pub mod items;
pub mod module_items;

use std::collections::BTreeMap;
use std::sync::Arc;

use blaze_events::{Event, EventHandler};
use blaze_optdefs::command::info_command::InfoCommandOptions;
use blaze_options::spelling::SpellChecker;
use blaze_protos::failure_details::{self, FailureDetail, failure_detail::Category};
use indexmap::{IndexMap, IndexSet};

pub use info_item_handler::{
    InfoItemHandler, InfoItemHandlerException, InfoItemHandlerFactory, InfoItemHandlerFactoryImpl,
    RemoteRequestedInfoItemHandler, StdoutInfoItemHandler,
};
pub use items::*;

use crate::command::{BlazeCommand, BlazeCommandResult, CommandAnnotation};
use crate::command_environment::CommandEnvironment;
use crate::exit_code::{DetailedExitCode, ExitCode};
use crate::info_item::{ConfigurationSupplier, InfoItem};
use crate::module::AbruptExitException;
use crate::options_result::OptionsParsingResult;

/// The memoizing configuration supplier of `blaze info` (Java: the `Suppliers.memoize` lambda in
/// `InfoCommand.exec`).
pub struct BuildConfigurationSupplier {
    options: Arc<OptionsParsingResult>,
    exec_root_base: blaze_vfs::Path,
    product_name: String,
    workspace: Option<blaze_vfs::Path>,
    reporter: Arc<blaze_events::Reporter>,
    configuration:
        Option<Arc<blaze_build_config::build_configuration_value::BuildConfigurationValue>>,
}

impl BuildConfigurationSupplier {
    pub fn new(env: &CommandEnvironment, options: &OptionsParsingResult) -> Self {
        let _ = options;
        BuildConfigurationSupplier {
            options: env.get_options().clone(),
            exec_root_base: env.get_directories().get_exec_root_base().clone(),
            product_name: env.get_runtime().get_product_name().to_string(),
            workspace: env.get_workspace().cloned(),
            reporter: env.get_reporter().clone(),
            configuration: None,
        }
    }

    // Java: com/google/devtools/build/lib/runtime/commands/InfoCommand.java:169 InfoCommand.exec
    fn configuration(
        &mut self,
    ) -> Result<
        Arc<blaze_build_config::build_configuration_value::BuildConfigurationValue>,
        AbruptExitException,
    > {
        use blaze_build_config::bazel_rule_class_provider::BazelRuleClassProvider;
        use blaze_build_config::top_level::{
            TopLevelConfigurationInputs, create_build_options, create_top_level_configuration,
            starlark_semantics_from,
        };
        if let Some(c) = &self.configuration {
            return Ok(c.clone());
        }
        // In order to be able to answer configuration-specific queries, we need to set up the
        // package path. Since info inherits all the build options, all the necessary information
        // is available here.
        // PORT: ensureSyncPackageLoading(env, optionsParsingResult) is not ported
        // (CommandEnvironment.syncPackageLoading); configuration creation reads the package
        // path roots (the workspace, `--package_path=%workspace%`) directly.
        let rule_class_provider = Arc::new(BazelRuleClassProvider::create());
        // TODO(bazel-team): What if there are multiple configurations? [multi-config]
        let build_options = create_build_options(&rule_class_provider, &self.options);
        // PORT: PlatformFunction (analysis of platform targets) is not ported; platform facts
        // come from BLAZE_W4_ORACLE_PLATFORMS (scripts/w4/oracle-platforms) when set.
        let platforms: blaze_build_config::platform::SharedPlatformValueProvider =
            match std::env::var("BLAZE_W4_ORACLE_PLATFORMS") {
                Ok(path) => Arc::new(
                    blaze_build_config::platform::OraclePlatforms::load(std::path::Path::new(
                        &path,
                    ))
                    .unwrap_or_else(|e| panic!("BLAZE_W4_ORACLE_PLATFORMS: {}", e)),
                ),
                Err(_) => Arc::new(blaze_build_config::platform::UnportedPlatforms),
            };
        let result = create_top_level_configuration(
            TopLevelConfigurationInputs {
                directories: blaze_build_config::output_directories::ConfigDirectories {
                    exec_root_base: self.exec_root_base.clone(),
                    product_name: self.product_name.clone(),
                },
                rule_class_provider,
                starlark_semantics: Arc::new(starlark_semantics_from(&self.options)),
                package_path_roots: self.workspace.iter().cloned().collect(),
                platforms,
                event_handler: self.reporter.clone(),
            },
            build_options,
            /* keep_going= */ true,
        );
        match result {
            Ok(top) => {
                self.configuration = Some(top.configuration.clone());
                Ok(top.configuration)
            }
            Err(e) => {
                self.reporter.handle(&Event::error(e.get_message()));
                let code = failure_details::build_configuration::Code::from_str_name(e.code)
                    .unwrap_or(
                        failure_details::build_configuration::Code::BuildConfigurationUnknown,
                    );
                Err(AbruptExitException::new(DetailedExitCode::of(
                    FailureDetail {
                        message: e.get_message().to_string(),
                        category: Some(Category::BuildConfiguration(
                            failure_details::BuildConfiguration { code: code as i32 },
                        )),
                    },
                )))
            }
        }
    }
}

impl ConfigurationSupplier for BuildConfigurationSupplier {
    fn get_make_environment(&mut self) -> Result<Vec<(String, String)>, AbruptExitException> {
        Ok(self
            .configuration()?
            .get_make_environment()
            .into_iter()
            .collect())
    }

    fn get_bin_directory(&mut self) -> Result<String, AbruptExitException> {
        let main = blaze_build_config::output_directories::main_repository();
        Ok(self
            .configuration()?
            .get_bin_directory(&main)
            .get_root_path()
            .to_string())
    }

    fn get_genfiles_directory(&mut self) -> Result<String, AbruptExitException> {
        let main = blaze_build_config::output_directories::main_repository();
        Ok(self
            .configuration()?
            .get_genfiles_directory(&main)
            .get_root_path()
            .to_string())
    }

    fn get_test_logs_directory(&mut self) -> Result<String, AbruptExitException> {
        let main = blaze_build_config::output_directories::main_repository();
        Ok(self
            .configuration()?
            .get_test_logs_directory(&main)
            .get_root_path()
            .to_string())
    }

    fn get(&mut self) -> Result<(), AbruptExitException> {
        self.configuration().map(|_| ())
    }
}

/// Implementation of 'blaze info'.
// Java: com/google/devtools/build/lib/runtime/commands/InfoCommand.java:110 InfoCommand
pub struct InfoCommand {
    info_item_handler_factory: Box<dyn InfoItemHandlerFactory>,
    /// Creates the (memoizing) configuration supplier of one `exec`.
    configuration_supplier_factory:
        fn(&CommandEnvironment, &OptionsParsingResult) -> Box<dyn ConfigurationSupplier>,
}

/// What the body of `exec`'s try block throws.
enum InfoException {
    AbruptExit(AbruptExitException),
    Io(std::io::Error),
}

impl From<AbruptExitException> for InfoException {
    fn from(e: AbruptExitException) -> Self {
        InfoException::AbruptExit(e)
    }
}

impl From<InfoItemHandlerException> for InfoException {
    fn from(e: InfoItemHandlerException) -> Self {
        match e {
            InfoItemHandlerException::AbruptExit(e) => InfoException::AbruptExit(e),
            InfoItemHandlerException::Io(e) => InfoException::Io(e),
        }
    }
}

/// `IOException.getMessage()` for an I/O error: the OS message without Rust's
/// " (os error N)" suffix.
fn java_io_message(e: &std::io::Error) -> String {
    let text = e.to_string();
    match text.rfind(" (os error ") {
        Some(i) if e.raw_os_error().is_some() => text[..i].to_string(),
        _ => text,
    }
}

impl InfoCommand {
    // Java: com/google/devtools/build/lib/runtime/commands/InfoCommand.java:164 InfoCommand.<init>
    pub fn new() -> Self {
        InfoCommand {
            info_item_handler_factory: Box::new(InfoItemHandlerFactoryImpl),
            configuration_supplier_factory: |env, options| {
                Box::new(BuildConfigurationSupplier::new(env, options))
            },
        }
    }

    /// For tests: a custom handler factory and configuration supplier.
    // Java: com/google/devtools/build/lib/runtime/commands/InfoCommand.java:160 InfoCommand.<init>
    pub fn with_factories(
        info_item_handler_factory: Box<dyn InfoItemHandlerFactory>,
        configuration_supplier_factory: fn(
            &CommandEnvironment,
            &OptionsParsingResult,
        ) -> Box<dyn ConfigurationSupplier>,
    ) -> Self {
        InfoCommand {
            info_item_handler_factory,
            configuration_supplier_factory,
        }
    }

    /// The body of Java's `try (InfoItemHandler infoItemHandler = ...) { ... }`. `Ok(Some(r))` is
    /// an early `return r`.
    fn exec_items(
        env: &mut CommandEnvironment,
        options_parsing_result: &OptionsParsingResult,
        info_options: &InfoCommandOptions,
        configuration_supplier: &mut dyn ConfigurationSupplier,
        items: &mut BTreeMap<String, Arc<dyn InfoItem>>,
        residue: &[String],
        info_item_handler: &mut dyn InfoItemHandler,
    ) -> Result<Option<BlazeCommandResult>, InfoException> {
        if info_options.show_make_environment {
            let make_env = configuration_supplier.get_make_environment()?;
            for (key, value) in make_env {
                let item: Arc<dyn InfoItem> = Arc::new(MakeInfoItem::new(&key, &value));
                items.insert(item.get_name().to_string(), item);
            }
        }

        // PORT: `env.getEventBus().post(new NoBuildEvent())` is skipped: Blaze has no event bus
        // yet.
        if !residue.is_empty() {
            let mut unknown_keys: IndexSet<String> = IndexSet::new();
            for key in residue {
                if let Some(info_item) = items.get(key).cloned() {
                    if info_item.needs_sync_package_loading() {
                        Self::ensure_sync_package_loading(env, options_parsing_result)?;
                    }
                    let value = info_item.get(configuration_supplier, env)?;
                    info_item_handler.add_info_item(key, &value)?;
                } else {
                    unknown_keys.insert(key.clone());
                }
            }
            if !unknown_keys.is_empty() {
                let message = format!(
                    "unknown key(s): {}",
                    unknown_keys
                        .iter()
                        .map(|key| format!(
                            "'{}'{}",
                            key,
                            SpellChecker::did_you_mean(key, items.keys())
                        ))
                        .collect::<Vec<_>>()
                        .join(", ")
                );
                env.get_reporter().handle(&Event::error(message.clone()));
                return Ok(Some(Self::create_failure_result(
                    &message,
                    ExitCode::COMMAND_LINE_ERROR,
                    failure_details::info_command::Code::KeyNotRecognized,
                )));
            }
        } else {
            // print them all
            configuration_supplier.get()?; // We'll need this later anyway
            for info_item in items.values() {
                if info_item.is_hidden() {
                    continue;
                }
                if info_item.needs_sync_package_loading() {
                    Self::ensure_sync_package_loading(env, options_parsing_result)?;
                }
                let value = info_item.get(configuration_supplier, env)?;
                info_item_handler.add_info_item(info_item.get_name(), &value)?;
            }
        }
        Ok(None)
    }

    // Java: com/google/devtools/build/lib/runtime/commands/InfoCommand.java:185 InfoCommand.ensureSyncPackageLoading
    fn ensure_sync_package_loading(
        env: &mut CommandEnvironment,
        options: &OptionsParsingResult,
    ) -> Result<(), AbruptExitException> {
        if !env.has_synced_package_loading() {
            env.sync_package_loading(options)?;
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/runtime/commands/InfoCommand.java:288 InfoCommand.createFailureResult
    fn create_failure_result(
        message: &str,
        exit_code: ExitCode,
        detailed_code: failure_details::info_command::Code,
    ) -> BlazeCommandResult {
        BlazeCommandResult::detailed_exit_code(DetailedExitCode::of_exit_code(
            exit_code,
            FailureDetail {
                message: message.to_string(),
                category: Some(Category::InfoCommand(failure_details::InfoCommand {
                    code: detailed_code as i32,
                })),
            },
        ))
    }

    /// The info items built into the info command, in Java's order. Panics on a duplicate name
    /// (`ImmutableMap.Builder.buildOrThrow`).
    // Java: com/google/devtools/build/lib/runtime/commands/InfoCommand.java:299 InfoCommand.getHardwiredInfoItemMap
    fn get_hardwired_info_item_map(
        command_options: Option<&OptionsParsingResult>,
        product_name: &str,
    ) -> IndexMap<String, Arc<dyn InfoItem>> {
        let hardwired_info_items: Vec<Arc<dyn InfoItem>> = vec![
            Arc::new(WorkspaceInfoItem::new()),
            Arc::new(InstallBaseInfoItem::new()),
            Arc::new(InstallMd5InfoItem::new()),
            Arc::new(OutputBaseInfoItem::new(product_name)),
            Arc::new(ExecutionRootInfoItem::new()),
            Arc::new(OutputPathInfoItem::new()),
            Arc::new(ClientEnv::new()),
            Arc::new(BlazeBinInfoItem::new(product_name)),
            Arc::new(BlazeGenfilesInfoItem::new(product_name)),
            Arc::new(BlazeTestlogsInfoItem::new(product_name)),
            Arc::new(ReleaseInfoItem::new(product_name)),
            Arc::new(ServerPidInfoItem::new(product_name)),
            Arc::new(ServerLogInfoItem::new(product_name)),
            Arc::new(PackagePathInfoItem::new(command_options)),
            Arc::new(UsedHeapSizeInfoItem::new()),
            Arc::new(UsedHeapSizeAfterGcInfoItem::new()),
            Arc::new(CommittedHeapSizeInfoItem::new()),
            Arc::new(MaxHeapSizeInfoItem::new()),
            Arc::new(GcTimeInfoItem::new()),
            Arc::new(GcCountInfoItem::new()),
            Arc::new(JavaRuntimeInfoItem::new()),
            Arc::new(JavaVirtualMachineInfoItem::new()),
            Arc::new(JavaHomeInfoItem::new()),
            Arc::new(CharacterEncodingInfoItem::new()),
            Arc::new(DefaultsPackageInfoItem::new()),
            Arc::new(BuildLanguageInfoItem::new()),
            Arc::new(DefaultPackagePathInfoItem::new(command_options)),
            Arc::new(StarlarkSemanticsInfoItem::new(command_options)),
            Arc::new(WorkerMetricsInfoItem::new()),
            Arc::new(LocalResourcesInfoItem::new()),
        ];
        let mut result: IndexMap<String, Arc<dyn InfoItem>> = IndexMap::new();
        for item in hardwired_info_items {
            let name = item.get_name().to_string();
            if result.insert(name.clone(), item).is_some() {
                panic!(
                    "IllegalArgumentException: Multiple entries with same key: {}",
                    name
                );
            }
        }
        result
    }

    // Java: com/google/devtools/build/lib/runtime/commands/InfoCommand.java:340 InfoCommand.getHardwiredInfoItemNames
    pub fn get_hardwired_info_item_names(product_name: &str) -> Vec<String> {
        Self::get_hardwired_info_item_map(None, product_name)
            .keys()
            .cloned()
            .collect()
    }

    /// The runtime's info items merged with the hardwired ones (which win), sorted by name.
    // Java: com/google/devtools/build/lib/runtime/commands/InfoCommand.java:204 InfoCommand.getInfoItemMap
    pub fn get_info_item_map(
        env: &CommandEnvironment,
        options_parsing_result: &OptionsParsingResult,
    ) -> BTreeMap<String, Arc<dyn InfoItem>> {
        let mut items: BTreeMap<String, Arc<dyn InfoItem>> = env
            .get_runtime()
            .get_info_items()
            .iter()
            .map(|(k, v)| (k.clone(), v.clone()))
            .collect();
        items.extend(Self::get_hardwired_info_item_map(
            Some(options_parsing_result),
            env.get_runtime().get_product_name(),
        ));
        items
    }
}

impl BlazeCommand for InfoCommand {
    fn annotation(&self) -> &'static CommandAnnotation {
        &crate::command_registry::INFO_COMMAND
    }

    // Java: com/google/devtools/build/lib/runtime/commands/InfoCommand.java:169 InfoCommand.exec
    fn exec(
        &self,
        env: &mut CommandEnvironment,
        options_parsing_result: &OptionsParsingResult,
    ) -> BlazeCommandResult {
        env.get_reporter().switch_to_ansi_allowing_handler();
        let info_options = options_parsing_result
            .get_options::<InfoCommandOptions>()
            .expect("InfoCommand.Options");
        // Creating a BuildConfigurationValue is expensive and often unnecessary. Delay the
        // creation until it is needed. We memoize so that it's cached intra-command (it's still
        // created freshly on every command since the configuration can change across commands).
        let mut configuration_supplier =
            (self.configuration_supplier_factory)(env, options_parsing_result);

        let mut items = Self::get_info_item_map(env, options_parsing_result);
        let residue = options_parsing_result.get_residue();

        let mut info_item_handler = self.info_item_handler_factory.create(
            env,
            info_options.info_output_type,
            /* print_keys= */ residue.len() != 1,
        );
        let body = Self::exec_items(
            env,
            options_parsing_result,
            &info_options,
            configuration_supplier.as_mut(),
            &mut items,
            &residue,
            info_item_handler.as_mut(),
        );
        // try-with-resources: close() runs after the body; its IOException is suppressed when
        // the body threw.
        let close = info_item_handler.close();
        let outcome = match body {
            Ok(result) => match close {
                Ok(()) => Ok(result),
                Err(e) => Err(InfoException::Io(e)),
            },
            Err(e) => Err(e),
        };
        match outcome {
            Ok(Some(result)) => result,
            Ok(None) => BlazeCommandResult::success(),
            Err(InfoException::AbruptExit(e)) => {
                BlazeCommandResult::detailed_exit_code(e.get_detailed_exit_code().clone())
            }
            Err(InfoException::Io(e)) => Self::create_failure_result(
                &format!("Cannot write info block: {}", java_io_message(&e)),
                ExitCode::LOCAL_ENVIRONMENTAL_ERROR,
                failure_details::info_command::Code::AllInfoWriteFailure,
            ),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The oracle's `bazel help info-keys` (Bazel 9.3.2): every info key (hardwired and from
    /// Bazel's modules) with its description, sorted by name.
    const ORACLE_INFO_KEYS: &str = "\
bazel-bin               Configuration dependent directory for binaries.
bazel-genfiles          Configuration dependent directory for generated files.
bazel-testlogs          Configuration dependent directory for logs from a test run.
build-language          A protobuffer with the build language structure
character-encoding      Information about the character encoding used by the running JVM.
client-env              The specifications that need to be added to the project-specific rc file to freeze the current client environment
command_log             Location of the log containing the output from the build commands.
committed-heap-size     The amount of memory in bytes that is committed for the Java virtual machine to use
default-package-path    The default package path
defaults-package        Obsolete. Retained for backwards compatibility.
execution_root          A directory that makes all input and output files visible to the build.
gc-count                Number of garbage collection runs.
gc-time                 The approximate accumulated time spend on garbage collection.
install_base            The installation base directory.
install_md5             The logical checksum of the Blaze server.
java-home               Location of the current Java runtime.
java-runtime            Name and version of the current Java runtime environment.
java-vm                 Name and version of the current Java virtual machine.
local_resources         Auto detected local resources
max-heap-size           The maximum amount of memory in bytes that can be used for memory management.
output_base             A directory for shared bazel state as well as tool and strategy specific subdirectories.
output_path             Output directory
package_path            The search path for resolving package labels.
peak-heap-size          The peak amount of used memory in bytes after any full GC during the most recent invocation.
release                 bazel release identifier
repository_cache        The location of the repository download cache used
server_log              bazel server log path
server_pid              bazel process id
starlark-semantics      The effective set of Starlark semantics option values.
used-heap-size          The amount of used memory in bytes. Note that this is not a good indicator of the actual memory use, as it includes any remaining inaccessible memory.
used-heap-size-after-gc The amount of used memory in bytes after a call to System.gc().
worker_metrics          persistent worker metrics
workspace               The working directory of the server.
";

    /// The hardwired items plus Bazel's module items, merged like `getInfoItemMap`.
    fn all_bazel_items() -> BTreeMap<String, Arc<dyn InfoItem>> {
        let module_items: Vec<Arc<dyn InfoItem>> = vec![
            Arc::new(module_items::CommandLogInfoItem::new()),
            Arc::new(module_items::RepositoryCacheInfoItem::new()),
            Arc::new(module_items::PeakMemInfoItem::new()),
        ];
        let mut items: BTreeMap<String, Arc<dyn InfoItem>> = module_items
            .into_iter()
            .map(|i| (i.get_name().to_string(), i))
            .collect();
        let empty = blaze_options::OptionsParser::builder().build().unwrap();
        items.extend(InfoCommand::get_hardwired_info_item_map(
            Some(&empty),
            "bazel",
        ));
        items
    }

    #[test]
    fn info_keys_match_oracle() {
        let mut listing = String::new();
        for item in all_bazel_items().values() {
            // HelpCommand.emitInfoKeysHelp
            listing.push_str(&format!(
                "{:<23} {}\n",
                item.get_name(),
                item.get_description()
            ));
        }
        assert_eq!(listing, ORACLE_INFO_KEYS);
    }

    /// The keys the oracle's argumentless `bazel info` prints (non-hidden), in order.
    #[test]
    fn non_hidden_keys_match_oracle_info_listing() {
        let shown: Vec<String> = all_bazel_items()
            .values()
            .filter(|i| !i.is_hidden())
            .map(|i| i.get_name().to_string())
            .collect();
        assert_eq!(
            shown,
            [
                "bazel-bin",
                "bazel-genfiles",
                "bazel-testlogs",
                "character-encoding",
                "command_log",
                "committed-heap-size",
                "execution_root",
                "gc-count",
                "gc-time",
                "install_base",
                "java-home",
                "java-runtime",
                "java-vm",
                "local_resources",
                "max-heap-size",
                "output_base",
                "output_path",
                "package_path",
                "release",
                "repository_cache",
                "server_log",
                "server_pid",
                "used-heap-size",
                "workspace",
            ]
        );
    }

    #[test]
    fn hardwired_info_item_names_in_java_order() {
        assert_eq!(
            InfoCommand::get_hardwired_info_item_names("bazel"),
            [
                "workspace",
                "install_base",
                "install_md5",
                "output_base",
                "execution_root",
                "output_path",
                "client-env",
                "bazel-bin",
                "bazel-genfiles",
                "bazel-testlogs",
                "release",
                "server_pid",
                "server_log",
                "package_path",
                "used-heap-size",
                "used-heap-size-after-gc",
                "committed-heap-size",
                "max-heap-size",
                "gc-time",
                "gc-count",
                "java-runtime",
                "java-vm",
                "java-home",
                "character-encoding",
                "defaults-package",
                "build-language",
                "default-package-path",
                "starlark-semantics",
                "worker_metrics",
                "local_resources",
            ]
        );
    }

    /// Oracle: `bazel info nosuchkey relase` prints
    /// `ERROR: unknown key(s): 'nosuchkey', 'relase' (did you mean 'release'?)`.
    #[test]
    fn unknown_key_suggestion_matches_oracle() {
        let items = all_bazel_items();
        let message = format!(
            "unknown key(s): {}",
            ["nosuchkey", "relase"]
                .iter()
                .map(|key| format!("'{}'{}", key, SpellChecker::did_you_mean(key, items.keys())))
                .collect::<Vec<_>>()
                .join(", ")
        );
        assert_eq!(
            message,
            "unknown key(s): 'nosuchkey', 'relase' (did you mean 'release'?)"
        );
    }

    #[test]
    fn unknown_key_failure_result_exit_code() {
        let result = InfoCommand::create_failure_result(
            "unknown key(s): 'x'",
            ExitCode::COMMAND_LINE_ERROR,
            failure_details::info_command::Code::KeyNotRecognized,
        );
        assert_eq!(result.get_exit_code().get_numeric_exit_code(), 2);
    }

    #[test]
    #[ignore = "needs a CommandEnvironment test fixture (BlazeRuntime + workspace) to run InfoCommand.exec end to end"]
    fn info_command_exec_end_to_end() {}
}
