//! Port of `src/test/java/com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtractTest.java`.

use std::io::Read;

use blaze_actions::ActionRef;
use blaze_actions::analysis::actions::binary_file_write_action::BinaryFileWriteAction;
use blaze_actions::analysis::actions::file_write_action::FileWriteAction;
use blaze_cmdline::Label;
use blaze_protos::stardoc_output::*;
use blaze_sky_top::testing::BuildViewTestCase;
use blaze_sky_top::testing::create_module_key;
use blaze_starlarkdocextract::module_info_extractor::implicit_repository_rule_attributes;
use blaze_starlarkdocextract::rule_info_extractor::implicit_rule_attributes;
use prost::Message;

// Java: src/test/java/com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtractTest.java:58 StarlarkDocExtractTest.protoFromBinaryFileWriteAction
fn proto_from_binary_file_write_action(action: &ActionRef) -> ModuleInfo {
    let action = action
        .as_any()
        .downcast_ref::<BinaryFileWriteAction>()
        .expect("instance of BinaryFileWriteAction");
    let mut bytes = Vec::new();
    action
        .get_source()
        .open_stream()
        .unwrap()
        .read_to_end(&mut bytes)
        .unwrap();
    ModuleInfo::decode(bytes.as_slice()).unwrap()
}

// Java: src/test/java/com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtractTest.java:65 StarlarkDocExtractTest.protoFromTextFileWriteAction
fn proto_from_text_file_write_action(action: &ActionRef) -> ModuleInfo {
    let action = action
        .as_any()
        .downcast_ref::<FileWriteAction>()
        .expect("instance of FileWriteAction");
    blaze_protos::text_format::parse::<ModuleInfo>(&action.get_file_contents()).unwrap()
}

// Java: src/test/java/com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtractTest.java:73 StarlarkDocExtractTest.protoFromConfiguredTarget
fn proto_from_configured_target(t: &mut BuildViewTestCase, target_name: &str) -> ModuleInfo {
    let target = t
        .get_configured_target(target_name)
        .unwrap_or_else(|| panic!("no configured target {target_name}"));
    let target_label = Label::parse_canonical_unchecked(target_name);
    let mut output_name = format!(
        "{}.binaryproto",
        target_label.to_path_fragment().get_path_string()
    );
    if !target_label.get_repository().is_main() {
        output_name = format!(
            "external/{}/{}",
            target_label.get_repository().get_name(),
            output_name
        );
    }
    let action = t
        .get_generating_action_for_output(&target, &output_name)
        .expect("generating action");
    proto_from_binary_file_write_action(&action)
}

/// `assertThrows(AssertionError.class, f)` (and `NoSuchElementException`): the panic message.
fn assert_throws(f: impl FnOnce()) -> String {
    let payload = std::panic::catch_unwind(std::panic::AssertUnwindSafe(f))
        .expect_err("expected an exception");
    payload
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| payload.downcast_ref::<&str>().map(|s| s.to_string()))
        .unwrap_or_default()
}

fn assert_contains(message: &str, expected: &str) {
    assert!(
        message.contains(expected),
        "expected message to contain:\n{expected}\nbut was:\n{message}"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtractTest.java:85 StarlarkDocExtractTest.setUpBzlLibrary
fn new_test() -> BuildViewTestCase {
    let t = BuildViewTestCase::new();
    // TODO(https://github.com/bazelbuild/bazel/issues/18599): get rid of this when we bundle
    // bzl_library with Bazel.
    t.scratch.file(
        "bzl_library.bzl",
        &[r#"
def _bzl_library_impl(ctx):
    deps_files = [x.files for x in ctx.attr.deps]
    all_files = depset(ctx.files.srcs, order = "postorder", transitive = deps_files)
    return DefaultInfo(files = all_files)

bzl_library = rule(
    implementation = _bzl_library_impl,
    attrs = {
        "srcs": attr.label_list(allow_files = [".bzl", ".scl"]),
        "deps": attr.label_list(),
    },
)
"#],
    );
    t
}

fn origin(name: &str, file: &str) -> OriginKey {
    OriginKey {
        name: name.to_owned(),
        file: file.to_owned(),
    }
}

/// Attribute 0 is the implicit `name` attribute.
// Java: src/test/java/com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtractTest.java:386 StarlarkDocExtractTest.getFirstRuleFirstAttr
fn get_first_rule_first_attr(module_info: &ModuleInfo) -> &AttributeInfo {
    &module_info.rule_info[0].attribute[1]
}

// Java: src/test/java/com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtractTest.java:107 StarlarkDocExtractTest.basicFunctionality
#[test]
fn basic_functionality() {
    let mut t = new_test();
    t.scratch
        .file("foo.bzl", &["'''Module doc string'''", "True"]);
    t.scratch.file(
        "BUILD",
        &[r#"
starlark_doc_extract(
    name = "extract",
    src = "foo.bzl",
)
"#],
    );
    let target = t.get_configured_target("//:extract").unwrap();
    let action = t
        .get_generating_action_for_output(&target, "extract.binaryproto")
        .unwrap();
    let module_info = proto_from_binary_file_write_action(&action);
    assert_eq!(module_info.module_docstring, "Module doc string");
    assert_eq!(module_info.file, "//:foo.bzl");
}

// Java: src/test/java/com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtractTest.java:130 StarlarkDocExtractTest.textprotoOut
#[test]
fn textproto_out() {
    let mut t = new_test();
    t.scratch
        .file("foo.bzl", &["'''Module doc string'''", "True"]);
    t.scratch.file(
        "BUILD",
        &[r#"
starlark_doc_extract(
    name = "extract",
    src = "foo.bzl",
)
"#],
    );
    let rule_target = t.get_configured_target("//:extract").unwrap();
    // Verify that we do not generate textproto output unless explicitly requested.
    let message = assert_throws(|| {
        t.get_generating_action_for_output(&rule_target, "extract.textproto");
    });
    assert_contains(&message, "NoSuchElementException");

    let textproto_output_target = t.get_configured_target("//:extract.textproto").unwrap();
    let action = t
        .get_generating_action_for_output(&textproto_output_target, "extract.textproto")
        .unwrap();
    let module_info = proto_from_text_file_write_action(&action);
    assert_eq!(module_info.module_docstring, "Module doc string");
}

// Java: src/test/java/com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtractTest.java:158 StarlarkDocExtractTest.sclDialect
#[test]
fn scl_dialect() {
    let mut t = new_test();
    t.set_build_language_options(&["--experimental_enable_scl_dialect"]);
    t.scratch.file(
        "foo.scl",
        &["def f():", "    '''This is my function'''", "    pass"],
    );
    t.scratch.file(
        "bar.scl",
        &[
            "'''My scl module string'''",
            "load(\"//:foo.scl\", \"f\")",
            "",
            "bar_f = f",
        ],
    );
    t.scratch.file(
        "BUILD",
        &[r#"
load("bzl_library.bzl", "bzl_library")

bzl_library(
    name = "foo_scl",
    srcs = ["foo.scl"],
)

starlark_doc_extract(
    name = "extract",
    src = "bar.scl",
    deps = ["foo_scl"],
)
"#],
    );
    let module_info = proto_from_configured_target(&mut t, "//:extract");
    assert_eq!(module_info.module_docstring, "My scl module string");
    assert_eq!(module_info.file, "//:bar.scl");
    assert_eq!(module_info.func_info[0].doc_string, "This is my function");
}

// Java: src/test/java/com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtractTest.java:198 StarlarkDocExtractTest.sourceWithSyntaxError_fails
#[test]
fn source_with_syntax_error_fails() {
    let mut t = new_test();
    t.scratch.file("error.bzl", &["!!!"]);
    t.scratch.file(
        "error_loader.bzl",
        &["'''This is my module'''", "load(\"error.bzl\", \"x\")"],
    );
    t.scratch.file(
        "BUILD",
        &[r#"
load("bzl_library.bzl", "bzl_library")

bzl_library(
    name = "error_bzl",
    srcs = ["error.bzl"],
)

starlark_doc_extract(
    name = "error_doc",
    src = "error.bzl",
)

starlark_doc_extract(
    name = "error_loader_doc",
    src = "error_loader.bzl",
    deps = ["error_bzl"],
)
"#],
    );

    let error_doc_failure = assert_throws(|| {
        t.get_configured_target("//:error_doc");
    });
    assert_contains(&error_doc_failure, "invalid character: '!'");

    let error_loader_doc_failure = assert_throws(|| {
        t.get_configured_target("//:error_loader_doc");
    });
    assert_contains(&error_loader_doc_failure, "invalid character: '!'");
}

fn function_names(module_info: &ModuleInfo) -> Vec<&str> {
    module_info
        .func_info
        .iter()
        .map(|f| f.function_name.as_str())
        .collect()
}

fn sorted<T: Ord>(mut v: Vec<T>) -> Vec<T> {
    v.sort();
    v
}

// Java: src/test/java/com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtractTest.java:240 StarlarkDocExtractTest.symbolNames
#[test]
fn symbol_names() {
    let mut t = new_test();
    t.scratch.file(
        "foo.bzl",
        &[r#"
def func1():
    pass

def func2():
    pass

def _hidden():
    pass
"#],
    );
    t.scratch.file(
        "BUILD",
        &[r#"
starlark_doc_extract(
    name = "extract_some",
    src = "foo.bzl",
    symbol_names = ["func1"],
)

starlark_doc_extract(
    name = "extract_all",
    src = "foo.bzl",
)
"#],
    );

    let dump_some = proto_from_configured_target(&mut t, "//:extract_some");
    assert_eq!(function_names(&dump_some), vec!["func1"]);

    let dump_all = proto_from_configured_target(&mut t, "//:extract_all");
    assert_eq!(
        sorted(function_names(&dump_all)),
        sorted(vec!["func1", "func2"])
    );
}

// Java: src/test/java/com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtractTest.java:278 StarlarkDocExtractTest.originKey
#[test]
fn origin_key() {
    let mut t = new_test();
    t.scratch.file(
        "origin.bzl",
        &[r#"
def my_macro():
    pass

MyInfo = provider()
MyOtherInfo = provider()
my_rule = rule(
    implementation = lambda ctx: None,
    attrs = {"a": attr.label(providers = [MyInfo, MyOtherInfo])},
    provides = [MyInfo, MyOtherInfo],
)
my_aspect = aspect(implementation = lambda target, ctx: None)
"#],
    );
    t.scratch.file(
        "renamer.bzl",
        &[r#"
load(":origin.bzl", "MyInfo", "MyOtherInfo", "my_aspect", "my_macro", "my_rule")

namespace = struct(
    renamed_macro = my_macro,
    RenamedInfo = MyInfo,
    renamed_rule = my_rule,
    renamed_aspect = my_aspect,
)
other_namespace = struct(
    RenamedOtherInfo = MyOtherInfo,
)
"#],
    );
    t.scratch.file(
        "BUILD",
        &[r#"
load("bzl_library.bzl", "bzl_library")

bzl_library(
    name = "origin_bzl",
    srcs = ["origin.bzl"],
)

starlark_doc_extract(
    name = "extract_renamed",
    src = "renamer.bzl",
    deps = ["origin_bzl"],
    symbol_names = ["namespace"],
)
"#],
    );

    let module_info = proto_from_configured_target(&mut t, "//:extract_renamed");

    assert_eq!(
        function_names(&module_info),
        vec!["namespace.renamed_macro"]
    );
    assert_eq!(
        module_info
            .func_info
            .iter()
            .map(|f| f.origin_key.clone().unwrap())
            .collect::<Vec<_>>(),
        vec![origin("my_macro", "//:origin.bzl")]
    );

    assert_eq!(
        module_info
            .provider_info
            .iter()
            .map(|p| p.provider_name.as_str())
            .collect::<Vec<_>>(),
        vec!["namespace.RenamedInfo"]
    );
    assert_eq!(
        module_info
            .provider_info
            .iter()
            .map(|p| p.origin_key.clone().unwrap())
            .collect::<Vec<_>>(),
        vec![origin("MyInfo", "//:origin.bzl")]
    );

    assert_eq!(
        module_info
            .rule_info
            .iter()
            .map(|r| r.rule_name.as_str())
            .collect::<Vec<_>>(),
        vec!["namespace.renamed_rule"]
    );
    assert_eq!(
        module_info
            .rule_info
            .iter()
            .map(|r| r.origin_key.clone().unwrap())
            .collect::<Vec<_>>(),
        vec![origin("my_rule", "//:origin.bzl")]
    );

    let renamed_group = ProviderNameGroup {
        provider_name: vec![
            "namespace.RenamedInfo".to_owned(),
            "other_namespace.RenamedOtherInfo".to_owned(),
        ],
        origin_key: vec![
            origin("MyInfo", "//:origin.bzl"),
            origin("MyOtherInfo", "//:origin.bzl"),
        ],
    };
    let mut expected_attributes: Vec<AttributeInfo> = implicit_rule_attributes()
        .into_iter()
        .map(|(_, a)| a)
        .collect();
    expected_attributes.push(AttributeInfo {
        name: "a".to_owned(),
        r#type: AttributeType::Label as i32,
        default_value: "None".to_owned(),
        provider_name_group: vec![renamed_group.clone()],
        ..Default::default()
    });
    assert_eq!(module_info.rule_info[0].attribute, expected_attributes);
    assert_eq!(
        module_info.rule_info[0].advertised_providers,
        Some(renamed_group)
    );

    assert_eq!(
        module_info
            .aspect_info
            .iter()
            .map(|a| a.aspect_name.as_str())
            .collect::<Vec<_>>(),
        vec!["namespace.renamed_aspect"]
    );
    assert_eq!(
        module_info
            .aspect_info
            .iter()
            .map(|a| a.origin_key.clone().unwrap())
            .collect::<Vec<_>>(),
        vec![origin("my_aspect", "//:origin.bzl")]
    );
}

// Java: src/test/java/com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtractTest.java:392 StarlarkDocExtractTest.originKeyFileAndModuleInfoFileLabels_forBzlFileInBzlmodModule_areDisplayForm
#[test]
fn origin_key_file_and_module_info_file_labels_for_bzl_file_in_bzlmod_module_are_display_form() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &["bazel_dep(name='origin_repo', version='0.1')"],
    );
    t.registry().add_module(
        &create_module_key("origin_repo", "0.1"),
        &["module(name='origin_repo', version='0.1')"],
    );
    let origin_repo_path = format!("{}/origin_repo+0.1", t.module_root.get_path_string());
    t.scratch
        .file(&format!("{origin_repo_path}/REPO.bazel"), &[]);
    t.scratch.file(
        &format!("{origin_repo_path}/BUILD"),
        &["exports_files(['origin.bzl'])"],
    );
    t.scratch.file(
        &format!("{origin_repo_path}/origin.bzl"),
        &[r#"
def my_macro():
    pass

MyInfo = provider()
my_rule = rule(
    implementation = lambda ctx: None,
    attrs = {"a": attr.label(providers = [MyInfo])},
    provides = [MyInfo],
)
my_aspect = aspect(implementation = lambda target, ctx: None)
"#],
    );
    t.scratch.file(
        "renamer.bzl",
        &[r#"
load("@origin_repo//:origin.bzl", "MyInfo", "my_aspect", "my_macro", "my_rule")

namespace = struct(
    renamed_macro = my_macro,
    RenamedInfo = MyInfo,
    renamed_rule = my_rule,
    renamed_aspect = my_aspect,
)
"#],
    );
    t.scratch.file(
        "BUILD",
        &[r#"
load("bzl_library.bzl", "bzl_library")

bzl_library(
    name = "origin_bzl",
    srcs = ["@origin_repo//:origin.bzl"],
)

starlark_doc_extract(
    name = "extract_origin",
    src = "@origin_repo//:origin.bzl",
)

starlark_doc_extract(
    name = "extract_renamed",
    src = "renamer.bzl",
    deps = ["origin_bzl"],
)
"#],
    );
    t.invalidate_packages();

    // verify that ModuleInfo.name for a .bzl file in another bzlmod module is in display form,
    // i.e. "@origin_repo//:origin.bzl" as opposed to "@@origin_repo+0.1//:origin.bzl"
    let origin_module_info = proto_from_configured_target(&mut t, "//:extract_origin");
    assert_eq!(origin_module_info.file, "@origin_repo//:origin.bzl");

    // verify that OriginKey.name for entities defined in a .bzl file in another bzlmod module is
    // in display form, i.e. "@origin_repo//:origin.bzl" as opposed to
    // "@@origin_repo+0.1//:origin.bzl"
    let renamed = proto_from_configured_target(&mut t, "//:extract_renamed");
    assert_eq!(renamed.file, "//:renamer.bzl");
    let expected = "@origin_repo//:origin.bzl";
    assert_eq!(
        renamed.func_info[0].origin_key.as_ref().unwrap().file,
        expected
    );
    assert_eq!(
        renamed.provider_info[0].origin_key.as_ref().unwrap().file,
        expected
    );
    assert_eq!(
        renamed.aspect_info[0].origin_key.as_ref().unwrap().file,
        expected
    );
    assert_eq!(
        renamed.rule_info[0].origin_key.as_ref().unwrap().file,
        expected
    );
    assert_eq!(
        get_first_rule_first_attr(&renamed).provider_name_group[0].origin_key[0].file,
        expected
    );
    assert_eq!(
        renamed.rule_info[0]
            .advertised_providers
            .as_ref()
            .unwrap()
            .origin_key[0]
            .file,
        expected
    );
}

// Java: src/test/java/com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtractTest.java:480 StarlarkDocExtractTest.exportNestedFunctionsAndLambdas
#[test]
fn export_nested_functions_and_lambdas() {
    let mut t = new_test();
    t.scratch.file(
        "origin.bzl",
        &[r#"
def return_nested():
    def nested(x):
        '''My nested function'''
        pass

    return nested

def return_lambda():
    return lambda y: y
"#],
    );
    t.scratch.file(
        "exporter.bzl",
        &[r#"
load(":origin.bzl", "return_lambda", "return_nested")

exported_nested = return_nested()
exported_lambda = return_lambda()
"#],
    );
    t.scratch.file(
        "BUILD",
        &[r#"
load("bzl_library.bzl", "bzl_library")

bzl_library(
    name = "origin_bzl",
    srcs = ["origin.bzl"],
)

starlark_doc_extract(
    name = "extract_exporter",
    src = "exporter.bzl",
    deps = ["origin_bzl"],
)
"#],
    );

    let module_info = proto_from_configured_target(&mut t, "//:extract_exporter");

    let param = |name: &str| FunctionParamInfo {
        name: name.to_owned(),
        role: FunctionParamRole::ParamRoleOrdinary as i32,
        mandatory: true,
        ..Default::default()
    };
    assert_eq!(
        module_info.func_info,
        vec![
            StarlarkFunctionInfo {
                function_name: "exported_nested".to_owned(),
                doc_string: "My nested function".to_owned(),
                parameter: vec![param("x")],
                // OriginKey.name for nested functions is explicitly unset
                origin_key: Some(origin("", "//:origin.bzl")),
                ..Default::default()
            },
            StarlarkFunctionInfo {
                function_name: "exported_lambda".to_owned(),
                parameter: vec![param("y")],
                // OriginKey.name for lambdas is explicitly unset
                origin_key: Some(origin("", "//:origin.bzl")),
                ..Default::default()
            },
        ]
    );
}

// Java: src/test/java/com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtractTest.java:549 StarlarkDocExtractTest.missingBzlLibraryDeps_fails
#[test]
fn missing_bzl_library_deps_fails() {
    let mut t = new_test();
    t.scratch.file(
        "dep.bzl",
        &[r#"
load("//:forgotten_dep_of_dep.bzl", "g")

def f():
    pass
"#],
    );
    t.scratch
        .file("forgotten_dep_of_dep.bzl", &["def g(): pass"]);
    t.scratch.file(
        "forgotten_dep.bzl",
        &[r#"
load("//:forgotten_dep_of_forgotten_dep.bzl", "j")

def h():
    pass
"#],
    );
    t.scratch.file("forgotten_dep2.bzl", &["def i(): pass"]);
    t.scratch
        .file("forgotten_dep_of_forgotten_dep.bzl", &["def j(): pass"]);
    t.scratch.file(
        "foo.bzl",
        &[r#"
load("//:dep.bzl", "f")
load("//:forgotten_dep.bzl", "h")
load("//:forgotten_dep2.bzl", "i")
"#],
    );
    t.scratch.file(
        "BUILD",
        &[r#"
load("bzl_library.bzl", "bzl_library")

bzl_library(
    name = "dep_bzl",
    srcs = ["dep.bzl"],
)

starlark_doc_extract(
    name = "extract",
    src = "foo.bzl",  # Note that src does not need to be part of deps
    deps = ["dep_bzl"],
)
"#],
    );

    let e = assert_throws(|| {
        t.get_configured_target("//:extract");
    });
    assert_contains(
        &e,
        "missing bzl_library targets for Starlark module(s) //:forgotten_dep_of_dep.bzl, \
         //:forgotten_dep.bzl, //:forgotten_dep2.bzl",
    );
    // We do not want to log transitive deps of already missing deps in the error message - it
    // would be hard to read and unnecessary, since a valid bzl_library target should bring in
    // its transitive deps.
    assert!(!e.contains("forgotten_dep_of_forgotten_dep.bzl"), "{e}");
}

// Java: src/test/java/com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtractTest.java:612 StarlarkDocExtractTest.depsWithDerivedFiles_onUnknownLoads_failsAndPrintsDerivedFiles
#[test]
fn deps_with_derived_files_on_unknown_loads_fails_and_prints_derived_files() {
    let mut t = new_test();
    t.scratch.file("BUILD", &[]);
    t.scratch.file(
        "pkg/source_file_masked_by_rule_name.bzl",
        &["def f(): pass"],
    );
    t.scratch.file(
        "pkg/source_file_masked_by_rule_output_name.bzl",
        &["def g(): pass"],
    );
    t.scratch.file(
        "pkg/foo.bzl",
        &[r#"
load("//pkg:source_file_masked_by_rule_name.bzl", "f")
load("//pkg:source_file_masked_by_rule_output_name.bzl", "g")
"#],
    );
    t.scratch.file(
        "pkg/BUILD",
        &[r#"
load("//:bzl_library.bzl", "bzl_library")

genrule(
    name = "source_file_masked_by_rule_name.bzl",
    outs = ["some_output.bzl"],
    cmd = "touch $@",
)

genrule(
    name = "source_file_masked_by_rule_output_name_bzl_generator",
    outs = ["source_file_masked_by_rule_output_name.bzl"],
    cmd = "touch $@",
)

genrule(
    name = "some_rule",
    outs = ["ordinary_generated_file.bzl"],
    cmd = "touch $@",
)

bzl_library(
    name = "deps_bzl",
    srcs = [
        "source_file_masked_by_rule_name.bzl",
        "source_file_masked_by_rule_output_name.bzl",
        "ordinary_generated_file.bzl",
    ],
)

starlark_doc_extract(
    name = "extract",
    src = "foo.bzl",
    deps = ["deps_bzl"],
)
"#],
    );

    let error = assert_throws(|| {
        t.get_configured_target("//pkg:extract");
    });
    assert_contains(
        &error,
        "missing bzl_library targets for Starlark module(s) \
         //pkg:source_file_masked_by_rule_name.bzl, \
         //pkg:source_file_masked_by_rule_output_name.bzl\n\
         Note the following are generated file(s) and cannot be loaded in Starlark: \
         pkg/some_output.bzl (generated by rule //pkg:source_file_masked_by_rule_name.bzl), \
         pkg/source_file_masked_by_rule_output_name.bzl (generated by rule \
         //pkg:source_file_masked_by_rule_output_name_bzl_generator), \
         pkg/ordinary_generated_file.bzl (generated by rule //pkg:some_rule)",
    );
}

// Java: src/test/java/com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtractTest.java:682 StarlarkDocExtractTest.depsWithDerivedFiles_onNoUnknownLoads_succeeds
#[test]
fn deps_with_derived_files_on_no_unknown_loads_succeeds() {
    let mut t = new_test();
    t.scratch.file("BUILD", &[]);
    t.scratch.file(
        "util.bzl",
        &[r#"
def _impl(ctx):
    out = ctx.actions.declare_file(ctx.attr.out)
    ctx.actions.run_shell(command = "touch $1", arguments = [out.path], outputs = [out])
    return DefaultInfo(files = depset([out]), runfiles = ctx.runfiles([out]))

generate_out_without_declaring_it_as_a_target = rule(
    attrs = {"out": attr.string()},
    implementation = _impl,
)
"#],
    );
    t.scratch.file("pkg/source_dep.bzl", &["def f(): pass"]);
    t.scratch
        .file("pkg/foo.bzl", &["load('//pkg:source_dep.bzl', 'f')"]);
    t.scratch.file(
        "pkg/BUILD",
        &[r#"
load("//:bzl_library.bzl", "bzl_library")
load("//:util.bzl", "generate_out_without_declaring_it_as_a_target")

genrule(
    name = "some_rule",
    outs = ["declared_derived_dep.bzl"],
    cmd = "touch $@",
)

# //pkg:generate_source_dep_without_declaring_it_as_a_target masks the source_dep.bzl
# source artifact with a non-target, derived artifact having the same root-relative path.
generate_out_without_declaring_it_as_a_target(
    name = "generate_source_dep_without_declaring_it_as_a_target",
    out = "source_dep.bzl",
)

bzl_library(
    name = "deps_bzl",
    srcs = [
        "declared_derived_dep.bzl",
        "source_dep.bzl",
        "generate_source_dep_without_declaring_it_as_a_target",
    ],
)

starlark_doc_extract(
    name = "extract",
    src = "foo.bzl",
    deps = ["deps_bzl"],
)
"#],
    );

    t.get_configured_target("//pkg:extract");
    t.assert_no_events();
}

// Java: src/test/java/com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtractTest.java:743 StarlarkDocExtractTest.srcDerivedFile_fails
#[test]
fn src_derived_file_fails() {
    let mut t = new_test();
    t.scratch.file("BUILD", &[]);
    t.scratch
        .file("pkg/source_file_masked_by_rule_name.bzl", &[]);
    t.scratch
        .file("pkg/source_file_masked_by_rule_output_name.bzl", &[]);
    t.scratch.file(
        "pkg/BUILD",
        &[r#"
genrule(
    name = "source_file_masked_by_rule_name.bzl",
    outs = ["some_output.bzl"],
    cmd = "touch $@",
)

genrule(
    name = "source_file_masked_by_rule_output_name_bzl_generator",
    outs = ["source_file_masked_by_rule_output_name.bzl"],
    cmd = "touch $@",
)

genrule(
    name = "some_rule",
    outs = ["ordinary_generated_file.bzl"],
    cmd = "touch $@",
)

starlark_doc_extract(
    name = "source_file_masked_by_rule_name_doc",
    src = "source_file_masked_by_rule_name.bzl",
)

starlark_doc_extract(
    name = "source_file_masked_by_rule_output_name_doc",
    src = "source_file_masked_by_rule_output_name.bzl",
)

starlark_doc_extract(
    name = "ordinary_generated_file_doc",
    src = "ordinary_generated_file.bzl",
)
"#],
    );

    let masked_by_rule_error = assert_throws(|| {
        t.get_configured_target("//pkg:source_file_masked_by_rule_name_doc");
    });
    assert_contains(
        &masked_by_rule_error,
        "pkg/some_output.bzl (generated by rule //pkg:source_file_masked_by_rule_name.bzl) is not \
         a source file and cannot be loaded in Starlark",
    );

    let masked_by_rule_output_error = assert_throws(|| {
        t.get_configured_target("//pkg:source_file_masked_by_rule_output_name_doc");
    });
    assert_contains(
        &masked_by_rule_output_error,
        "pkg/source_file_masked_by_rule_output_name.bzl (generated by rule \
         //pkg:source_file_masked_by_rule_output_name_bzl_generator) is not a source file and \
         cannot be loaded in Starlark",
    );

    let ordinary_generated_file_error = assert_throws(|| {
        t.get_configured_target("//pkg:ordinary_generated_file_doc");
    });
    assert_contains(
        &ordinary_generated_file_error,
        "pkg/ordinary_generated_file.bzl (generated by rule //pkg:some_rule) is not a source file \
         and cannot be loaded in Starlark",
    );
}

// Java: src/test/java/com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtractTest.java:816 StarlarkDocExtractTest.srcAlias_resolvesToActual
#[test]
fn src_alias_resolves_to_actual() {
    let mut t = new_test();
    t.scratch.file("alias_name.bzl", &[]);
    t.scratch.file("alias_actual.bzl", &[]);
    t.scratch.file(
        "BUILD",
        &[r#"
alias(
    name = "alias_name.bzl",
    actual = "alias_actual.bzl",
)

starlark_doc_extract(
    name = "extract",
    src = "alias_name.bzl",
)
"#],
    );

    let module_info = proto_from_configured_target(&mut t, "//:extract");
    assert_eq!(module_info.file, "//:alias_actual.bzl");
}

// Java: src/test/java/com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtractTest.java:838 StarlarkDocExtractTest.srcFilegroup_resolvesToFilegroupSrc
#[test]
fn src_filegroup_resolves_to_filegroup_src() {
    let mut t = new_test();
    t.scratch.file("masked_by_filegroup_name.bzl", &[]);
    t.scratch.file("filegroup_src_actual.bzl", &[]);
    t.scratch.file(
        "BUILD",
        &[r#"
filegroup(
    name = "masked_by_filegroup_name.bzl",
    srcs = ["filegroup_src_actual.bzl"],
)

starlark_doc_extract(
    name = "extract",
    src = "masked_by_filegroup_name.bzl",
)
"#],
    );

    let module_info = proto_from_configured_target(&mut t, "//:extract");
    assert_eq!(module_info.file, "//:filegroup_src_actual.bzl");
}

// Java: src/test/java/com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtractTest.java:860 StarlarkDocExtractTest.srcFilegroup_mustHaveSingleSrc
#[test]
fn src_filegroup_must_have_single_src() {
    let mut t = new_test();
    t.scratch.file("foo.bzl", &[]);
    t.scratch.file("bar.bzl", &[]);
    t.scratch.file(
        "BUILD",
        &[r#"
filegroup(
    name = "no_files",
    srcs = [],
)

filegroup(
    name = "two_files",
    srcs = ["foo.bzl", "bar.bzl"],
)

starlark_doc_extract(
    name = "no_files_doc",
    src = "no_files",
)

starlark_doc_extract(
    name = "two_files_doc",
    src = "two_files",
)
"#],
    );

    let extract_no_files_error = assert_throws(|| {
        t.get_configured_target("//:no_files_doc");
    });
    assert_contains(
        &extract_no_files_error,
        "'//:no_files' must produce a single file",
    );

    let extract_two_files_error = assert_throws(|| {
        t.get_configured_target("//:two_files_doc");
    });
    assert_contains(
        &extract_two_files_error,
        "'//:two_files' must produce a single file",
    );
}

// Java: src/test/java/com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtractTest.java:901 StarlarkDocExtractTest.repositoryRule
#[test]
fn repository_rule() {
    let mut t = new_test();
    t.scratch.file(
        "dep.bzl",
        &[r#"
def _impl(repository_ctx):
    pass

my_repo_rule = repository_rule(
    implementation = _impl,
    doc = '''My repository rule

    With details
    ''',
    attrs = {
        "a": attr.string(doc = "My doc", default = "foo"),
        "b": attr.string(mandatory = True),
        "_c": attr.string(doc = "Hidden attribute"),
    },
    environ = ["FOO_PATH", "BAR_COMPILER"],
)
"#],
    );
    t.scratch.file(
        "foo.bzl",
        &[r#"
load("//:dep.bzl", "my_repo_rule")

foo = struct(repo_rule = my_repo_rule)
"#],
    );
    t.scratch.file(
        "BUILD",
        &[r#"
load("bzl_library.bzl", "bzl_library")

bzl_library(
    name = "dep_bzl",
    srcs = ["dep.bzl"],
)

starlark_doc_extract(
    name = "extract",
    src = "foo.bzl",
    deps = ["dep_bzl"],
)
"#],
    );
    let module_info = proto_from_configured_target(&mut t, "//:extract");
    let mut attribute: Vec<AttributeInfo> = implicit_repository_rule_attributes()
        .into_iter()
        .map(|(_, a)| a)
        .collect();
    attribute.push(AttributeInfo {
        name: "a".to_owned(),
        r#type: AttributeType::String as i32,
        doc_string: "My doc".to_owned(),
        default_value: "\"foo\"".to_owned(),
        ..Default::default()
    });
    attribute.push(AttributeInfo {
        name: "b".to_owned(),
        r#type: AttributeType::String as i32,
        mandatory: true,
        ..Default::default()
    });
    assert_eq!(
        module_info.repository_rule_info,
        vec![RepositoryRuleInfo {
            rule_name: "foo.repo_rule".to_owned(),
            origin_key: Some(origin("my_repo_rule", "//:dep.bzl")),
            doc_string: "My repository rule\n\nWith details".to_owned(),
            attribute,
            environ: vec!["FOO_PATH".to_owned(), "BAR_COMPILER".to_owned()],
        }]
    );
}

// Java: src/test/java/com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtractTest.java:972 StarlarkDocExtractTest.unexportedRepositoryRule_notDocumented
#[test]
fn unexported_repository_rule_not_documented() {
    let mut t = new_test();
    t.scratch.file(
        "foo.bzl",
        &[r#"
def _my_impl(repository_ctx):
    pass

s = struct(
    unexported_repo_rule = repository_rule(
        doc = "Unexported repo rule",
        implementation = _my_impl,
    ),
)
"#],
    );
    t.scratch.file(
        "BUILD",
        &[r#"
starlark_doc_extract(
    name = "extract",
    src = "foo.bzl",
)
"#],
    );
    let module_info = proto_from_configured_target(&mut t, "//:extract");
    assert!(module_info.repository_rule_info.is_empty());
}

// Java: src/test/java/com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtractTest.java:999 StarlarkDocExtractTest.moduleExtension
#[test]
fn module_extension() {
    let mut t = new_test();
    t.scratch.file(
        "dep.bzl",
        &[r#"
_install = tag_class(
    doc = '''Install

    With details''',
    attrs = {
        "artifacts": attr.string_list(doc = "Artifacts"),
        "_hidden": attr.bool(),
    },
)

_artifact = tag_class(
    attrs = {
        "group": attr.string(),
        "artifact": attr.string(default = "foo"),
    },
)

def _impl(ctx):
    pass

my_ext = module_extension(
    doc = '''My extension

    With details''',
    tag_classes = {
        "install": _install,
        "artifact": _artifact,
    },
    implementation = _impl,
)
"#],
    );
    t.scratch.file(
        "foo.bzl",
        &[r#"
load("//:dep.bzl", "my_ext")

foo = struct(ext = my_ext)
"#],
    );
    t.scratch.file(
        "BUILD",
        &[r#"
load("bzl_library.bzl", "bzl_library")

bzl_library(
    name = "dep_bzl",
    srcs = ["dep.bzl"],
)

starlark_doc_extract(
    name = "extract",
    src = "foo.bzl",
    deps = ["dep_bzl"],
)
"#],
    );
    let module_info = proto_from_configured_target(&mut t, "//:extract");
    assert_eq!(module_info.file, "//:foo.bzl");
    let attr = |name: &str, type_: AttributeType, doc: &str, default_value: &str| AttributeInfo {
        name: name.to_owned(),
        r#type: type_ as i32,
        doc_string: doc.to_owned(),
        default_value: default_value.to_owned(),
        ..Default::default()
    };
    assert_eq!(
        module_info.module_extension_info,
        vec![ModuleExtensionInfo {
            extension_name: "foo.ext".to_owned(),
            doc_string: "My extension\n\nWith details".to_owned(),
            origin_key: Some(origin("", "//:dep.bzl")),
            tag_class: vec![
                ModuleExtensionTagClassInfo {
                    tag_name: "install".to_owned(),
                    doc_string: "Install\n\nWith details".to_owned(),
                    attribute: vec![attr(
                        "artifacts",
                        AttributeType::StringList,
                        "Artifacts",
                        "[]"
                    )],
                },
                ModuleExtensionTagClassInfo {
                    tag_name: "artifact".to_owned(),
                    doc_string: String::new(),
                    attribute: vec![
                        attr("group", AttributeType::String, "", "\"\""),
                        attr("artifact", AttributeType::String, "", "\"foo\""),
                    ],
                },
            ],
        }]
    );
}

// Java: src/test/java/com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtractTest.java:1094 StarlarkDocExtractTest.repoName_inMainBzlmodModule
#[test]
fn repo_name_in_main_bzlmod_module() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &["module(name = 'my_module', repo_name = 'legacy_internal_repo_name')"],
    );
    t.scratch.file(
        "foo.bzl",
        &[r#"
def my_macro(arg = Label("//target:target")):
    pass

my_rule = rule(
    implementation = lambda ctx: None,
    attrs = {"a": attr.label(default = "//target:target")},
)
"#],
    );
    t.scratch.file(
        "BUILD",
        &[r#"
starlark_doc_extract(
    name = "with_main_repo_name",
    src = "foo.bzl",
    render_main_repo_name = True,
)

# render_main_repo_name is false by default
starlark_doc_extract(
    name = "without_main_repo_name",
    src = "foo.bzl",
)
"#],
    );
    t.invalidate_packages();

    let with_main_repo_name = proto_from_configured_target(&mut t, "//:with_main_repo_name");
    assert_eq!(with_main_repo_name.file, "@my_module//:foo.bzl");
    assert_eq!(
        with_main_repo_name.rule_info[0]
            .origin_key
            .as_ref()
            .unwrap()
            .file,
        "@my_module//:foo.bzl"
    );
    assert_eq!(
        with_main_repo_name.func_info[0].parameter[0].default_value,
        "Label(\"@my_module//target:target\")"
    );
    assert_eq!(
        get_first_rule_first_attr(&with_main_repo_name).default_value,
        "\"@my_module//target\""
    );

    let without_main_repo_name = proto_from_configured_target(&mut t, "//:without_main_repo_name");
    assert_eq!(without_main_repo_name.file, "//:foo.bzl");
    assert_eq!(
        without_main_repo_name.func_info[0].parameter[0].default_value,
        "Label(\"//target:target\")"
    );
    assert_eq!(
        without_main_repo_name.rule_info[0]
            .origin_key
            .as_ref()
            .unwrap()
            .file,
        "//:foo.bzl"
    );
    assert_eq!(
        get_first_rule_first_attr(&without_main_repo_name).default_value,
        "\"//target\""
    );
}

// Java: src/test/java/com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtractTest.java:1145 StarlarkDocExtractTest.repoName_inBzlmodDep
#[test]
fn repo_name_in_bzlmod_dep() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "module(name = 'my_module')",
            "bazel_dep(name='dep_mod', version='0.1')",
        ],
    );
    t.registry().add_module(
        &create_module_key("dep_mod", "0.1"),
        &["module(name='dep_mod', version='0.1')"],
    );
    let dep_mod_repo_path = format!("{}/dep_mod+0.1", t.module_root.get_path_string());
    t.scratch
        .file(&format!("{dep_mod_repo_path}/REPO.bazel"), &[]);
    t.scratch.file(
        &format!("{dep_mod_repo_path}/foo.bzl"),
        &[r#"
def my_macro(arg = Label("//target:target")):
    pass

my_rule = rule(
    implementation = lambda ctx: None,
    attrs = {"a": attr.label(default = "//target:target")},
)
"#],
    );
    t.scratch.file(
        &format!("{dep_mod_repo_path}/BUILD"),
        &[r#"
starlark_doc_extract(
    name = "extract",
    src = "foo.bzl",
)
"#],
    );
    t.invalidate_packages();

    let module_info = proto_from_configured_target(&mut t, "@dep_mod+//:extract");
    assert_eq!(module_info.file, "@dep_mod//:foo.bzl");
    assert_eq!(
        module_info.func_info[0].parameter[0].default_value,
        "Label(\"@dep_mod//target:target\")"
    );
    assert_eq!(
        module_info.rule_info[0].origin_key.as_ref().unwrap().file,
        "@dep_mod//:foo.bzl"
    );
    assert_eq!(
        get_first_rule_first_attr(&module_info).default_value,
        "\"@dep_mod//target\""
    );
}

// Java: src/test/java/com/google/devtools/build/lib/rules/starlarkdocextract/StarlarkDocExtractTest.java:1183 StarlarkDocExtractTest.unusedDocComments
fn unused_doc_comments(allow_unused_doc_comments: bool) {
    let mut t = new_test();
    t.scratch.file(
        "foo.bzl",
        &[
            "#: Unused doc comment",
            "def my_func():",
            "    pass",
            "",
            "def _my_function():",
            "    pass",
            "",
            "#: Unexpected doc comment",
            "MY_FUNCTION_ALIAS = _my_function",
        ],
    );
    t.scratch.file(
        "BUILD",
        &[&format!(
            "starlark_doc_extract(\n    name = \"extract\",\n    src = \"foo.bzl\",{}\n)\n",
            if allow_unused_doc_comments {
                "\n    allow_unused_doc_comments = True"
            } else {
                ""
            }
        )],
    );
    if allow_unused_doc_comments {
        assert!(
            proto_from_configured_target(&mut t, "//:extract")
                .starlark_other_symbol_info
                .is_empty()
        );
    } else {
        let expected =
            t.ws("in /workspace/foo.bzl: unexpected or conflicting doc comments on line 1");
        let error = assert_throws(|| {
            t.get_configured_target("//:extract");
        });
        assert_contains(&error, &expected);
    }
}

#[test]
fn unused_doc_comments_allow_unused_doc_comments_true() {
    unused_doc_comments(true);
}

#[test]
fn unused_doc_comments_allow_unused_doc_comments_false() {
    unused_doc_comments(false);
}
