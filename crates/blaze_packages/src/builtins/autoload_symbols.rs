//! `--incompatible_autoload_externally`: which symbols Bazel loads from rules repositories.
//!
//! Java: com/google/devtools/build/lib/packages/AutoloadSymbols.java.
//!
//! PORT: only the disabled configuration is ported (Bazel 9.3.2's default: the flag is empty).
//! A non-empty `--incompatible_autoload_externally` fails loudly. With autoloads disabled Java
//! still rewrites the BUILD-loaded .bzl environment (`native.legacy_globals`), which is ported.

use blaze_cmdline::RepositoryName;
use blaze_starlark::StarlarkSemantics;

use crate::builtins::env::{EnvEntry, StarlarkEnv, create_struct, struct_fields};

/// Whether a symbol in [`AUTOLOAD_CONFIG`] is a rule (it lives in `native`) and where it is
/// loaded from.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SymbolRedirect {
    pub rule: bool,
    pub load_label: &'static str,
}

const fn rule_redirect(load_label: &'static str) -> SymbolRedirect {
    SymbolRedirect {
        rule: true,
        load_label,
    }
}

const fn symbol_redirect(load_label: &'static str) -> SymbolRedirect {
    SymbolRedirect {
        rule: false,
        load_label,
    }
}

/// The symbols that may be autoloaded, in Java's table order.
// Java: com/google/devtools/build/lib/packages/AutoloadSymbols.java:675 AutoloadSymbols.AUTOLOAD_CONFIG
pub const AUTOLOAD_CONFIG: &[(&str, SymbolRedirect)] = &[
    (
        "CcSharedLibraryInfo",
        symbol_redirect("@rules_cc//cc/common:cc_shared_library_info.bzl"),
    ),
    (
        "CcSharedLibraryHintInfo",
        symbol_redirect("@rules_cc//cc/common:cc_shared_library_hint_info.bzl"),
    ),
    (
        "cc_proto_aspect",
        symbol_redirect("@com_google_protobuf//bazel/private:bazel_cc_proto_library.bzl"),
    ),
    (
        "ProtoInfo",
        symbol_redirect("@com_google_protobuf//bazel/common:proto_info.bzl"),
    ),
    (
        "proto_common_do_not_use",
        symbol_redirect("@com_google_protobuf//:dummy.bzl"),
    ),
    (
        "cc_common",
        symbol_redirect("@rules_cc//cc/common:cc_common.bzl"),
    ),
    (
        "CcInfo",
        symbol_redirect("@rules_cc//cc/common:cc_info.bzl"),
    ),
    (
        "DebugPackageInfo",
        symbol_redirect("@rules_cc//cc/common:debug_package_info.bzl"),
    ),
    (
        "CcToolchainConfigInfo",
        symbol_redirect("@rules_cc//cc/toolchains:cc_toolchain_config_info.bzl"),
    ),
    (
        "java_common",
        symbol_redirect("@rules_java//java/common:java_common.bzl"),
    ),
    (
        "JavaInfo",
        symbol_redirect("@rules_java//java/common:java_info.bzl"),
    ),
    (
        "JavaPluginInfo",
        symbol_redirect("@rules_java//java/common:java_plugin_info.bzl"),
    ),
    (
        "ProguardSpecProvider",
        symbol_redirect("@rules_java//java/common:proguard_spec_info.bzl"),
    ),
    (
        "PyInfo",
        symbol_redirect("@rules_python//python:py_info.bzl"),
    ),
    (
        "PyRuntimeInfo",
        symbol_redirect("@rules_python//python:py_runtime_info.bzl"),
    ),
    (
        "PyCcLinkParamsProvider",
        symbol_redirect("@rules_python//python:py_cc_link_params_info.bzl"),
    ),
    (
        "AndroidIdeInfo",
        symbol_redirect("@rules_android//providers:providers.bzl"),
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
        symbol_redirect("@build_bazel_apple_support//lib:apple_common.bzl"),
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
    /// If the list of symbols to autoload is empty, autoloads are disabled.
    autoloads_enabled: bool,
    /// Should the autoloads be disabled in root project (main repository).
    disable_autoloads_in_main_repo: bool,
    uninjected_build_bzl_env_with_autoloads: StarlarkEnv,
    uninjected_build_bzl_env_without_autoloads: StarlarkEnv,
    removed_symbols: Vec<String>,
    partially_removed_symbols: Vec<String>,
}

impl AutoloadSymbols {
    /// `original_build_bzl_env` is `BazelStarlarkEnvironment.getUninjectedBuildBzlEnv()`.
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
                autoloads_enabled,
                disable_autoloads_in_main_repo,
                uninjected_build_bzl_env_with_autoloads: original_build_bzl_env.clone(),
                uninjected_build_bzl_env_without_autoloads: original_build_bzl_env.clone(),
                removed_symbols: Vec::new(),
                partially_removed_symbols: Vec::new(),
            };
        }
        blaze_util::unported!(
            "AutoloadSymbols.<init>",
            "--incompatible_autoload_externally={}",
            symbol_configuration.join(",")
        )
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
    // Java: com/google/devtools/build/lib/packages/AutoloadSymbols.java:263 AutoloadSymbols.autoloadsDisabledForRepo
    pub fn autoloads_disabled_for_repo(&self, _repo: &str) -> bool {
        if !self.autoloads_enabled {
            return true;
        }
        blaze_util::unported!("AutoloadSymbols.autoloadsDisabledForRepo (autoloads enabled)")
    }

    /// Modifies the environment for BzlLoad function (returned from StarlarkBuiltinsFunction).
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
                new_symbols,
                &self.removed_symbols,
                is_with_autoloads,
            )
        } else {
            Self::modify_build_bzl_env_internal(
                original_env,
                &StarlarkEnv::new(),
                &self.partially_removed_symbols,
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
                blaze_util::unported!(
                    "AutoloadSymbols.modifyBuildBzlEnv (removing proto_common_do_not_use)"
                );
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
                EnvEntry::plain(create_struct(&legacy_symbols)),
            );
        }

        env_builder.insert(
            "native".to_owned(),
            EnvEntry::plain(create_struct(&native_bindings)),
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

    /// Processes the loaded autoload .bzl values into the symbols they define. Only the empty
    /// case (autoloads disabled) is ported.
    // Java: com/google/devtools/build/lib/packages/AutoloadSymbols.java:501 AutoloadSymbols.processLoads
    pub fn process_loads(&self, loaded_symbols_empty: bool) -> StarlarkEnv {
        if loaded_symbols_empty && !self.autoloads_enabled {
            return StarlarkEnv::new();
        }
        blaze_util::unported!("AutoloadSymbols.processLoads")
    }
}
