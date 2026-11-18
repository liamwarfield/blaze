//! Golden test: the oracle's own C++ toolchain, the compile of brotli's `c/common/dictionary.c`.
//!
//! - tests/data/local_config_cc_k8.json: `json.encode` of the `CcToolchainConfigInfo` of
//!   `@@rules_cc++cc_configure_extension+local_config_cc//:local` (the `k8` toolchain rules_cc's
//!   cc_autoconf generated for this host), taken from the oracle with
//!   `scripts/cc-features-golden` (cquery --output=starlark).
//! - tests/data/oracle_brotli_dictionary_c_compile.json: the oracle's aquery record of the
//!   `CppCompile` action for `bazel-out/k8-fastbuild/bin/_objs/brotlicommon/dictionary.pic.o`.
//!
//! The requested features follow rules_cc 0.2.17's `configure_features.bzl` for a `cc_library` in
//! k8-fastbuild without coverage/FDO; the build variables follow `compile_build_variables.bzl` with
//! the values (paths, copts, include directories) from the recording.

mod common;

use std::sync::Arc;

use blaze_cc_features::actions::NOOP;
use blaze_cc_features::cc_starlark_internal::{
    cc_toolchain_variables, combine_cc_toolchain_variables, create_copts_filter,
    intern_string_sequence_variable_value,
};
use blaze_cc_features::cc_toolchain_variables::VariableObject;
use blaze_cc_features::compile_command_line::CompileCommandLine;
use blaze_cc_features::starlark_info::{JsonLike, StarlarkInfoValue};
use blaze_cc_features::{
    CcToolchainConfigInfo, CcToolchainFeatures, CcToolchainVariables, CppActionNames,
};
use blaze_collect::{NestedSetBuilder, Order};
use blaze_vfs::PathFragment;
use common::TestArtifact;
use indexmap::{IndexMap, IndexSet};

const DATA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data");

fn to_json_like(v: &serde_json::Value) -> JsonLike {
    match v {
        serde_json::Value::Null => JsonLike::Null,
        serde_json::Value::Bool(b) => JsonLike::Bool(*b),
        serde_json::Value::Number(n) => JsonLike::Int(n.as_i64().unwrap()),
        serde_json::Value::String(s) => JsonLike::String(s.clone()),
        serde_json::Value::Array(a) => JsonLike::Array(a.iter().map(to_json_like).collect()),
        serde_json::Value::Object(o) => JsonLike::Object(
            o.iter()
                .map(|(k, v)| (k.clone(), to_json_like(v)))
                .collect(),
        ),
    }
}

fn read_json(name: &str) -> serde_json::Value {
    serde_json::from_str(&std::fs::read_to_string(format!("{DATA}/{name}")).unwrap()).unwrap()
}

/// `cc_helper.package_exec_path` of `@local_config_cc//:cc-compiler-k8`'s package.
const TOOLS_DIRECTORY: &str = "external/rules_cc++cc_configure_extension+local_config_cc";

fn toolchain_features() -> CcToolchainFeatures {
    let info = match StarlarkInfoValue::from_json_like(&to_json_like(&read_json(
        "local_config_cc_k8.json",
    ))) {
        StarlarkInfoValue::Info(i) => i,
        other => panic!("{other:?}"),
    };
    CcToolchainFeatures::new(
        &CcToolchainConfigInfo::new(info),
        PathFragment::create(TOOLS_DIRECTORY),
    )
    .unwrap()
}

/// rules_cc's `configure_features(ctx, cc_toolchain, requested_features = ctx.features,
/// unsupported_features = ctx.disabled_features)` for `//:brotlicommon` in k8-fastbuild.
fn requested_features(features: &CcToolchainFeatures) -> IndexSet<String> {
    // DEFAULT_ACTION_CONFIGS = ALL_COMPILE_ACTIONS + ALL_LINK_ACTIONS + ALL_ARCHIVE_ACTIONS +
    // ALL_OTHER_ACTIONS (configure_features.bzl).
    let default_action_configs = [
        "c-compile",
        "c++-compile",
        "c++-header-parsing",
        "c++-module-compile",
        "c++-module-codegen",
        "c++-module-deps-scanning",
        "c++20-module-compile",
        "c++20-module-codegen",
        "assemble",
        "preprocess-assemble",
        "clif-match",
        "linkstamp-compile",
        "cc-flags-make-variable",
        "lto-backend",
        "c++-header-analysis",
        "lto-index-for-executable",
        "lto-index-for-dynamic-library",
        "lto-index-for-nodeps-dynamic-library",
        "c++-link-executable",
        "c++-link-dynamic-library",
        "c++-link-nodeps-dynamic-library",
        "c++-link-static-library",
        "strip",
    ];
    // The cc_toolchain has no module_map, so "module_maps" is unsupported; it supports header
    // parsing (supports_header_parsing = 1); brotli has no features/disabled features.
    let unsupported = ["module_maps"];
    let mut all_requested: IndexSet<String> = IndexSet::new();
    // apple_generate_dsym is false.
    all_requested.insert("no_generate_debug_symbols".to_string());
    let mut all_features: Vec<String> = vec!["fastbuild".to_string()];
    all_features.extend(default_action_configs.iter().map(|s| s.to_string()));
    all_features.extend(
        features
            .get_default_features_and_action_configs()
            .iter()
            .cloned(),
    );
    // --incompatible_dont_enable_host_nonhost_crosstool_features is true: no host/nonhost.
    for feature in all_features {
        if !unsupported.contains(&feature.as_str()) {
            all_requested.insert(feature);
        }
    }
    all_requested
}

#[test]
fn golden_brotli_dictionary_c_compile_command_line() {
    let oracle = read_json("oracle_brotli_dictionary_c_compile.json");
    let expected_argv: Vec<String> = oracle["argv"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_string())
        .collect();

    let features = toolchain_features();
    let feature_configuration = features
        .get_feature_configuration(&requested_features(&features))
        .unwrap();
    assert!(!feature_configuration.is_enabled("module_maps"));

    // cc_toolchain._build_variables: no sysroot, no minimum_os_version.
    let toolchain_vars = cc_toolchain_variables(&IndexMap::new());

    // setup_common_compile_build_variables (values from the recorded command line).
    let depset = |items: &[&str]| {
        VariableObject::NestedSet(NestedSetBuilder::create(
            Order::STABLE_ORDER,
            items.iter().map(|s| VariableObject::string(s)),
        ))
    };
    let mut common: IndexMap<String, VariableObject> = IndexMap::new();
    common.insert(
        "include_paths".into(),
        depset(&["bazel-out/k8-fastbuild/bin/_virtual_includes/brotli_inc"]),
    );
    common.insert(
        "quote_include_paths".into(),
        depset(&[".", "bazel-out/k8-fastbuild/bin"]),
    );
    common.insert("system_include_paths".into(), depset(&[]));
    common.insert("framework_include_paths".into(), depset(&[]));
    common.insert(
        "preprocessor_defines".into(),
        intern_string_sequence_variable_value(&[]),
    );
    let common_vars = cc_toolchain_variables(&common);
    let common_compile_build_variables =
        combine_cc_toolchain_variables(&toolchain_vars, &[common_vars]);

    // get_specific_compile_build_variables for the PIC compile of dictionary.c.
    let copts: Vec<String> = [
        "--pedantic-errors",
        "-Wall",
        "-Wconversion",
        "-Werror",
        "-Wextra",
        "-Wlong-long",
        "-Wmissing-declarations",
        "-Wmissing-prototypes",
        "-Wno-strict-aliasing",
        "-Wshadow",
        "-Wsign-compare",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();
    let mut specific: IndexMap<String, VariableObject> = IndexMap::new();
    specific.insert(
        "user_compile_flags".into(),
        intern_string_sequence_variable_value(&copts),
    );
    specific.insert(
        "source_file".into(),
        VariableObject::Artifact(TestArtifact::source("c/common/dictionary.c")),
    );
    specific.insert(
        "output_file".into(),
        VariableObject::Artifact(TestArtifact::derived(
            "bazel-out/k8-fastbuild/bin/_objs/brotlicommon/dictionary.pic.o",
        )),
    );
    specific.insert(
        "dependency_file".into(),
        VariableObject::Artifact(TestArtifact::derived(
            "bazel-out/k8-fastbuild/bin/_objs/brotlicommon/dictionary.pic.d",
        )),
    );
    specific.insert("pic".into(), VariableObject::string(""));
    let specific_vars = cc_toolchain_variables(&specific);
    let compile_variables =
        combine_cc_toolchain_variables(&common_compile_build_variables, &[specific_vars]);

    let compile_command_line = CompileCommandLine::builder(
        create_copts_filter(None).unwrap(),
        CppActionNames::C_COMPILE,
    )
    .set_feature_configuration(Arc::clone(&feature_configuration))
    .set_variables(compile_variables.clone())
    .build();
    // CppCompileAction.getOverwrittenVariables() without header modules.
    let overwritten = CcToolchainVariables::builder().build();
    let argv = compile_command_line
        .get_arguments(None, Some(&overwritten), &NOOP)
        .unwrap();

    if argv != expected_argv {
        let n = argv.len().max(expected_argv.len());
        let mut diff = String::new();
        for i in 0..n {
            let a = argv.get(i).map(|s| s.as_str()).unwrap_or("<none>");
            let e = expected_argv.get(i).map(|s| s.as_str()).unwrap_or("<none>");
            diff.push_str(&format!(
                "{}{i}: blaze={a} oracle={e}\n",
                if a == e { " " } else { "*" }
            ));
        }
        panic!("argv mismatch:\n{diff}");
    }

    // The toolchain sets no env for c-compile: the recorded env (PATH, PWD) comes from the action
    // environment and CppCompileAction.getEffectiveEnvironment, not from features.
    assert!(
        compile_command_line
            .get_environment(&NOOP)
            .unwrap()
            .is_empty()
    );
}

/// The artifact names rules_cc asks for while declaring this compile's outputs.
#[test]
fn golden_brotli_dictionary_artifact_names() {
    let features = toolchain_features();
    use blaze_cc_features::cc_starlark_internal::get_artifact_name_for_category;
    let pic = get_artifact_name_for_category(&features, "PIC_FILE", "brotlicommon/dictionary");
    assert_eq!(pic, "brotlicommon/dictionary.pic");
    assert_eq!(
        get_artifact_name_for_category(&features, "PIC_OBJECT_FILE", "brotlicommon/dictionary"),
        "brotlicommon/dictionary.pic.o"
    );
    assert_eq!(
        get_artifact_name_for_category(&features, "OBJECT_FILE", &pic),
        "brotlicommon/dictionary.pic.o"
    );
    assert_eq!(
        get_artifact_name_for_category(&features, "INCLUDED_FILE_LIST", &pic),
        "brotlicommon/dictionary.pic.d"
    );
    // The toolchain overrides only cpp_module (with its default '.pcm').
    assert_eq!(
        get_artifact_name_for_category(&features, "CPP_MODULE", "m/x"),
        "m/x.pcm"
    );
}
