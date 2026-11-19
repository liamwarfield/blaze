//! `--incompatible_autoload_externally`: which symbols Bazel loads from rules repositories.
//!
//! Java: com/google/devtools/build/lib/packages/AutoloadSymbols.java.
//!
//! PORT: Java's `getLoadKeys(env)` requests `BazelDepGraphValue` itself; here the caller
//! (blaze_sky_loading's StarlarkBuiltinsFunction) requests it and passes it to
//! [`AutoloadSymbols::get_load_keys`], which returns the labels Java wraps in
//! `BzlLoadValue.keyForBuild`.

use std::fmt;

use allocative::Allocative;
use blaze_bzlmod_model::{BazelDepGraphValue, ModuleKey, Version};
use blaze_cmdline::{Label, LabelRepoContextImpl, RepositoryMapping, RepositoryName};
use blaze_starlark::StarlarkSemantics;
use blaze_starlark::structs::StarlarkInfo;
use indexmap::{IndexMap, IndexSet};
use starlark::eval::{Arguments, Evaluator};
use starlark::starlark_simple_value;
use starlark::values::{NoSerialize, OwnedFrozenValue, StarlarkValue, Value, starlark_value};

use crate::builtins::env::{
    EnvEntry, StarlarkEnv, create_struct_with_message, freeze_value, struct_fields,
};

/// Configuration of a symbol.
// Java: com/google/devtools/build/lib/packages/AutoloadSymbols.java:587 AutoloadSymbols.SymbolRedirect
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SymbolRedirect {
    pub load_label: &'static str,
    pub rule: bool,
    pub new_name: Option<&'static str>,
    pub rdeps: &'static [&'static str],
}

/// `SymbolRedirect.moduleToRepoName` (a bimap).
const MODULE_TO_REPO_NAME: &[(&str, &str)] = &[
    ("apple_support", "build_bazel_apple_support"),
    ("protobuf", "com_google_protobuf"),
];

impl SymbolRedirect {
    // Java: com/google/devtools/build/lib/packages/AutoloadSymbols.java:598 SymbolRedirect.getModuleName
    pub fn get_module_name(&self) -> String {
        let repo_name = self.get_repo_name();
        MODULE_TO_REPO_NAME
            .iter()
            .find(|(_, repo)| *repo == repo_name)
            .map_or(repo_name.clone(), |(module, _)| (*module).to_owned())
    }

    // Java: com/google/devtools/build/lib/packages/AutoloadSymbols.java:603 SymbolRedirect.getRepoName
    pub fn get_repo_name(&self) -> String {
        Label::parse_canonical_unchecked(self.load_label)
            .get_repository()
            .get_name()
            .to_owned()
    }

    // Java: com/google/devtools/build/lib/packages/AutoloadSymbols.java:607 SymbolRedirect.getLabel
    fn get_label(&self, repo_context: &LabelRepoContextImpl) -> Label {
        Label::parse_with_repo_context(self.load_label, repo_context)
            .unwrap_or_else(|e| panic!("IllegalStateException: {e}"))
    }

    // Java: com/google/devtools/build/lib/packages/AutoloadSymbols.java:615 SymbolRedirect.toRepoName
    fn to_repo_name(module_key: &ModuleKey) -> String {
        MODULE_TO_REPO_NAME
            .iter()
            .find(|(module, _)| *module == module_key.name())
            .map_or(module_key.name().to_owned(), |(_, repo)| (*repo).to_owned())
    }
}

// Java: com/google/devtools/build/lib/packages/AutoloadSymbols.java:629 AutoloadSymbols.ruleRedirect
const fn rule_redirect(load_label: &'static str) -> SymbolRedirect {
    SymbolRedirect {
        load_label,
        rule: true,
        new_name: None,
        rdeps: &[],
    }
}

// Java: com/google/devtools/build/lib/packages/AutoloadSymbols.java:633 AutoloadSymbols.symbolRedirect
const fn symbol_redirect(
    load_label: &'static str,
    rdeps: &'static [&'static str],
) -> SymbolRedirect {
    SymbolRedirect {
        load_label,
        rule: false,
        new_name: None,
        rdeps,
    }
}

// Java: com/google/devtools/build/lib/packages/AutoloadSymbols.java:637 AutoloadSymbols.renamedSymbolRedirect
const fn renamed_symbol_redirect(
    load_label: &'static str,
    new_name: &'static str,
    rdeps: &'static [&'static str],
) -> SymbolRedirect {
    SymbolRedirect {
        load_label,
        rule: false,
        new_name: Some(new_name),
        rdeps,
    }
}

/// Repositories where autoloads are never used.
// Java: com/google/devtools/build/lib/packages/AutoloadSymbols.java:642 AutoloadSymbols.PREDECLARED_REPOS_DISALLOWING_AUTOLOADS
const PREDECLARED_REPOS_DISALLOWING_AUTOLOADS: &[&str] = &[
    "protobuf",
    "com_google_protobuf",
    "proto_bazel_features",
    "rules_android",
    "rules_cc",
    "rules_java",
    "rules_java_builtin",
    "compatibility_proxy",
    "rules_python",
    "rules_python_internal",
    "rules_shell",
    "apple_support",
    "build_bazel_apple_support",
    "bazel_skylib",
    "bazel_tools",
    "bazel_features",
    "bazel_features_version",
    "bazel_features_globals",
];

/// Modules whose versions up to these have the rules natively (skipped by `getLoadKeys`).
// Java: com/google/devtools/build/lib/packages/AutoloadSymbols.java:662 AutoloadSymbols.requiredVersionForModules
fn required_version_for_module(module_name: &str) -> Option<Version> {
    let version = match module_name {
        "protobuf" => "29.0-rc1",
        "rules_android" => "0.6.0-rc1",
        _ => return None,
    };
    Some(Version::parse(version).unwrap_or_else(|e| panic!("IllegalStateException: {e}")))
}

/// The symbols that may be autoloaded, in Java's table order (generated from the Java table).
// Java: com/google/devtools/build/lib/packages/AutoloadSymbols.java:675 AutoloadSymbols.AUTOLOAD_CONFIG
pub const AUTOLOAD_CONFIG: &[(&str, SymbolRedirect)] = &[
    (
        "CcSharedLibraryInfo",
        symbol_redirect(
            "@rules_cc//cc/common:cc_shared_library_info.bzl",
            &["cc_shared_library"],
        ),
    ),
    (
        "CcSharedLibraryHintInfo",
        symbol_redirect(
            "@rules_cc//cc/common:cc_shared_library_hint_info.bzl",
            &["cc_common"],
        ),
    ),
    (
        "cc_proto_aspect",
        symbol_redirect(
            "@com_google_protobuf//bazel/private:bazel_cc_proto_library.bzl",
            &["cc_proto_aspect"],
        ),
    ),
    (
        "ProtoInfo",
        symbol_redirect(
            "@com_google_protobuf//bazel/common:proto_info.bzl",
            &[
                "proto_library",
                "cc_proto_library",
                "java_lite_proto_library",
                "java_proto_library",
                "proto_lang_toolchain",
                "java_binary",
            ],
        ),
    ),
    (
        "proto_common_do_not_use",
        symbol_redirect("@com_google_protobuf//:dummy.bzl", &[]),
    ),
    (
        "cc_common",
        symbol_redirect("@rules_cc//cc/common:cc_common.bzl", &[]),
    ),
    (
        "CcInfo",
        symbol_redirect(
            "@rules_cc//cc/common:cc_info.bzl",
            &[
                "cc_binary",
                "cc_library",
                "cc_test",
                "cc_shared_library",
                "cc_common",
                "java_library",
                "cc_proto_library",
                "java_import",
                "java_runtime",
                "java_binary",
                "objc_library",
                "java_common",
                "JavaInfo",
                "py_extension",
                "cc_import",
                "objc_import",
                "objc_library",
                "cc_toolchain",
                "PyCcLinkParamsProvider",
                "py_library",
            ],
        ),
    ),
    (
        "DebugPackageInfo",
        symbol_redirect(
            "@rules_cc//cc/common:debug_package_info.bzl",
            &["cc_binary", "cc_test"],
        ),
    ),
    (
        "CcToolchainConfigInfo",
        symbol_redirect(
            "@rules_cc//cc/toolchains:cc_toolchain_config_info.bzl",
            &["cc_toolchain"],
        ),
    ),
    (
        "java_common",
        symbol_redirect("@rules_java//java/common:java_common.bzl", &[]),
    ),
    (
        "JavaInfo",
        symbol_redirect(
            "@rules_java//java/common:java_info.bzl",
            &[
                "java_binary",
                "java_library",
                "java_test",
                "java_proto_library",
                "java_lite_proto_library",
                "java_plugin",
                "java_import",
                "java_common",
            ],
        ),
    ),
    (
        "JavaPluginInfo",
        symbol_redirect(
            "@rules_java//java/common:java_plugin_info.bzl",
            &["java_plugin", "java_library", "java_binary", "java_test"],
        ),
    ),
    (
        "ProguardSpecProvider",
        renamed_symbol_redirect(
            "@rules_java//java/common:proguard_spec_info.bzl",
            "ProguardSpecInfo",
            &[
                "java_lite_proto_library",
                "java_import",
                "android_binary",
                "android_library",
            ],
        ),
    ),
    (
        "PyInfo",
        symbol_redirect(
            "@rules_python//python:py_info.bzl",
            &["py_binary", "py_test", "py_library"],
        ),
    ),
    (
        "PyRuntimeInfo",
        symbol_redirect(
            "@rules_python//python:py_runtime_info.bzl",
            &["py_binary", "py_test", "py_library"],
        ),
    ),
    (
        "PyCcLinkParamsProvider",
        renamed_symbol_redirect(
            "@rules_python//python:py_cc_link_params_info.bzl",
            "PyCcLinkParamsInfo",
            &["py_binary", "py_test", "py_library"],
        ),
    ),
    (
        "AndroidIdeInfo",
        symbol_redirect(
            "@rules_android//providers:providers.bzl",
            &[
                "aar_import",
                "android_binary",
                "android_library",
                "android_local_test",
                "android_sdk",
            ],
        ),
    ),
    (
        "aar_import",
        rule_redirect("@rules_android//rules:rules.bzl"),
    ),
    (
        "android_binary",
        rule_redirect("@rules_android//rules:rules.bzl"),
    ),
    (
        "android_library",
        rule_redirect("@rules_android//rules:rules.bzl"),
    ),
    (
        "android_local_test",
        rule_redirect("@rules_android//rules:rules.bzl"),
    ),
    (
        "android_sdk",
        rule_redirect("@rules_android//rules:rules.bzl"),
    ),
    (
        "android_tools_defaults_jar",
        rule_redirect("@rules_android//rules:rules.bzl"),
    ),
    ("cc_binary", rule_redirect("@rules_cc//cc:cc_binary.bzl")),
    ("cc_import", rule_redirect("@rules_cc//cc:cc_import.bzl")),
    ("cc_library", rule_redirect("@rules_cc//cc:cc_library.bzl")),
    (
        "cc_proto_library",
        rule_redirect("@com_google_protobuf//bazel:cc_proto_library.bzl"),
    ),
    (
        "cc_shared_library",
        rule_redirect("@rules_cc//cc:cc_shared_library.bzl"),
    ),
    ("cc_test", rule_redirect("@rules_cc//cc:cc_test.bzl")),
    (
        "cc_toolchain",
        rule_redirect("@rules_cc//cc/toolchains:cc_toolchain.bzl"),
    ),
    (
        "cc_toolchain_suite",
        rule_redirect("@rules_cc//cc/toolchains:cc_toolchain_suite.bzl"),
    ),
    (
        "fdo_prefetch_hints",
        rule_redirect("@rules_cc//cc/toolchains:fdo_prefetch_hints.bzl"),
    ),
    (
        "fdo_profile",
        rule_redirect("@rules_cc//cc/toolchains:fdo_profile.bzl"),
    ),
    (
        "java_binary",
        rule_redirect("@rules_java//java:java_binary.bzl"),
    ),
    (
        "java_import",
        rule_redirect("@rules_java//java:java_import.bzl"),
    ),
    (
        "java_library",
        rule_redirect("@rules_java//java:java_library.bzl"),
    ),
    (
        "java_lite_proto_library",
        rule_redirect("@com_google_protobuf//bazel:java_lite_proto_library.bzl"),
    ),
    (
        "java_package_configuration",
        rule_redirect("@rules_java//java/toolchains:java_package_configuration.bzl"),
    ),
    (
        "java_plugin",
        rule_redirect("@rules_java//java:java_plugin.bzl"),
    ),
    (
        "java_proto_library",
        rule_redirect("@com_google_protobuf//bazel:java_proto_library.bzl"),
    ),
    (
        "java_runtime",
        rule_redirect("@rules_java//java/toolchains:java_runtime.bzl"),
    ),
    (
        "java_test",
        rule_redirect("@rules_java//java:java_test.bzl"),
    ),
    (
        "java_toolchain",
        rule_redirect("@rules_java//java/toolchains:java_toolchain.bzl"),
    ),
    (
        "memprof_profile",
        rule_redirect("@rules_cc//cc/toolchains:memprof_profile.bzl"),
    ),
    (
        "objc_import",
        rule_redirect("@rules_cc//cc:objc_import.bzl"),
    ),
    (
        "objc_library",
        rule_redirect("@rules_cc//cc:objc_library.bzl"),
    ),
    (
        "propeller_optimize",
        rule_redirect("@rules_cc//cc/toolchains:propeller_optimize.bzl"),
    ),
    (
        "proto_lang_toolchain",
        rule_redirect("@com_google_protobuf//bazel/toolchains:proto_lang_toolchain.bzl"),
    ),
    (
        "proto_library",
        rule_redirect("@com_google_protobuf//bazel:proto_library.bzl"),
    ),
    (
        "py_binary",
        rule_redirect("@rules_python//python:py_binary.bzl"),
    ),
    (
        "py_library",
        rule_redirect("@rules_python//python:py_library.bzl"),
    ),
    (
        "py_runtime",
        rule_redirect("@rules_python//python:py_runtime.bzl"),
    ),
    (
        "py_test",
        rule_redirect("@rules_python//python:py_test.bzl"),
    ),
    (
        "sh_binary",
        rule_redirect("@rules_shell//shell:sh_binary.bzl"),
    ),
    (
        "sh_library",
        rule_redirect("@rules_shell//shell:sh_library.bzl"),
    ),
    ("sh_test", rule_redirect("@rules_shell//shell:sh_test.bzl")),
    (
        "available_xcodes",
        rule_redirect("@build_bazel_apple_support//xcode:available_xcodes.bzl"),
    ),
    (
        "xcode_config",
        rule_redirect("@build_bazel_apple_support//xcode:xcode_config.bzl"),
    ),
    (
        "xcode_config_alias",
        rule_redirect("@build_bazel_apple_support//xcode:xcode_config_alias.bzl"),
    ),
    (
        "xcode_version",
        rule_redirect("@build_bazel_apple_support//xcode:xcode_version.bzl"),
    ),
    (
        "apple_common",
        symbol_redirect(
            "@build_bazel_apple_support//lib:apple_common.bzl",
            &["objc_import", "objc_library"],
        ),
    ),
];

fn autoload_config(symbol: &str) -> Option<&'static SymbolRedirect> {
    AUTOLOAD_CONFIG
        .iter()
        .find(|(name, _)| *name == symbol)
        .map(|(_, r)| r)
}

/// Java `AutoloadSymbols`, the value of the `AUTOLOAD_SYMBOLS` precomputed variable.
// Java: com/google/devtools/build/lib/packages/AutoloadSymbols.java:72 AutoloadSymbols
pub struct AutoloadSymbols {
    // Following fields autoloaded_symbols, removed_symbols, partially_removed_symbols,
    // repos_disallowing_autoloads are empty if autoloads aren't used
    /// Symbols that aren't prefixed with '-', that get loaded from external repositories (in
    /// allowed repositories).
    autoloaded_symbols: Vec<String>,
    /// Symbols that are prefixed with '-', that are removed everywhere (from each repository).
    removed_symbols: Vec<String>,
    /// Symbols that aren't prefixed with '+', that are removed from everywhere, except in
    /// repositories where autoloaded symbols are defined.
    partially_removed_symbols: Vec<String>,
    /// Repositories where autoloads shouldn't be used.
    repos_disallowing_autoloads: IndexSet<String>,
    /// The environment formed by taking BazelStarlarkEnvironment's bzl environment and
    /// adding/removing autoloaded symbols. The values of any added symbols are set to None (i.e.
    /// not actually loaded). This is intended for BzlCompileFunction only.
    uninjected_build_bzl_env_with_autoloads: StarlarkEnv,
    /// bzl environment where autoloads aren't used, uninjected (not loaded yet).
    uninjected_build_bzl_env_without_autoloads: StarlarkEnv,
    /// If the list of symbols to autoload is empty, autoloads are disabled.
    autoloads_enabled: bool,
    /// Should the autoloads be disabled in root project (main repository).
    disable_autoloads_in_main_repo: bool,
}

/// `symbol.startsWith("+") || symbol.startsWith("-") ? symbol.substring(1) : symbol`.
fn without_prefix(symbol: &str) -> &str {
    if symbol.starts_with('+') || symbol.starts_with('-') {
        &symbol[1..]
    } else {
        symbol
    }
}

/// The value Java puts for a symbol that is only declared (`Starlark.NONE`).
fn none_value() -> OwnedFrozenValue {
    freeze_value(|_| Value::new_none())
}

impl AutoloadSymbols {
    /// `original_build_bzl_env` is `BazelStarlarkEnvironment.getUninjectedBuildBzlEnv()`.
    ///
    /// Panics with Java's `IllegalStateException` text on an invalid flag value (Bazel crashes
    /// there too).
    // Java: com/google/devtools/build/lib/packages/AutoloadSymbols.java:102 AutoloadSymbols.<init>
    pub fn new(
        original_build_bzl_env: &StarlarkEnv,
        semantics: &StarlarkSemantics,
    ) -> AutoloadSymbols {
        let symbol_configuration = &semantics.incompatible_autoload_externally;
        let disable_autoloads_in_main_repo = semantics.incompatible_disable_autoloads_in_main_repo;
        let autoloads_enabled = !symbol_configuration.is_empty();

        if !autoloads_enabled {
            return AutoloadSymbols {
                autoloaded_symbols: Vec::new(),
                removed_symbols: Vec::new(),
                partially_removed_symbols: Vec::new(),
                repos_disallowing_autoloads: IndexSet::new(),
                uninjected_build_bzl_env_with_autoloads: original_build_bzl_env.clone(),
                uninjected_build_bzl_env_without_autoloads: original_build_bzl_env.clone(),
                autoloads_enabled,
                disable_autoloads_in_main_repo,
            };
        }

        // Expand symbols given with @rules_foo
        let symbol_configuration: Vec<String> = symbol_configuration
            .iter()
            .flat_map(|flag| {
                let mut prefix = "";
                let mut flag_without_prefix = flag.as_str();
                if flag.starts_with('+') || flag.starts_with('-') {
                    prefix = &flag[..1];
                    flag_without_prefix = &flag[1..];
                }
                if flag_without_prefix.starts_with('@') {
                    Self::get_all_symbols(flag_without_prefix, prefix)
                } else {
                    vec![flag.clone()]
                }
            })
            .collect();

        // Validates the inputs
        let mut unique_symbols: std::collections::HashSet<&str> = Default::default();
        for symbol in &symbol_configuration {
            let symbol_without_prefix = without_prefix(symbol);
            if !unique_symbols.insert(symbol_without_prefix) {
                panic!(
                    "IllegalStateException: Duplicated symbol '{}' in \
                     --incompatible_autoload_externally",
                    symbol_without_prefix
                );
            }
            if autoload_config(symbol_without_prefix).is_none() {
                panic!(
                    "IllegalStateException: Undefined symbol in \
                     --incompatible_autoload_externally: {}",
                    symbol_without_prefix
                );
            }
        }

        let autoloaded_symbols = Self::filter_symbols(&symbol_configuration, |symbol| {
            !symbol.starts_with('-') && !symbol.ends_with("proto_common_do_not_use")
        });
        let removed_symbols = Self::filter_symbols(&symbol_configuration, |symbol| {
            symbol.starts_with('-') || symbol.ends_with("proto_common_do_not_use")
        });
        let partially_removed_symbols =
            Self::filter_symbols(&symbol_configuration, |symbol| !symbol.starts_with('+'));

        let mut repos_disallowing_autoloads: IndexSet<String> =
            PREDECLARED_REPOS_DISALLOWING_AUTOLOADS
                .iter()
                .map(|r| (*r).to_owned())
                .collect();
        repos_disallowing_autoloads
            .extend(semantics.repositories_without_autoloads.iter().cloned());

        let mut this = AutoloadSymbols {
            autoloaded_symbols,
            removed_symbols,
            partially_removed_symbols,
            repos_disallowing_autoloads,
            uninjected_build_bzl_env_with_autoloads: StarlarkEnv::new(),
            uninjected_build_bzl_env_without_autoloads: StarlarkEnv::new(),
            autoloads_enabled,
            disable_autoloads_in_main_repo,
        };

        // Sets up environments for BzlCompile function
        let new_symbols: StarlarkEnv = this
            .autoloaded_symbols
            .iter()
            .map(|key| (key.clone(), EnvEntry::plain(none_value())))
            .collect();
        this.uninjected_build_bzl_env_with_autoloads = this.modify_build_bzl_env(
            /* is_with_autoloads= */ true,
            original_build_bzl_env,
            &new_symbols,
        );
        this.uninjected_build_bzl_env_without_autoloads = this.modify_build_bzl_env(
            /* is_with_autoloads= */ false,
            original_build_bzl_env,
            /* new_symbols= */ &StarlarkEnv::new(),
        );

        // Validate rdeps - this ensures that all the rules using a provider are also removed
        // Check what's still available in Bazel (some symbols might already be deleted)
        let mut all_available_symbols: IndexSet<String> = this
            .uninjected_build_bzl_env_without_autoloads
            .keys()
            .cloned()
            .collect();
        all_available_symbols.extend(
            struct_fields(&this.uninjected_build_bzl_env_without_autoloads["native"].value)
                .into_keys(),
        );
        if this
            .partially_removed_symbols
            .iter()
            .any(|s| s == "ProtoInfo")
        {
            // special case, because only partially removed
            if !this
                .removed_symbols
                .iter()
                .any(|s| s == "proto_common_do_not_use")
            {
                panic!(
                    "IllegalStateException: Symbol in 'ProtoInfo' can't be removed, because it's \
                     still used by: 'proto_common_do_not_use'"
                );
            }
        }
        for symbol in &this.partially_removed_symbols {
            // Java: AUTOLOAD_CONFIG.get(symbol).rdeps() is an ImmutableSet (first occurrence
            // order, duplicates dropped).
            let rdeps: IndexSet<&str> = autoload_config(symbol)
                .expect("AUTOLOAD_CONFIG")
                .rdeps
                .iter()
                .copied()
                .collect();
            let unsatisfied_rdeps: Vec<&str> = rdeps
                .into_iter()
                .filter(|rdep| all_available_symbols.contains(*rdep))
                .collect();
            if !unsatisfied_rdeps.is_empty() {
                panic!(
                    "IllegalStateException: Symbol '{}' can't be removed, because it's still \
                     used by: {}",
                    symbol,
                    unsatisfied_rdeps.join(", ")
                );
            }
        }
        this
    }

    /// An optimisation, checking is autoloads are used at all.
    // Java: com/google/devtools/build/lib/packages/AutoloadSymbols.java:222 AutoloadSymbols.isEnabled
    pub fn is_enabled(&self) -> bool {
        self.autoloads_enabled
    }

    /// Returns the environment for BzlCompile function.
    // Java: com/google/devtools/build/lib/packages/AutoloadSymbols.java:227 AutoloadSymbols.getUninjectedBuildBzlEnv
    pub fn get_uninjected_build_bzl_env(
        &self,
        repository: Option<&RepositoryName>,
    ) -> &StarlarkEnv {
        if self.autoloads_disabled_in_bzl_for_repo(repository) {
            &self.uninjected_build_bzl_env_without_autoloads
        } else {
            &self.uninjected_build_bzl_env_with_autoloads
        }
    }

    /// Check if autoloads shouldn't be used for specific repository in bzl files.
    // Java: com/google/devtools/build/lib/packages/AutoloadSymbols.java:235 AutoloadSymbols.autoloadsDisabledInBzlForRepo
    pub fn autoloads_disabled_in_bzl_for_repo(&self, repository: Option<&RepositoryName>) -> bool {
        if !self.autoloads_enabled {
            return true;
        }
        if self.disable_autoloads_in_main_repo && repository.is_some_and(|r| r.is_main()) {
            return true;
        }
        match repository {
            None => true,
            Some(r) => self.autoloads_disabled_for_repo(r.get_name()),
        }
    }

    /// Check if autoloads shouldn't be used for specific repository in BUILD files.
    // Java: com/google/devtools/build/lib/packages/AutoloadSymbols.java:246 AutoloadSymbols.autoloadsDisabledInBuildForRepo
    pub fn autoloads_disabled_in_build_for_repo(&self, repository: &RepositoryName) -> bool {
        if !self.autoloads_enabled {
            return true;
        }
        if self.disable_autoloads_in_main_repo && repository.is_main() {
            return true;
        }
        false
    }

    /// Check if autoloads shouldn't be used in the given repository.
    ///
    /// Autoloads aren't used for repos in [`PREDECLARED_REPOS_DISALLOWING_AUTOLOADS`], specified
    /// by the --repositories_without_autoloads flag or any of their immediate descendants
    /// (parsing the cannonical repository name to check this).
    // Java: com/google/devtools/build/lib/packages/AutoloadSymbols.java:263 AutoloadSymbols.autoloadsDisabledForRepo
    pub fn autoloads_disabled_for_repo(&self, repo: &str) -> bool {
        if !self.autoloads_enabled {
            return true;
        }
        let separator_index = if repo.contains('~') {
            repo.find('~')
        } else {
            repo.find('+')
        };
        self.repos_disallowing_autoloads
            .contains(match separator_index {
                Some(i) => &repo[..i],
                None => repo,
            })
    }

    /// Modifies the environment for BzlLoad function (returned from StarlarkBuiltinsFunction).
    ///
    /// `original_env` contains original environment and `new_symbols` is a map from new symbol
    /// name to symbol's value. `is_with_autoloads` chooses the semantics, described in details
    /// on --incompatible_autoload_externally flag.
    // Java: com/google/devtools/build/lib/packages/AutoloadSymbols.java:279 AutoloadSymbols.modifyBuildBzlEnv
    pub fn modify_build_bzl_env(
        &self,
        is_with_autoloads: bool,
        original_env: &StarlarkEnv,
        new_symbols: &StarlarkEnv,
    ) -> StarlarkEnv {
        if is_with_autoloads {
            Self::modify_build_bzl_env_internal(
                original_env,
                /* add= */ new_symbols,
                /* remove= */ &self.removed_symbols,
                is_with_autoloads,
            )
        } else {
            Self::modify_build_bzl_env_internal(
                original_env,
                /* add= */ &StarlarkEnv::new(),
                /* remove= */ &self.partially_removed_symbols,
                is_with_autoloads,
            )
        }
    }

    /// Creates modified environment that's used in BzlCompileFunction and
    /// StarlarkBuiltinsFunction. It starts with the original environment. Adds the symbols to it
    /// or removes them.
    // Java: com/google/devtools/build/lib/packages/AutoloadSymbols.java:298 AutoloadSymbols.modifyBuildBzlEnv
    fn modify_build_bzl_env_internal(
        original_env: &StarlarkEnv,
        add: &StarlarkEnv,
        remove: &[String],
        is_with_autoloads: bool,
    ) -> StarlarkEnv {
        let mut env_builder = original_env.clone();
        let native = env_builder
            .shift_remove("native")
            .expect("the .bzl environment has 'native'");
        let mut native_bindings = struct_fields(&native.value);

        for (name, value) in add {
            if autoload_config(name).expect("AUTOLOAD_CONFIG").rule {
                native_bindings.insert(name.clone(), value.clone());
            } else {
                env_builder.insert(name.clone(), value.clone());
            }
        }
        for symbol in remove {
            if autoload_config(symbol).expect("AUTOLOAD_CONFIG").rule {
                native_bindings.shift_remove(symbol);
            } else if symbol == "proto_common_do_not_use" {
                let current = env_builder.get("proto_common_do_not_use").filter(|e| {
                    e.guard.is_none() && StarlarkInfo::from_value(e.value.value()).is_some()
                });
                if let Some(current) = current {
                    // proto_common_do_not_use can't be completely removed, because the
                    // implementation of proto rules in protobuf still relies on
                    // INCOMPATIBLE_ENABLE_PROTO_TOOLCHAIN_RESOLUTION, that reads the build
                    // language flag.
                    let flag = struct_fields(&current.value)
                        .shift_remove("INCOMPATIBLE_ENABLE_PROTO_TOOLCHAIN_RESOLUTION")
                        .unwrap_or_else(|| {
                            panic!(
                                "NullPointerException: \
                                 proto_common_do_not_use.INCOMPATIBLE_ENABLE_PROTO_TOOLCHAIN_RESOLUTION"
                            )
                        });
                    let mut fields = StarlarkEnv::new();
                    fields.insert(
                        "INCOMPATIBLE_ENABLE_PROTO_TOOLCHAIN_RESOLUTION".to_owned(),
                        flag,
                    );
                    env_builder.insert(
                        "proto_common_do_not_use".to_owned(),
                        EnvEntry::plain(create_struct_with_message(
                            &fields,
                            "no native symbol '%s'",
                        )),
                    );
                }
                // else GuardedValue used in BzlCompileFunction (where we need to keep the
                // symbol, but the value doesn't matter)
            } else {
                env_builder.shift_remove(symbol);
            }
        }

        if !is_with_autoloads {
            // In the repositories that don't have autoloads we also expose
            // native.legacy_globals. Those can be used to fallback to the native symbol, whenever
            // it's still available in Bazel. Fallback using a top-level symbol doesn't work,
            // because BzlCompileFunction would throw an error when it's mentioned.
            // legacy_globals aren't available when autoloads are not enabled. The feature is
            // intended to be use with bazel_features repository, which can correctly report
            // native symbols on all versions of Bazel.
            let legacy_symbols: StarlarkEnv = env_builder
                .iter()
                .filter(|(name, _)| autoload_config(name).is_some())
                // Drop GuardedValue - it doesn't work on non-toplevel symbols
                .map(|(name, e)| (name.clone(), EnvEntry::plain(e.value.clone())))
                .collect();
            native_bindings.insert(
                "legacy_globals".to_owned(),
                EnvEntry::plain(create_struct_with_message(
                    &legacy_symbols,
                    "no native symbol '%s'",
                )),
            );
        }

        env_builder.insert(
            "native".to_owned(),
            EnvEntry::plain(create_struct_with_message(
                &native_bindings,
                "no native function or rule '%s'",
            )),
        );
        env_builder
    }

    /// Modifies the environment for Package function (returned from StarlarkBuiltinsFunction).
    // Java: com/google/devtools/build/lib/packages/AutoloadSymbols.java:372 AutoloadSymbols.modifyBuildEnv
    pub fn modify_build_env(
        &self,
        is_with_autoloads: bool,
        original_env: &StarlarkEnv,
        new_symbols: &StarlarkEnv,
    ) -> StarlarkEnv {
        let empty = StarlarkEnv::new();
        let (add, remove) = if is_with_autoloads {
            (new_symbols, &self.removed_symbols)
        } else {
            (&empty, &self.partially_removed_symbols)
        };
        let mut env_builder = original_env.clone();
        for (name, value) in add {
            if autoload_config(name).expect("AUTOLOAD_CONFIG").rule {
                env_builder.insert(name.clone(), value.clone());
            }
        }
        for symbol in remove {
            if autoload_config(symbol).expect("AUTOLOAD_CONFIG").rule {
                env_builder.shift_remove(symbol);
            }
        }
        env_builder
    }

    // Java: com/google/devtools/build/lib/packages/AutoloadSymbols.java:392 AutoloadSymbols.filterSymbols
    fn filter_symbols(symbols: &[String], when: impl Fn(&str) -> bool) -> Vec<String> {
        symbols
            .iter()
            .filter(|s| when(s))
            .map(|s| without_prefix(s).to_owned())
            .collect()
    }

    // Java: com/google/devtools/build/lib/packages/AutoloadSymbols.java:402 AutoloadSymbols.getAllSymbols
    fn get_all_symbols(repository: &str, prefix: &str) -> Vec<String> {
        AUTOLOAD_CONFIG
            .iter()
            .filter(|(_, redirect)| redirect.load_label.starts_with(&format!("{repository}//")))
            .map(|(name, _)| format!("{prefix}{name}"))
            .collect()
    }

    /// Returns the extra .bzl files that need to be loaded, by symbol (keys from
    /// [`AUTOLOAD_CONFIG`]).
    ///
    /// PORT: Java requests `BazelDepGraphValue` here (returning null when missing) and returns
    /// `BzlLoadValue.keyForBuild(label)` keys; the caller requests the value and makes the keys.
    // Java: com/google/devtools/build/lib/packages/AutoloadSymbols.java:425 AutoloadSymbols.getLoadKeys
    pub fn get_load_keys(
        &self,
        bazel_dep_graph_value: &BazelDepGraphValue,
    ) -> IndexMap<String, Label> {
        let lookup = &bazel_dep_graph_value.canonical_repo_name_lookup;
        let mut highest_version_repo: IndexMap<String, ModuleKey> = IndexMap::new();
        for module_key in lookup.values() {
            let repo_name = SymbolRedirect::to_repo_name(module_key);
            match highest_version_repo.get_mut(&repo_name) {
                // (m1, m2) -> m1.version().compareTo(m2.version()) >= 0 ? m1 : m2
                Some(m1) => {
                    if m1.version() < module_key.version() {
                        *m1 = module_key.clone();
                    }
                }
                None => {
                    highest_version_repo.insert(repo_name, module_key.clone());
                }
            }
        }
        let repository_mapping = RepositoryMapping::create(
            highest_version_repo.iter().map(|(repo_name, module_key)| {
                (
                    repo_name.clone(),
                    lookup
                        .iter()
                        .find(|(_, k)| *k == module_key)
                        .map(|(name, _)| name.clone())
                        .expect("the inverse of canonicalRepoNameLookup"),
                )
            }),
            blaze_cmdline::repository_name::MAIN.clone(),
        );
        let repo_context = LabelRepoContextImpl::of(
            blaze_cmdline::repository_name::MAIN.clone(),
            repository_mapping,
        );

        // Inject loads for rules and symbols removed from Bazel
        let mut load_keys = IndexMap::new();
        for symbol in &self.autoloaded_symbols {
            let redirect = autoload_config(symbol).expect("AUTOLOAD_CONFIG");
            // Skip if version doesn't have the rules
            if let (Some(module_key), Some(required)) = (
                highest_version_repo.get(&redirect.get_repo_name()),
                required_version_for_module(&redirect.get_module_name()),
            ) && *module_key.version() <= required
            {
                continue;
            }

            let label = redirect.get_label(&repo_context);

            let repository_exists = label.get_repository().is_visible();
            // Only load if the dependency is present
            if repository_exists {
                load_keys.insert(symbol.clone(), label);
            }
        }
        load_keys
    }

    /// Processes the loaded autoload .bzl files into a map of symbols.
    ///
    /// `autoload_values` corresponds to the map [`Self::get_load_keys`] returned: for each
    /// symbol, a lookup of a global of its loaded module (Java `v.getModule().getGlobal(name)`).
    // Java: com/google/devtools/build/lib/packages/AutoloadSymbols.java:501 AutoloadSymbols.processLoads
    pub fn process_loads(
        &self,
        autoload_values: &IndexMap<String, &dyn Fn(&str) -> Option<OwnedFrozenValue>>,
    ) -> Result<StarlarkEnv, AutoloadException> {
        if self.autoloaded_symbols.is_empty() {
            return Ok(StarlarkEnv::new());
        }

        let mut new_symbols = StarlarkEnv::new();
        for (symbol, get_global) in autoload_values {
            // Check if the symbol is named differently in the bzl file than natively. Renames
            // are rare: Example is renaming native.ProguardSpecProvider to ProguardSpecInfo.
            let redirect = autoload_config(symbol).expect("AUTOLOAD_CONFIG");
            let new_name = redirect.new_name.unwrap_or(symbol);
            let Some(symbol_value) = get_global(new_name) else {
                return Err(AutoloadException(format!(
                    "The toplevel symbol '{}' set by --incompatible_load_symbols_externally \
                     couldn't be loaded. '{}' not found in auto loaded '{}'.",
                    symbol, new_name, redirect.load_label
                )));
            };
            new_symbols.insert(symbol.clone(), EnvEntry::plain(symbol_value)); // Exposed as old name
        }
        // Add also symbols that couldn't be autoloaded (missing dependency on the repository)
        // This way we get consistent behaviour for symbols that are still part of Bazel
        for symbol in &self.autoloaded_symbols {
            if !autoload_values.contains_key(symbol) {
                let redirect = autoload_config(symbol).expect("AUTOLOAD_CONFIG");
                let missing = MissingAutoload {
                    name: symbol.clone(),
                    load_label: redirect.load_label,
                    module_name: redirect.get_module_name(),
                };
                new_symbols.insert(
                    symbol.clone(),
                    EnvEntry::plain(freeze_value(move |m| m.heap().alloc(missing))),
                );
            }
        }
        Ok(new_symbols)
    }
}

impl PartialEq for AutoloadSymbols {
    // Java: com/google/devtools/build/lib/packages/AutoloadSymbols.java:570 AutoloadSymbols.equals
    fn eq(&self, other: &Self) -> bool {
        // These fields are used to generate all other private fields.
        // Thus, other fields don't need to be included in comparison.
        self.autoloaded_symbols == other.autoloaded_symbols
            && self.removed_symbols == other.removed_symbols
            && self.partially_removed_symbols == other.partially_removed_symbols
            && self.repos_disallowing_autoloads == other.repos_disallowing_autoloads
            && self.disable_autoloads_in_main_repo == other.disable_autoloads_in_main_repo
    }
}

/// Indicates a problem performing automatic loads.
// Java: com/google/devtools/build/lib/packages/AutoloadSymbols.java:621 AutoloadSymbols.AutoloadException
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct AutoloadException(pub String);

/// The callable `processLoads` puts for a symbol whose repository is not a dependency: calling
/// it fails (Java: an anonymous `StarlarkCallable`).
#[derive(Debug, starlark::any::ProvidesStaticType, NoSerialize, Allocative)]
struct MissingAutoload {
    name: String,
    load_label: &'static str,
    module_name: String,
}

starlark_simple_value!(MissingAutoload);

impl fmt::Display for MissingAutoload {
    // Java: net/starlark/java/eval/StarlarkCallable.java StarlarkCallable.repr (`<function NAME>`)
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<function {}>", self.name)
    }
}

// PORT: Java's anonymous class has no @StarlarkBuiltin; starlark-rust needs a type name.
#[starlark_value(type = "function")]
impl<'v> StarlarkValue<'v> for MissingAutoload {
    // Java: com/google/devtools/build/lib/packages/AutoloadSymbols.java:540 AutoloadSymbols.processLoads (StarlarkCallable.call)
    fn invoke(
        &self,
        _me: Value<'v>,
        _args: &Arguments<'v, '_>,
        _eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        let what = format!(
            "a 'bazel_dep(name = \"{}\", ...)' in your MODULE.bazel file",
            self.module_name
        );
        Err(starlark::Error::new_other(anyhow::anyhow!(
            "Couldn't auto load '{}' from '{}'. Ensure that you have {} or add an explicit load \
             statement to your BUILD file.",
            self.name,
            self.load_label,
            what
        )))
    }
}
