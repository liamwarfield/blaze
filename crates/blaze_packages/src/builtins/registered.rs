//! What Bazel's rule sets register on the `ConfiguredRuleClassProvider` besides rule classes:
//! additional .bzl toplevels (`addBzlToplevel`, `addStarlarkBootstrap`), `@_builtins` internals
//! (`addStarlarkBuiltinsInternal`) and BUILD toplevels (`addBuildFileToplevel`), in rule-set order
//! (Java `BazelRuleClassProvider.RULE_SETS`, then `BazelRepositoryModule`).
//!
//! The bootstrap objects ported in `starlark_defs` (ConfigBootstrap, PlatformBootstrap,
//! TestingBootstrap, `proto_common_do_not_use`, `PackageSpecificationInfo`) are registered here
//! through `starlark_defs::register_rule_class_provider_bzl_toplevels`: like every
//! `addBzlToplevel` / `addStarlarkBootstrap` symbol they may be overridden by `@_builtins`.

use std::fmt;

use allocative::Allocative;
use blaze_cmdline::PackageIdentifier;
use blaze_starlark::StarlarkSemantics;
use starlark::any::ProvidesStaticType;
use starlark::environment::GlobalsBuilder;
use starlark::eval::Arguments;
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::{NoSerialize, StarlarkValue, Value, starlark_value};

use crate::builtins::env::{EnvEntry, Guard, StarlarkEnv, freeze_value};
use crate::builtins::stubs;

/// The registration data of the rule class provider (Java `ConfiguredRuleClassProvider` fields
/// `registeredBuildFileToplevels`, `registeredBzlToplevels`, `starlarkBuiltinsInternals`).
#[derive(Clone)]
pub struct RuleClassProviderRegistrations {
    pub registered_build_file_toplevels: StarlarkEnv,
    pub registered_bzl_toplevels: StarlarkEnv,
    pub builtins_internals: StarlarkEnv,
}

fn pkgs(entries: &[(&str, &str)]) -> Vec<PackageIdentifier> {
    entries
        .iter()
        .map(|(repo, pkg)| PackageIdentifier::create_unchecked(repo, pkg))
        .collect()
}

fn none() -> starlark::values::OwnedFrozenValue {
    freeze_value(|_| Value::new_none())
}

fn stop_exporting_language_modules(s: &StarlarkSemantics) -> bool {
    s.incompatible_stop_exporting_language_modules
}

impl RuleClassProviderRegistrations {
    /// Bazel 9.3.2's registrations.
    pub fn bazel() -> RuleClassProviderRegistrations {
        let build_file = StarlarkEnv::new();
        let mut bzl = StarlarkEnv::new();
        let mut internals = StarlarkEnv::new();

        // BAZEL_SETUP
        // Java: com/google/devtools/build/lib/bazel/rules/BazelRuleClassProvider.java:240
        internals.insert(
            "cc_internal".to_owned(),
            EnvEntry::plain(freeze_value(|m| {
                m.heap().alloc(crate::builtins::cc_internal::cc_internal())
            })),
        );
        // PORT: `package()` (Java PackageCallable, an addBuildFileToplevel) is registered by
        // agent C's build::register_build_globals; Java also exposes it as `native.package`.

        // GenericRules
        // Java: com/google/devtools/build/lib/bazel/rules/GenericRules.java:57
        bzl.insert(
            "_builtins_dummy".to_owned(),
            EnvEntry::guarded(
                freeze_value(|m| m.heap().alloc("original value")),
                Guard::WhenExperimentalFlagIsTrue {
                    flag: "experimental_builtins_dummy",
                    get: |s| s.experimental_builtins_dummy,
                },
            ),
        );

        // ConfigRules, PlatformRules, TestingSupportRules bootstraps; PROTO_RULES
        // (`proto_common_do_not_use`); PACKAGING_RULES (`PackageSpecificationInfo`).
        // Java: com/google/devtools/build/lib/bazel/rules/BazelRuleClassProvider.java:258
        for (name, entry) in crate::builtins::env::env_from_registration(
            &crate::starlark_defs::register_rule_class_provider_bzl_toplevels,
        ) {
            bzl.insert(name, entry);
        }

        // CcRules
        // Java: com/google/devtools/build/lib/bazel/rules/CcRules.java:42
        for (name, label) in [
            ("CcInfo", "@rules_cc//cc/common:cc_info.bzl"),
            (
                "DebugPackageInfo",
                "@rules_cc//cc/common:debug_package_info.bzl",
            ),
            (
                "CcSharedLibraryInfo",
                "@rules_cc//cc/common:cc_shared_library_info.bzl",
            ),
            (
                "CcSharedLibraryHintInfo",
                "@rules_cc//cc/common:cc_shared_library_hint_info.bzl",
            ),
        ] {
            bzl.insert(
                name.to_owned(),
                EnvEntry::guarded(
                    none(),
                    Guard::Removed {
                        bzl_load_label: label,
                    },
                ),
            );
        }
        // Java: com/google/devtools/build/lib/bazel/rules/CcRules.java:70
        internals.insert(
            "cc_common".to_owned(),
            EnvEntry::plain(freeze_value(|m| m.heap().alloc(stubs::cc_common()))),
        );
        // CcBootstrap
        // Java: com/google/devtools/build/lib/starlarkbuildapi/cpp/CcBootstrap.java:50 CcBootstrap.addBindingsToBuilder
        // PORT: skipped, the constructor takes the cc module and keeps nothing (an empty body at the pin): CcBootstrap.<init>
        let cc_allowed = pkgs(&[
            ("_builtins", ""),
            ("bazel_tools", ""),
            ("local_config_cc", ""),
            ("rules_cc", ""),
            ("", "tools/build_defs/cc"),
        ]);
        for name in ["cc_common", "CcToolchainConfigInfo"] {
            bzl.insert(
                name.to_owned(),
                EnvEntry::guarded(
                    none(),
                    Guard::InAllowedReposOrWhenIncompatibleFlagIsFalse {
                        flag: "incompatible_stop_exporting_language_modules",
                        get: stop_exporting_language_modules,
                        allowed_entries: cc_allowed.clone(),
                    },
                ),
            );
        }

        // JavaRules
        // Java: com/google/devtools/build/lib/bazel/rules/JavaRules.java:63
        bzl.insert("java_common".to_owned(), EnvEntry::plain(none()));
        internals.insert(
            "java_common_internal_do_not_use".to_owned(),
            EnvEntry::plain(freeze_value(|m| {
                m.heap()
                    .alloc(crate::builtins::java_common::java_common_internal())
            })),
        );

        // ANDROID_RULES
        // Java: com/google/devtools/build/lib/bazel/rules/BazelRuleClassProvider.java:278
        bzl.insert(
            "android_common".to_owned(),
            EnvEntry::guarded(
                freeze_value(|m| m.heap().alloc(stubs::android_common())),
                Guard::InAllowedReposOrWhenIncompatibleFlagIsFalse {
                    flag: "incompatible_stop_exporting_language_modules",
                    get: stop_exporting_language_modules,
                    allowed_entries: pkgs(&[("rules_android", "")]),
                },
            ),
        );

        // PYTHON_RULES
        // Java: com/google/devtools/build/lib/bazel/rules/BazelRuleClassProvider.java:307
        bzl.insert(
            "py_internal".to_owned(),
            EnvEntry::guarded(
                none(),
                Guard::InAllowedRepos {
                    allowed_entries: pkgs(&[
                        ("_builtins", ""),
                        ("bazel_tools", ""),
                        ("rules_python", ""),
                        ("", "tools/build_defs/python"),
                    ]),
                },
            ),
        );
        internals.insert(
            "py_builtins".to_owned(),
            EnvEntry::plain(freeze_value(|m| m.heap().alloc(stubs::py_builtins()))),
        );

        // ObjcRules
        // Java: com/google/devtools/build/lib/bazel/rules/ObjcRules.java:45
        internals.insert(
            "apple_common".to_owned(),
            EnvEntry::plain(freeze_value(|m| m.heap().alloc(stubs::apple_common()))),
        );
        // AppleBootstrap
        // Java: com/google/devtools/build/lib/starlarkbuildapi/objc/AppleBootstrap.java:48 AppleBootstrap.addBindingsToBuilder
        bzl.insert(
            "apple_common".to_owned(),
            EnvEntry::guarded(
                none(),
                Guard::InAllowedReposOrWhenIncompatibleFlagIsFalse {
                    flag: "incompatible_stop_exporting_language_modules",
                    get: stop_exporting_language_modules,
                    allowed_entries: pkgs(&[
                        ("_builtins", ""),
                        ("apple_support", ""),
                        ("bazel_tools", ""),
                        ("build_bazel_rules_apple", ""),
                        ("build_bazel_rules_swift", ""),
                        ("io_bazel_rules_go", ""),
                        ("local_config_cc", ""),
                        ("rules_apple", ""),
                        ("rules_cc", ""),
                        ("rules_go", ""),
                        ("rules_ios", ""),
                        ("rules_swift", ""),
                        ("stardoc", ""),
                        ("tulsi", ""),
                        ("", "test_starlark"),
                        ("", "tools/osx"),
                    ]),
                },
            ),
        );

        // BazelRepositoryModule: RepositoryBootstrap(new StarlarkRepositoryModule())
        // Java: com/google/devtools/build/lib/bazel/BazelRepositoryModule.java:272
        for (name, entry) in
            crate::builtins::env::env_from_registration(&register_repository_module)
        {
            bzl.insert(name, entry);
        }

        RuleClassProviderRegistrations {
            registered_build_file_toplevels: build_file,
            registered_bzl_toplevels: bzl,
            builtins_internals: internals,
        }
    }
}

/// A repository rule, module extension or tag class defined at .bzl top level.
///
/// PORT: Java `StarlarkRepositoryModule` (blaze_bzlmod, not ported yet) builds real
/// `RepositoryRuleFunction` / `ModuleExtension` / `TagClass` objects; loading only needs the
/// definitions to evaluate, so this placeholder records the kind and fails loudly when used.
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct RepositoryModuleObject {
    kind: &'static str,
}

starlark_simple_value!(RepositoryModuleObject);

impl fmt::Display for RepositoryModuleObject {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<{}>", self.kind)
    }
}

#[starlark_value(type = "repository_module_object")]
impl<'v> StarlarkValue<'v> for RepositoryModuleObject {
    fn invoke(
        &self,
        _me: Value<'v>,
        _args: &Arguments<'v, '_>,
        _eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        blaze_util::unported!(format!("StarlarkRepositoryModule ({} call)", self.kind))
    }
}

/// Java `StarlarkRepositoryModule`'s top-level functions.
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryModule.java StarlarkRepositoryModule
#[starlark_module]
pub fn register_repository_module(builder: &mut GlobalsBuilder) {
    fn repository_rule<'v>(
        #[starlark(args)] args: starlark::values::tuple::UnpackTuple<Value<'v>>,
        #[starlark(kwargs)] kwargs: Value<'v>,
    ) -> anyhow::Result<RepositoryModuleObject> {
        let _ = (args, kwargs);
        Ok(RepositoryModuleObject {
            kind: "repository_rule",
        })
    }

    fn module_extension<'v>(
        #[starlark(args)] args: starlark::values::tuple::UnpackTuple<Value<'v>>,
        #[starlark(kwargs)] kwargs: Value<'v>,
    ) -> anyhow::Result<RepositoryModuleObject> {
        let _ = (args, kwargs);
        Ok(RepositoryModuleObject {
            kind: "module_extension",
        })
    }

    fn tag_class<'v>(
        #[starlark(args)] args: starlark::values::tuple::UnpackTuple<Value<'v>>,
        #[starlark(kwargs)] kwargs: Value<'v>,
    ) -> anyhow::Result<RepositoryModuleObject> {
        let _ = (args, kwargs);
        Ok(RepositoryModuleObject { kind: "tag_class" })
    }
}
