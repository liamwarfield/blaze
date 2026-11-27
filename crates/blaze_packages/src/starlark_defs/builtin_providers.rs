//! Builtin (native) providers and objects of the .bzl environment.
//!
//! Java: packages/BuiltinProvider.java (repr, equality by provider class, key), the providers
//! that `StarlarkGlobalsImpl.getFixedBzlToplevels` and the rule sets put in the .bzl environment:
//! analysis/DefaultInfo.java (`DefaultInfo`), analysis/OutputGroupInfo.java
//! (`OutputGroupInfo`), analysis/RunEnvironmentInfo.java (`RunEnvironmentInfo`),
//! analysis/ActionsProvider.java (`Actions`), analysis/MaterializedDepsInfo.java
//! (`MaterializedDepsInfo`), analysis/PackageSpecificationProvider.java
//! (`PackageSpecificationInfo`), analysis/test/InstrumentedFilesInfo.java,
//! analysis/test/AnalysisFailureInfo.java, analysis/test/AnalysisTestResultInfo.java,
//! rules/proto/BazelProtoCommon.java (`proto_common_do_not_use`) and
//! analysis/BaseRuleClasses.java `createEmptySymbol` (the CcRules removed symbols).
//!
//! Every builtin provider value provides a [`BuiltinProviderId`] and a
//! `blaze_starlark::structs::ProviderInfo` through `StarlarkValue::provide`, so any file can
//! recognise it (Java `x instanceof BuiltinProvider`) without knowing its Rust type.
//!
//! PORT: Java's `Provider.Key` of a builtin provider compares the provider's Java class. Blaze
//! names that class by the provider's *value class* simple name (the `valueClass` argument of the
//! `BuiltinProvider` constructor), the convention `native_rules::builtin_provider_id` uses; the
//! value classes of all builtin providers are distinct (`struct`: StarlarkInfo, `Actions`:
//! StructImpl), and several provider classes are anonymous or share the simple name `Provider`.

use std::fmt;
use std::hash::Hash;

use blaze_cmdline::Label;
use blaze_starlark::Location;
use blaze_starlark::ThreadContext;
use blaze_starlark::structs::ProviderInfo;
use blaze_starlark::structs::StructProvider;
use starlark::any::ProvidesStaticType;
use starlark::environment::GlobalsBuilder;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::eval::Arguments;
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::Demand;
use starlark::values::FrozenValue;
use starlark::values::NoSerialize;
use starlark::values::StarlarkValue;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::list::AllocList;
use starlark::values::starlark_value;
use starlark_map::StarlarkHasher;

use crate::bzl_context::BazelModuleContext;
use crate::rule_class::ProviderKey;
use crate::rule_class::StarlarkProviderIdentifier;
use crate::starlark_defs::provider::StarlarkProviderBuilder;
use crate::starlark_defs::provider::StarlarkProviderGen;
use crate::starlark_defs::util::errorf;

/// The identity of a builtin provider: its Starlark name (Java `BuiltinProvider.name`, also its
/// printable name) and its provider class (see the module comment).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, ProvidesStaticType)]
pub struct BuiltinProviderId {
    pub name: &'static str,
    // Java: com/google/devtools/build/lib/packages/BuiltinProvider.java:50 BuiltinProvider.getValueClass
    pub value_class: &'static str,
}

impl BuiltinProviderId {
    /// Java `BuiltinProvider.getKey`.
    // Java: com/google/devtools/build/lib/packages/BuiltinProvider.java:80 BuiltinProvider.getKey
    pub fn key(&self) -> ProviderKey {
        ProviderKey::Builtin {
            name: self.name.to_owned(),
            provider_class: self.value_class.to_owned(),
        }
    }

    /// Java `BuiltinProvider.id`.
    // Java: com/google/devtools/build/lib/packages/BuiltinProvider.java:100 BuiltinProvider.id
    pub fn id(&self) -> StarlarkProviderIdentifier {
        StarlarkProviderIdentifier::for_key(self.key())
    }
}

/// A native provider constructor (the provider's `selfCall` method).
pub type NativeConstructor = for<'v> fn(
    &BuiltinProviderId,
    Value<'v>,
    &Arguments<'v, '_>,
    &mut Evaluator<'v, '_, '_>,
) -> starlark::Result<Value<'v>>;

/// What calling a builtin provider does.
#[derive(Clone, Copy)]
pub enum BuiltinConstructor {
    /// The constructor is not ported; the string is the Java method.
    Unported(&'static str),
    Native(NativeConstructor),
}

impl fmt::Debug for BuiltinConstructor {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BuiltinConstructor::Unported(m) => write!(f, "Unported({m})"),
            BuiltinConstructor::Native(_) => write!(f, "Native"),
        }
    }
}

/// The shared state and behaviour of builtin provider values (Java `BuiltinProvider`).
#[derive(Debug, Clone, Copy)]
pub struct BuiltinProviderData {
    pub id: BuiltinProviderId,
    pub constructor: BuiltinConstructor,
}

impl BuiltinProviderData {
    // Java: com/google/devtools/build/lib/packages/BuiltinProvider.java:44 BuiltinProvider.BuiltinProvider
    // PORT: skipped, a builtin provider has no location (`StarlarkValue::callable_location` is none, Location.BUILTIN): BuiltinProvider.getLocation
    // PORT: skipped, no provider of the Bazel binary implements it: BuiltinProvider.WithLegacyStarlarkName.getStarlarkName
    pub const fn new(
        name: &'static str,
        value_class: &'static str,
        constructor: BuiltinConstructor,
    ) -> Self {
        BuiltinProviderData {
            id: BuiltinProviderId { name, value_class },
            constructor,
        }
    }

    /// Java `BuiltinProvider.repr`: `<function NAME>`.
    // Java: com/google/devtools/build/lib/packages/BuiltinProvider.java:94 BuiltinProvider.repr
    pub fn fmt_repr(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // TODO(adonovan): change to '<provider name>'.
        write!(f, "<function {}>", self.id.name)
    }

    pub fn invoke<'v>(
        &self,
        me: Value<'v>,
        args: &Arguments<'v, '_>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        match self.constructor {
            BuiltinConstructor::Unported(m) => blaze_util::unported!(m),
            BuiltinConstructor::Native(f) => f(&self.id, me, args, eval),
        }
    }

    /// Java `BuiltinProvider.equals`: same provider class.
    // Java: com/google/devtools/build/lib/packages/BuiltinProvider.java:58 BuiltinProvider.equals
    pub fn equals(&self, other: Value) -> bool {
        builtin_provider_id(other).is_some_and(|o| o.value_class == self.id.value_class)
    }

    /// Java `BuiltinProvider.hashCode` (the class) and `checkHashable` (always hashable).
    // Java: com/google/devtools/build/lib/packages/BuiltinProvider.java:63 BuiltinProvider.hashCode
    // Java: com/google/devtools/build/lib/packages/BuiltinProvider.java:68 BuiltinProvider.checkHashable
    pub fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        self.id.value_class.hash(hasher);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/packages/BuiltinProvider.java:88 BuiltinProvider.getPrintableName
    // Java: com/google/devtools/build/lib/packages/BuiltinProvider.java:73 BuiltinProvider.isExported
    pub fn provide<'v>(&self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(self.id);
        demand.provide_value(ProviderInfo {
            printable_name: self.id.name.to_owned(),
            exported: true,
        });
    }
}

/// Defines a Starlark value type for builtin providers whose Starlark type name is `$type_name`
/// (Java `Starlark.type` of the provider class: `Provider` for most, the API name for a few).
macro_rules! builtin_provider_type {
    ($(#[$meta:meta])* $name:ident, $type_name:tt) => {
        $(#[$meta])*
        #[derive(
            Debug,
            starlark::any::ProvidesStaticType,
            starlark::values::NoSerialize,
            allocative::Allocative
        )]
        pub struct $name {
            #[allocative(skip)]
            pub provider: $crate::starlark_defs::builtin_providers::BuiltinProviderData,
        }

        starlark::starlark_simple_value!($name);

        impl std::fmt::Display for $name {
            fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
                self.provider.fmt_repr(f)
            }
        }

        #[starlark::values::starlark_value(type = $type_name)]
        impl<'v> starlark::values::StarlarkValue<'v> for $name {
            fn invoke(
                &self,
                me: starlark::values::Value<'v>,
                args: &starlark::eval::Arguments<'v, '_>,
                eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
            ) -> starlark::Result<starlark::values::Value<'v>> {
                self.provider.invoke(me, args, eval)
            }

            fn name_for_call_stack(&self, _me: starlark::values::Value<'v>) -> String {
                self.provider.id.name.to_owned()
            }

            fn equals(&self, other: starlark::values::Value<'v>) -> starlark::Result<bool> {
                Ok(self.provider.equals(other))
            }

            fn write_hash(
                &self,
                hasher: &mut starlark_map::StarlarkHasher,
            ) -> starlark::Result<()> {
                self.provider.write_hash(hasher)
            }

            fn provide(&'v self, demand: &mut starlark::values::Demand<'_, 'v>) {
                self.provider.provide(demand)
            }
        }
    };
}
pub(crate) use builtin_provider_type;

builtin_provider_type!(
    /// A builtin provider whose Starlark type is `Provider` (Java `ProviderApi`).
    BuiltinProvider,
    "Provider"
);

builtin_provider_type!(
    /// `Actions` (Java `ActionsProvider`, whose type is its API name `Actions`).
    ActionsProvider,
    "Actions"
);

impl BuiltinProvider {
    pub const fn new(provider: BuiltinProviderData) -> Self {
        BuiltinProvider { provider }
    }
}

/// The builtin provider id of `v`, if `v` is a builtin provider (Java
/// `x instanceof BuiltinProvider`), including `struct` (Java `StructProvider.STRUCT`).
pub fn builtin_provider_id(v: Value) -> Option<BuiltinProviderId> {
    if let Some(id) = v.request_value::<BuiltinProviderId>() {
        return Some(id);
    }
    if v.downcast_ref::<StructProvider>().is_some() {
        return Some(STRUCT);
    }
    None
}

/// Java `BuiltinProvider.getKey` of `v`, if `v` is a builtin provider.
pub fn builtin_provider_key(v: Value) -> Option<ProviderKey> {
    builtin_provider_id(v).map(|id| id.key())
}

// -------------------------------------------------------------------------------------------
// Provider identities.

/// Java `StructProvider.STRUCT` (owned by blaze_starlark).
// Java: com/google/devtools/build/lib/packages/StructProvider.java:34 StructProvider.<init>
pub const STRUCT: BuiltinProviderId = BuiltinProviderId {
    name: "struct",
    value_class: "StarlarkInfo",
};

// Java: com/google/devtools/build/lib/analysis/DefaultInfo.java:196 DefaultInfo.DefaultInfoProvider.<init>
pub const DEFAULT_INFO: BuiltinProviderId = BuiltinProviderId {
    name: "DefaultInfo",
    value_class: "DefaultInfo",
};

// Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:571 OutputGroupInfo.OutputGroupInfoProvider.<init>
pub const OUTPUT_GROUP_INFO: BuiltinProviderId = BuiltinProviderId {
    name: "OutputGroupInfo",
    value_class: "OutputGroupInfo",
};

// Java: com/google/devtools/build/lib/analysis/RunEnvironmentInfo.java:93 RunEnvironmentInfo.RunEnvironmentInfoProvider.<init>
pub const RUN_ENVIRONMENT_INFO: BuiltinProviderId = BuiltinProviderId {
    name: "RunEnvironmentInfo",
    value_class: "RunEnvironmentInfo",
};

// Java: com/google/devtools/build/lib/analysis/ActionsProvider.java:39 ActionsProvider.<init>
pub const ACTIONS: BuiltinProviderId = BuiltinProviderId {
    name: "Actions",
    value_class: "StructImpl",
};

// Java: com/google/devtools/build/lib/analysis/MaterializedDepsInfo.java:62 MaterializedDepsInfo.Provider.<init>
pub const MATERIALIZED_DEPS_INFO: BuiltinProviderId = BuiltinProviderId {
    name: "MaterializedDepsInfo",
    value_class: "MaterializedDepsInfo",
};

// Java: com/google/devtools/build/lib/analysis/PackageSpecificationProvider.java:42 PackageSpecificationProvider.PROVIDER
pub const PACKAGE_SPECIFICATION_INFO: BuiltinProviderId = BuiltinProviderId {
    name: "PackageSpecificationInfo",
    value_class: "PackageSpecificationProvider",
};

// Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesInfo.java:146 InstrumentedFilesInfo.InstrumentedFilesProvider.<init>
pub const INSTRUMENTED_FILES_INFO: BuiltinProviderId = BuiltinProviderId {
    name: "InstrumentedFilesInfo",
    value_class: "InstrumentedFilesInfo",
};

// Java: com/google/devtools/build/lib/analysis/test/AnalysisFailureInfo.java:84 AnalysisFailureInfo.AnalysisFailureInfoProvider.<init>
pub const ANALYSIS_FAILURE_INFO: BuiltinProviderId = BuiltinProviderId {
    name: "AnalysisFailureInfo",
    value_class: "AnalysisFailureInfo",
};

// Java: com/google/devtools/build/lib/analysis/test/AnalysisTestResultInfo.java:60 AnalysisTestResultInfo.TestResultInfoProvider.<init>
pub const ANALYSIS_TEST_RESULT_INFO: BuiltinProviderId = BuiltinProviderId {
    name: "AnalysisTestResultInfo",
    value_class: "AnalysisTestResultInfo",
};

// -------------------------------------------------------------------------------------------
// Provider values.

/// Java `DefaultInfo.PROVIDER`.
// Java: com/google/devtools/build/lib/analysis/DefaultInfo.java:34 DefaultInfo.PROVIDER
pub const fn default_info() -> BuiltinProvider {
    BuiltinProvider::new(BuiltinProviderData {
        id: DEFAULT_INFO,
        // PORT: needs Runfiles/Artifact (analysis).
        constructor: BuiltinConstructor::Unported("DefaultInfo.DefaultInfoProvider.constructor"),
    })
}

/// Java `OutputGroupInfo.STARLARK_CONSTRUCTOR`.
// Java: com/google/devtools/build/lib/analysis/OutputGroupInfo.java:79 OutputGroupInfo.STARLARK_CONSTRUCTOR
pub const fn output_group_info() -> BuiltinProvider {
    BuiltinProvider::new(BuiltinProviderData {
        id: OUTPUT_GROUP_INFO,
        constructor: BuiltinConstructor::Unported(
            "OutputGroupInfo.OutputGroupInfoProvider.constructor",
        ),
    })
}

/// Java `RunEnvironmentInfo.PROVIDER`.
// Java: com/google/devtools/build/lib/analysis/RunEnvironmentInfo.java:39 RunEnvironmentInfo.PROVIDER
pub const fn run_environment_info() -> BuiltinProvider {
    BuiltinProvider::new(BuiltinProviderData {
        id: RUN_ENVIRONMENT_INFO,
        constructor: BuiltinConstructor::Unported(
            "RunEnvironmentInfo.RunEnvironmentInfoProvider.constructor",
        ),
    })
}

/// Java `ActionsProvider.INSTANCE`.
// Java: com/google/devtools/build/lib/analysis/ActionsProvider.java:36 ActionsProvider.INSTANCE
pub const fn actions_provider() -> ActionsProvider {
    ActionsProvider {
        // Java ActionsProvider has no selfCall method: calling it fails in the interpreter.
        provider: BuiltinProviderData {
            id: ACTIONS,
            constructor: BuiltinConstructor::Native(not_callable),
        },
    }
}

/// Java `MaterializedDepsInfo.PROVIDER`.
// Java: com/google/devtools/build/lib/analysis/MaterializedDepsInfo.java:31 MaterializedDepsInfo.PROVIDER
pub const fn materialized_deps_info() -> BuiltinProvider {
    BuiltinProvider::new(BuiltinProviderData {
        id: MATERIALIZED_DEPS_INFO,
        constructor: BuiltinConstructor::Unported("MaterializedDepsInfo.Provider.constructor"),
    })
}

/// Java `PackageSpecificationProvider.PROVIDER`.
// Java: com/google/devtools/build/lib/analysis/PackageSpecificationProvider.java:42 PackageSpecificationProvider.PROVIDER
pub const fn package_specification_info() -> BuiltinProvider {
    BuiltinProvider::new(BuiltinProviderData {
        id: PACKAGE_SPECIFICATION_INFO,
        // Java: an anonymous BuiltinProvider without selfCall.
        constructor: BuiltinConstructor::Native(not_callable),
    })
}

/// Java `InstrumentedFilesInfo.STARLARK_CONSTRUCTOR`.
// Java: com/google/devtools/build/lib/analysis/test/InstrumentedFilesInfo.java:33 InstrumentedFilesInfo.STARLARK_CONSTRUCTOR
pub const fn instrumented_files_info() -> BuiltinProvider {
    BuiltinProvider::new(BuiltinProviderData {
        id: INSTRUMENTED_FILES_INFO,
        // Java: InstrumentedFilesProvider has no selfCall method.
        constructor: BuiltinConstructor::Native(not_callable),
    })
}

/// Java `AnalysisFailureInfo.STARLARK_CONSTRUCTOR`.
// Java: com/google/devtools/build/lib/analysis/test/AnalysisFailureInfo.java:31 AnalysisFailureInfo.STARLARK_CONSTRUCTOR
pub const fn analysis_failure_info() -> BuiltinProvider {
    BuiltinProvider::new(BuiltinProviderData {
        id: ANALYSIS_FAILURE_INFO,
        constructor: BuiltinConstructor::Unported(
            "AnalysisFailureInfo.AnalysisFailureInfoProvider.constructor",
        ),
    })
}

/// Java `AnalysisTestResultInfo.STARLARK_CONSTRUCTOR`.
// Java: com/google/devtools/build/lib/analysis/test/AnalysisTestResultInfo.java:28 AnalysisTestResultInfo.STARLARK_CONSTRUCTOR
pub const fn analysis_test_result_info() -> BuiltinProvider {
    BuiltinProvider::new(BuiltinProviderData {
        id: ANALYSIS_TEST_RESULT_INFO,
        constructor: BuiltinConstructor::Unported(
            "AnalysisTestResultInfo.TestResultInfoProvider.testResultInfo",
        ),
    })
}

/// A provider without a `selfCall` method: Java's interpreter rejects the call
/// (`Starlark.call` -> "'%s' object is not callable").
// Java: net/starlark/java/eval/Starlark.java:747 Starlark.call
pub(crate) fn not_callable<'v>(
    id: &BuiltinProviderId,
    me: Value<'v>,
    _args: &Arguments<'v, '_>,
    _eval: &mut Evaluator<'v, '_, '_>,
) -> starlark::Result<Value<'v>> {
    let _ = id;
    Err(errorf(format!(
        "'{}' object is not callable",
        me.get_type()
    )))
}

// -------------------------------------------------------------------------------------------
// CcRules removed symbols.

/// The symbols `CcRules` registers with `BaseRuleClasses.createEmptySymbol(name, bzlLoadLabel)`
/// (name, load label), in registration order. They are never accessible: referencing one fails
/// at resolution with [`create_empty_symbol_error`].
///
/// PORT: Java registers them as `GuardedValue`s in the predeclared environment; the environment
/// composer (agent D, `builtins::env::Guard::Removed`) owns that mechanism, so they are not
/// registered by [`register_builtin_providers`].
// Java: com/google/devtools/build/lib/bazel/rules/CcRules.java:42 CcRules.init
pub const CC_RULES_EMPTY_SYMBOLS: &[(&str, &str)] = &[
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
];

/// Java `createEmptySymbol(...).getErrorFromAttemptingAccess(name)`.
// Java: com/google/devtools/build/lib/analysis/BaseRuleClasses.java:506 BaseRuleClasses.createEmptySymbol
pub fn create_empty_symbol_error(name: &str, bzl_load_label: &str) -> String {
    format!(
        "The {name} symbol has been removed, add the following to your BUILD/bzl file:\n\nload(\"{bzl_load_label}\", \"{name}\")\n"
    )
}

// -------------------------------------------------------------------------------------------
// proto_common_do_not_use

/// `proto_common_do_not_use` (Java `BazelProtoCommon.INSTANCE`, type `proto_common`).
#[derive(Debug, ProvidesStaticType, NoSerialize, allocative::Allocative)]
pub struct BazelProtoCommon;

starlark_simple_value!(BazelProtoCommon);

impl fmt::Display for BazelProtoCommon {
    // Java: StarlarkValue.repr default (BazelProtoCommon has no repr).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "<unknown object com.google.devtools.build.lib.rules.proto.BazelProtoCommon>"
        )
    }
}

// Java: com/google/devtools/build/lib/rules/proto/BazelProtoCommon.java:31 BazelProtoCommon
#[starlark_value(type = "proto_common")]
impl<'v> StarlarkValue<'v> for BazelProtoCommon {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("proto_common", bazel_proto_common_methods);
        Some(RES.methods())
    }

    fn write_hash(&self, _hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        // Java: StarlarkValue.checkHashable (isImmutable is false).
        Err(errorf("unhashable type: 'proto_common'"))
    }
}

/// Java `BuiltinRestriction.failIfCalledOutsideAllowlist(thread, ImmutableSet.of())`.
// PORT: move to packages/BuiltinRestriction when it is ported.
// Java: com/google/devtools/build/lib/packages/BuiltinRestriction.java:168 BuiltinRestriction.failIfCalledOutsideAllowlist
fn fail_if_called_outside_empty_allowlist(eval: &Evaluator) -> starlark::Result<()> {
    // Java: BazelModuleContext.ofInnermostBzlOrThrow (NullPointerException when absent).
    let module_context = BazelModuleContext::of_innermost_bzl(eval)
        .expect("NullPointerException: no BazelModuleContext for the innermost .bzl");
    let label = &module_context.label;
    // Java: BuiltinRestriction.isNotAllowed with an empty allowlist.
    if label.get_repository() == &*blaze_cmdline::repository_name::BUILTINS {
        return Ok(());
    }
    Err(errorf(format!(
        "file '{}' cannot use private API",
        label.get_canonical_form()
    )))
}

/// Java `ProtoConstants.EXTERNAL_PROTO_INFO_KEYS` labels.
// Java: com/google/devtools/build/lib/rules/proto/ProtoConstants.java:27 ProtoConstants.EXTERNAL_PROTO_INFO_KEYS
const EXTERNAL_PROTO_INFO_LABELS: [&str; 3] = [
    // WORKSPACE
    "@com_google_protobuf//bazel/private:proto_info.bzl",
    // tests
    "@com_google_protobuf+//bazel/private:proto_info.bzl",
    // bzlmod
    "@@protobuf+//bazel/private:proto_info.bzl",
];

#[starlark_module]
fn bazel_proto_common_methods(builder: &mut MethodsBuilder) {
    // Java: com/google/devtools/build/lib/rules/proto/BazelProtoCommon.java:45 BazelProtoCommon.getDefineProtoToolchains
    fn incompatible_enable_proto_toolchain_resolution<'v>(
        this: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<bool> {
        let _ = this;
        fail_if_called_outside_empty_allowlist(eval)?;
        let cx = ThreadContext::get(eval)?;
        Ok(cx.semantics.incompatible_enable_proto_toolchain_resolution)
    }

    // Java: com/google/devtools/build/lib/rules/proto/BazelProtoCommon.java:53 BazelProtoCommon.getExternalProtoInfos
    fn external_proto_infos<'v>(
        this: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        let _ = this;
        fail_if_called_outside_empty_allowlist(eval)?;
        // Java: a static immutable list of exported providers. The frozen heap makes the list
        // immutable.
        let heap = eval.frozen_heap();
        let providers: Vec<FrozenValue> = EXTERNAL_PROTO_INFO_LABELS
            .iter()
            .map(|l| {
                let provider: StarlarkProviderGen<FrozenValue> = StarlarkProviderBuilder::new(
                    Location::builtin(),
                )
                .build_exported(ProviderKey::Starlark {
                    extension_label: Label::parse_canonical_unchecked(l),
                    exported_name: "ProtoInfo".to_owned(),
                });
                heap.alloc(provider)
            })
            .collect();
        Ok(heap.alloc(AllocList(providers)).to_value())
    }
}

// -------------------------------------------------------------------------------------------
// Registration

/// The providers of `StarlarkGlobalsImpl.getFixedBzlToplevels`: `OutputGroupInfo`, `Actions`,
/// `DefaultInfo`, `RunEnvironmentInfo`, `MaterializedDepsInfo` (`struct` is blaze_starlark's).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkGlobalsImpl.java:101 StarlarkGlobalsImpl.getFixedBzlToplevels
#[starlark_module]
pub fn register_fixed_bzl_providers(builder: &mut GlobalsBuilder) {
    const OutputGroupInfo: BuiltinProvider = output_group_info();
    const Actions: ActionsProvider = actions_provider();
    const DefaultInfo: BuiltinProvider = default_info();
    const RunEnvironmentInfo: BuiltinProvider = run_environment_info();
    const MaterializedDepsInfo: BuiltinProvider = materialized_deps_info();
}

/// The providers of `TestingBootstrap`: `InstrumentedFilesInfo`, `AnalysisFailureInfo`,
/// `AnalysisTestResultInfo` (`testing` and `coverage_common` are in `testing.rs`).
// Java: com/google/devtools/build/lib/starlarkbuildapi/test/TestingBootstrap.java:48 TestingBootstrap.addBindingsToBuilder
#[starlark_module]
pub fn register_testing_providers(builder: &mut GlobalsBuilder) {
    const InstrumentedFilesInfo: BuiltinProvider = instrumented_files_info();
    const AnalysisFailureInfo: BuiltinProvider = analysis_failure_info();
    const AnalysisTestResultInfo: BuiltinProvider = analysis_test_result_info();
}

/// `PROTO_RULES`: `proto_common_do_not_use`.
// Java: com/google/devtools/build/lib/bazel/rules/BazelRuleClassProvider.java:258 BazelRuleClassProvider.PROTO_RULES
#[starlark_module]
pub fn register_proto_rules(builder: &mut GlobalsBuilder) {
    const proto_common_do_not_use: BazelProtoCommon = BazelProtoCommon;
}

/// `PACKAGING_RULES`: `PackageSpecificationInfo`.
// Java: com/google/devtools/build/lib/bazel/rules/BazelRuleClassProvider.java:323 BazelRuleClassProvider.PACKAGING_RULES
#[starlark_module]
pub fn register_packaging_rules(builder: &mut GlobalsBuilder) {
    const PackageSpecificationInfo: BuiltinProvider = package_specification_info();
}

/// Every builtin provider/object of this file: [`register_fixed_bzl_providers`],
/// [`register_testing_providers`], [`register_proto_rules`], [`register_packaging_rules`].
/// The CcRules removed symbols are not registered (see [`CC_RULES_EMPTY_SYMBOLS`]).
pub fn register_builtin_providers(builder: &mut GlobalsBuilder) {
    register_fixed_bzl_providers(builder);
    register_testing_providers(builder);
    register_proto_rules(builder);
    register_packaging_rules(builder);
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::starlark_defs::provider::test_support::exec_bzl;
    use crate::starlark_defs::provider::test_support::string_global;

    #[test]
    fn builtin_provider_type_repr_and_equality() {
        // Expected values recorded from the oracle (Bazel 9.3.2).
        let m = exec_bzl(
            "builtin_providers_test_1.bzl",
            r#"
def _p(x):
    return "%s|%s" % (type(x), repr(x))
result = ",".join([
    _p(DefaultInfo), _p(OutputGroupInfo), _p(RunEnvironmentInfo), _p(Actions),
    _p(MaterializedDepsInfo), _p(PackageSpecificationInfo), _p(InstrumentedFilesInfo),
    _p(AnalysisFailureInfo), _p(AnalysisTestResultInfo),
    str(DefaultInfo == DefaultInfo), str(DefaultInfo == OutputGroupInfo),
    str({DefaultInfo: 1}[DefaultInfo]),
])
"#,
        )
        .unwrap();
        assert_eq!(
            string_global(&m, "result"),
            "Provider|<function DefaultInfo>,Provider|<function OutputGroupInfo>,\
             Provider|<function RunEnvironmentInfo>,Actions|<function Actions>,\
             Provider|<function MaterializedDepsInfo>,\
             Provider|<function PackageSpecificationInfo>,\
             Provider|<function InstrumentedFilesInfo>,Provider|<function AnalysisFailureInfo>,\
             Provider|<function AnalysisTestResultInfo>,True,False,1"
        );
    }

    #[test]
    fn builtin_provider_keys() {
        assert_eq!(
            DEFAULT_INFO.key(),
            ProviderKey::Builtin {
                name: "DefaultInfo".to_owned(),
                provider_class: "DefaultInfo".to_owned()
            }
        );
        assert_eq!(DEFAULT_INFO.key().to_string(), "DefaultInfo");
        assert_ne!(ACTIONS.key(), STRUCT.key());
    }

    #[test]
    fn create_empty_symbol_error_text() {
        assert_eq!(
            create_empty_symbol_error("CcInfo", "@rules_cc//cc/common:cc_info.bzl"),
            "The CcInfo symbol has been removed, add the following to your BUILD/bzl file:\n\n\
             load(\"@rules_cc//cc/common:cc_info.bzl\", \"CcInfo\")\n"
        );
    }
}
