//! Port of `src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesTest.java`:
//! materializer rules and `MaterializedDepsInfo`, on the in-process runtime (see
//! `analysis_common`).

mod analysis_common;

#[allow(unused_imports)]
use analysis_common::{
    Fixture, assert_contains_at_least, assert_contains_exactly, assert_contains_none_of,
    base_artifact_names,
};

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesTest.java:36 MaterializerRulesTest.enableDormantDeps
// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesTest.java:42 MaterializerRulesTest.writeMaterializerRulesAllowlist
fn set_up() -> Fixture {
    let mut t = Fixture::new();
    t.use_configuration(&[
        "--experimental_dormant_deps",
        "--incompatible_package_group_has_public_syntax",
    ]);
    write_materializer_rules_allowlist(&mut t, true, false);
    t
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesTest.java:46 MaterializerRulesTest.writeMaterializerRulesAllowlist
fn write_materializer_rules_allowlist(
    t: &mut Fixture,
    materializer_rule_allowed: bool,
    allow_real_deps: bool,
) {
    let allowed = if materializer_rule_allowed {
        "\"public\""
    } else {
        ""
    };
    let real = if allow_real_deps { "\"public\"" } else { "" };
    t.write_tools(
        "tools/allowlists/materializer_rule_allowlist/BUILD",
        &[
            "package_group(",
            "    name = 'materializer_rule_allowlist',",
            &format!("    packages = [{allowed}],"),
            ")",
            "",
            "package_group(",
            "    name = 'materializer_rule_real_deps_allowlist',",
            &format!("    packages = [{real}],"),
            ")",
        ],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesTest.java:69 basicMaterializerRule_works
#[test]
fn basic_materializer_rule_works() {
    let mut t = set_up();
    t.write(
        "defs.bzl",
        &[
            "# Component ######################################",
            "",
            "ComponentInfo = provider(fields = [\"output\"])",
            "",
            "def _component_impl(ctx):",
            "    f = ctx.actions.declare_file(ctx.label.name + \".txt\")",
            "    ctx.actions.write(f, ctx.label.name)",
            "    return ComponentInfo(output = f)",
            "",
            "component = rule(",
            "    implementation = _component_impl,",
            "    provides = [ComponentInfo],",
            ")",
            "",
            "# Component selector #############################",
            "",
            "def _component_selector_impl(ctx):",
            "    selected = []",
            "    for c in ctx.attr.all_components:",
            "        if \"yes\" in str(c.label):",
            "            selected.append(c)",
            "    return MaterializedDepsInfo(deps = selected)",
            "",
            "component_selector = materializer_rule(",
            "    implementation = _component_selector_impl,",
            "    attrs = {",
            "        \"all_components\": attr.dormant_label_list(),",
            "    },",
            ")",
            "",
            "# Binary #########################################",
            "",
            "def _binary_impl(ctx):",
            "    files = [dep[ComponentInfo].output for dep in ctx.attr.deps]",
            "    return DefaultInfo(files = depset(direct = files))",
            "",
            "binary = rule(",
            "    implementation = _binary_impl,",
            "    attrs = {",
            "        \"deps\": attr.label_list(providers = [ComponentInfo]),",
            "    },",
            ")",
        ],
    );
    t.write(
        "BUILD",
        &[
            "load(\":defs.bzl\", \"component\", \"component_selector\", \"binary\")",
            "",
            "binary(",
            "    name = \"bin\",",
            "    deps = [",
            "        \":aaa\",",
            "        \":component_selector\",",
            "        \":zzz\",",
            "    ],",
            ")",
            "",
            "component_selector(",
            "    name = \"component_selector\",",
            "    all_components = [",
            "        \":a_yes\",",
            "        \":b_yes\",",
            "        \":a_no\",",
            "        \":b_no\",",
            "    ],",
            ")",
            "",
            "component(name = \"aaa\")",
            "component(name = \"a_yes\")",
            "component(name = \"b_yes\")",
            "component(name = \"a_no\")",
            "component(name = \"b_no\")",
            "component(name = \"zzz\")",
        ],
    );
    t.update(&["//:bin"]).expect("update");
    let target = "//:bin";
    let files_to_build = t.files_to_build(target);
    assert_contains_exactly(
        &base_artifact_names(&files_to_build),
        &["aaa.txt", "a_yes.txt", "b_yes.txt", "zzz.txt"],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesTest.java:158 multipleMaterializerRules_works
#[test]
fn multiple_materializer_rules_works() {
    let mut t = set_up();
    t.write(
        "defs.bzl",
        &[
            "# Component ######################################",
            "",
            "ComponentInfo = provider(fields = [\"output\"])",
            "",
            "def _component_impl(ctx):",
            "    f = ctx.actions.declare_file(ctx.label.name + \".txt\")",
            "    ctx.actions.write(f, ctx.label.name)",
            "    return ComponentInfo(output = f)",
            "",
            "component = rule(",
            "    implementation = _component_impl,",
            "    provides = [ComponentInfo],",
            ")",
            "",
            "# Component selector #############################",
            "",
            "def _component_selector_impl(ctx):",
            "    selected = []",
            "    for c in ctx.attr.all_components:",
            "        if \"yes\" in str(c.label):",
            "            selected.append(c)",
            "    return MaterializedDepsInfo(deps = selected)",
            "",
            "component_selector = materializer_rule(",
            "    implementation = _component_selector_impl,",
            "    attrs = {",
            "        \"all_components\": attr.dormant_label_list(),",
            "    },",
            ")",
            "",
            "# Binary #########################################",
            "",
            "def _binary_impl(ctx):",
            "    files = [dep[ComponentInfo].output for dep in ctx.attr.deps]",
            "    return DefaultInfo(files = depset(direct = files))",
            "",
            "binary = rule(",
            "    implementation = _binary_impl,",
            "    attrs = {",
            "        \"deps\": attr.label_list(providers = [ComponentInfo]),",
            "    },",
            ")",
        ],
    );
    t.write(
        "BUILD",
        &[
            "load(\":defs.bzl\", \"component\", \"component_selector\", \"binary\")",
            "",
            "binary(",
            "    name = \"bin\",",
            "    deps = [",
            "        \":aaa\",",
            "        \":component_selector\",",
            "        \":component_selector_2\",",
            "        \":zzz\",",
            "    ],",
            ")",
            "",
            "component_selector(",
            "    name = \"component_selector\",",
            "    all_components = [",
            "        \":a_yes\",",
            "        \":b_yes\",",
            "        \":a_no\",",
            "        \":b_no\",",
            "    ],",
            ")",
            "",
            "component_selector(",
            "    name = \"component_selector_2\",",
            "    all_components = [",
            "        \":c_yes\",",
            "        \":d_yes\",",
            "        \":c_no\",",
            "        \":d_no\",",
            "    ],",
            ")",
            "",
            "component(name = \"aaa\")",
            "component(name = \"a_yes\")",
            "component(name = \"b_yes\")",
            "component(name = \"a_no\")",
            "component(name = \"b_no\")",
            "component(name = \"c_yes\")",
            "component(name = \"d_yes\")",
            "component(name = \"c_no\")",
            "component(name = \"d_no\")",
            "component(name = \"zzz\")",
        ],
    );
    t.update(&["//:bin"]).expect("update");
    let target = "//:bin";
    let files_to_build = t.files_to_build(target);
    assert_contains_exactly(
        &base_artifact_names(&files_to_build),
        &[
            "aaa.txt",
            "a_yes.txt",
            "b_yes.txt",
            "c_yes.txt",
            "d_yes.txt",
            "zzz.txt",
        ],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesTest.java:261 multipleMaterializersReturnSameTarget_works
#[test]
fn multiple_materializers_return_same_target_works() {
    let mut t = set_up();
    t.write(
        "defs.bzl",
        &[
            "# Component ######################################",
            "",
            "ComponentInfo = provider(fields = [\"output\"])",
            "",
            "def _component_impl(ctx):",
            "    f = ctx.actions.declare_file(ctx.label.name + \".txt\")",
            "    ctx.actions.write(f, ctx.label.name)",
            "    return ComponentInfo(output = f)",
            "",
            "component = rule(",
            "    implementation = _component_impl,",
            "    provides = [ComponentInfo],",
            ")",
            "",
            "# Component selector #############################",
            "",
            "def _component_selector_impl(ctx):",
            "    selected = []",
            "    for c in ctx.attr.all_components:",
            "        if \"yes\" in str(c.label):",
            "            selected.append(c)",
            "    return MaterializedDepsInfo(deps = selected)",
            "",
            "component_selector = materializer_rule(",
            "    implementation = _component_selector_impl,",
            "    attrs = {",
            "        \"all_components\": attr.dormant_label_list(),",
            "    },",
            ")",
            "",
            "# Binary #########################################",
            "",
            "def _binary_impl(ctx):",
            "    files = [dep[ComponentInfo].output for dep in ctx.attr.deps]",
            "    return DefaultInfo(files = depset(direct = files))",
            "",
            "binary = rule(",
            "    implementation = _binary_impl,",
            "    attrs = {",
            "        \"deps\": attr.label_list(providers = [ComponentInfo]),",
            "    },",
            ")",
        ],
    );
    t.write(
        "BUILD",
        &[
            "load(\":defs.bzl\", \"component\", \"component_selector\", \"binary\")",
            "",
            "binary(",
            "    name = \"bin\",",
            "    deps = [",
            "        \":aaa\",",
            "        \":component_selector\",",
            "        \":component_selector_2\",",
            "        \":zzz\",",
            "    ],",
            ")",
            "",
            "component_selector(",
            "    name = \"component_selector\",",
            "    all_components = [",
            "        \":a_yes\",",
            "        \":b_yes\", # <- overlap",
            "        \":c_yes\", # <- overlap",
            "        \":a_no\",",
            "        \":b_no\",",
            "    ],",
            ")",
            "",
            "component_selector(",
            "    name = \"component_selector_2\",",
            "    all_components = [",
            "        \":b_yes\", # <- overlap",
            "        \":c_yes\", # <- overlap",
            "        \":d_yes\",",
            "        \":c_no\",",
            "        \":d_no\",",
            "    ],",
            ")",
            "",
            "component(name = \"aaa\")",
            "component(name = \"a_yes\")",
            "component(name = \"b_yes\")",
            "component(name = \"a_no\")",
            "component(name = \"b_no\")",
            "component(name = \"c_yes\")",
            "component(name = \"d_yes\")",
            "component(name = \"c_no\")",
            "component(name = \"d_no\")",
            "component(name = \"zzz\")",
        ],
    );
    t.update(&["//:bin"]).expect("update");
    let target = "//:bin";
    let files_to_build = t.files_to_build(target);
    assert_contains_exactly(
        &base_artifact_names(&files_to_build),
        &[
            "aaa.txt",
            "a_yes.txt",
            "b_yes.txt",
            "c_yes.txt",
            "d_yes.txt",
            "zzz.txt",
        ],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesTest.java:366 materializerForDependencyResolutionRule_works
#[test]
fn materializer_for_dependency_resolution_rule_works() {
    let mut t = set_up();
    t.write("defs.bzl", &[
        "# Module Implementation ######################################",
        "",
        "ModuleImplementationInfo = provider(fields = [\"output\"])",
        "",
        "def _module_implementation_impl(ctx):",
        "    f = ctx.actions.declare_file(ctx.label.name + \".txt\")",
        "    ctx.actions.write(f, ctx.label.name)",
        "    return ModuleImplementationInfo(output = f)",
        "",
        "module_implementation = rule(",
        "    implementation = _module_implementation_impl,",
        "    provides = [ModuleImplementationInfo],",
        ")",
        "",
        "# Module ########################################",
        "",
        "# A provider containing a number of modules",
        "ModulesInfo = provider(fields = [\"interface\", \"modules\"])",
        "",
        "# A provider describing a single module",
        "SingleModuleInfo = provider(fields = [\"interface\", \"implementation\"])",
        "",
        "# A module collects every other module in its transitive closure.",
        "def _module_impl(ctx):",
        "    me = SingleModuleInfo(",
        "        interface = ctx.attr.interface,",
        "        implementation = ctx.attr.implementation,",
        "    )",
        "    modules = depset(direct = [me], transitive = [dep[ModulesInfo].modules for dep in ctx.attr.deps])",
        "    return ModulesInfo(interface = ctx.attr.interface, modules = modules)",
        "",
        "module = rule(",
        "    implementation = _module_impl,",
        "    dependency_resolution_rule = True,  # Accessible from materializers",
        "    attrs = {",
        "        \"interface\": attr.string(),",
        "        \"implementation\": attr.dormant_label(),",
        "        \"deps\": attr.label_list(),",
        "    },",
        ")",
        "",
        "# Materializer rule #############################",
        "",
        "def _module_materializer_impl(ctx):",
        "    selected = []",
        "    for m in ctx.attr.all_modules:",
        "        mi = m[ModulesInfo]",
        "        if mi.interface == \"yes\":",
        "            selected.extend([m.implementation for m in mi.modules.to_list()])",
        "    return MaterializedDepsInfo(deps = selected)",
        "",
        "module_materializer = materializer_rule(",
        "    implementation = _module_materializer_impl,",
        "    attrs = {",
        "        \"all_modules\": attr.label_list(),",
        "    },",
        ")",
        "",
        "# Binary #########################################",
        "",
        "def _binary_impl(ctx):",
        "    files = [m[ModuleImplementationInfo].output for m in ctx.attr.modules]",
        "    return DefaultInfo(files = depset(direct = files))",
        "",
        "binary = rule(",
        "    implementation = _binary_impl,",
        "    attrs = {",
        "        \"modules\": attr.label_list(providers = [ModuleImplementationInfo]),",
        "    },",
        ")",
    ]);
    t.write("BUILD", &[
        "load(\":defs.bzl\", \"module_implementation\", \"module\", \"module_materializer\", \"binary\")",
        "",
        "binary(",
        "    name = \"bin\",",
        "    modules = [",
        "        \":module_materializer\",",
        "    ],",
        ")",
        "",
        "module_materializer(",
        "    name = \"module_materializer\",",
        "    all_modules = [",
        "        \":foo_a\",",
        "        \":foo_b\",",
        "        \":bar_a\",",
        "        \":bar_b\",",
        "    ],",
        ")",
        "",
        "module(name = \"foo_a\", interface = \"yes\", implementation = \":foo_a_impl\", deps = [\":baz_a\"])",
        "module(name = \"foo_b\", interface = \"yes\", implementation = \":foo_b_impl\", deps = [\":baz_b\"])",
        "module(name = \"bar_a\", interface = \"no\", implementation = \":bar_a_impl\")",
        "module(name = \"bar_b\", interface = \"no\", implementation = \":bar_b_impl\")",
        "",
        "module(name = \"baz_a\", implementation = \":baz_a_impl\")",
        "module(name = \"baz_b\", implementation = \":baz_b_impl\")",
        "",
        "module_implementation(name = \"foo_a_impl\")",
        "module_implementation(name = \"foo_b_impl\")",
        "module_implementation(name = \"bar_a_impl\")",
        "module_implementation(name = \"bar_b_impl\")",
        "module_implementation(name = \"baz_a_impl\")",
        "module_implementation(name = \"baz_b_impl\")",
    ]);
    t.update(&["//:bin"]).expect("update");
    let target = "//:bin";
    let files_to_build = t.files_to_build(target);
    assert_contains_exactly(
        &base_artifact_names(&files_to_build),
        &[
            "foo_a_impl.txt",
            "foo_b_impl.txt",
            "baz_a_impl.txt",
            "baz_b_impl.txt",
        ],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesTest.java:488 materializerWithRealDeps_throwsError
#[test]
fn materializer_with_real_deps_throws_error() {
    let mut t = set_up();
    t.write("defs.bzl", &[
        "# Component ######################################",
        "",
        "ComponentInfo = provider(fields = [\"output\"])",
        "",
        "def _component_impl(ctx):",
        "    f = ctx.actions.declare_file(ctx.label.name + \".txt\")",
        "    ctx.actions.write(f, ctx.label.name)",
        "    return ComponentInfo(output = f)",
        "",
        "component = rule(",
        "    implementation = _component_impl,",
        "    provides = [ComponentInfo],",
        ")",
        "",
        "# Component selector #############################",
        "",
        "def _component_selector_impl(ctx):",
        "    selected = []",
        "    for c in ctx.attr.all_components:",
        "        if \"yes\" in str(c.label):",
        "            selected.append(c)",
        "    return MaterializedDepsInfo(deps = selected)",
        "",
        "component_selector = materializer_rule(",
        "    implementation = _component_selector_impl,",
        "    attrs = {",
        "        \"all_components\": attr.label_list(),  # Only allows For Dependency Resolution rules",
        "    },",
        ")",
        "",
        "# Binary #########################################",
        "",
        "def _binary_impl(ctx):",
        "    files = [ctx.attr.dep[ComponentInfo].output]",
        "    return DefaultInfo(files = depset(direct = files))",
        "",
        "binary = rule(",
        "    implementation = _binary_impl,",
        "    attrs = {",
        "        \"deps\": attr.label_list(providers = [ComponentInfo], mandatory = True),",
        "    },",
        ")",
    ]);
    t.write(
        "BUILD",
        &[
            "load(\":defs.bzl\", \"component\", \"component_selector\", \"binary\")",
            "",
            "binary(",
            "    name = \"bin\",",
            "    deps = [\":component_selector\"],",
            ")",
            "",
            "component_selector(",
            "    name = \"component_selector\",",
            "    all_components = [\":a_yes\", \":b_no\", \":c_no\", \":d_no\"],",
            ")",
            "",
            "component(name = \"a_yes\")",
            "component(name = \"b_no\")",
            "component(name = \"c_no\")",
            "component(name = \"d_no\")",
        ],
    );
    // reporter.removeHandler(failFastHandler): a failing update returns an error.
    // assertThrows(ViewCreationFailedException)
    assert!(t.update(&["//:bin"]).is_err());
    t.assert_contains_event("in all_components attribute of component_selector rule //:component_selector: materializer rules can depend on only dependency resolution rules via non-dormant attributes; all_components is a non-dormant attribute, and //:a_yes is not a dependency resolution rule");
    t.assert_contains_event("in all_components attribute of component_selector rule //:component_selector: materializer rules can depend on only dependency resolution rules via non-dormant attributes; all_components is a non-dormant attribute, and //:b_no is not a dependency resolution rule");
    t.assert_contains_event("in all_components attribute of component_selector rule //:component_selector: materializer rules can depend on only dependency resolution rules via non-dormant attributes; all_components is a non-dormant attribute, and //:c_no is not a dependency resolution rule");
    t.assert_contains_event("in all_components attribute of component_selector rule //:component_selector: materializer rules can depend on only dependency resolution rules via non-dormant attributes; all_components is a non-dormant attribute, and //:d_no is not a dependency resolution rule");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesTest.java:582 dormantDepsNotAnalyzed
#[test]
fn dormant_deps_not_analyzed() {
    let mut t = set_up();
    t.write(
        "defs.bzl",
        &[
            "# Component ######################################",
            "",
            "ComponentInfo = provider()",
            "",
            "def _component_impl(ctx):",
            "    return ComponentInfo()",
            "",
            "component = rule(",
            "    implementation = _component_impl,",
            ")",
            "",
            "# Fail Component ######################################",
            "",
            "def _fail_component_impl(ctx):",
            "    fail(\"component \" + ctx.label.name + \" should not be analyzed\")",
            "",
            "fail_component = rule(",
            "    implementation = _fail_component_impl,",
            ")",
            "",
            "# Component selector #############################",
            "",
            "def _component_selector_impl(ctx):",
            "    selected = []",
            "    for c in ctx.attr.all_components:",
            "        if not \"fail\" in str(c.label):",
            "            selected.append(c)",
            "    return MaterializedDepsInfo(deps = selected)",
            "",
            "component_selector = materializer_rule(",
            "    implementation = _component_selector_impl,",
            "    attrs = {",
            "        \"all_components\": attr.dormant_label_list(),",
            "    },",
            ")",
            "",
            "# Binary #########################################",
            "",
            "def _binary_impl(ctx):",
            "    return DefaultInfo()",
            "",
            "binary = rule(",
            "    implementation = _binary_impl,",
            "    attrs = {",
            "        \"deps\": attr.label_list(),",
            "    },",
            ")",
        ],
    );
    t.write("BUILD", &[
        "load(\":defs.bzl\", \"component\", \"fail_component\", \"component_selector\", \"binary\")",
        "",
        "binary(",
        "    name = \"bin\",",
        "    deps = [",
        "        \":component_selector\",",
        "    ],",
        ")",
        "",
        "component_selector(",
        "    name = \"component_selector\",",
        "    all_components = [",
        "        \":a_fail\",",
        "        \":b\",",
        "        \":c_fail\",",
        "    ],",
        ")",
        "",
        "fail_component(name = \"a_fail\")",
        "component(name = \"b\")",
        "fail_component(name = \"c_fail\")",
    ]);
    // No assertion needed, the test passes if update() does not throw an exception.
    t.update(&["//:bin"]).expect("update");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesTest.java:666 aspectsThroughMaterializerRules_works
#[test]
fn aspects_through_materializer_rules_works() {
    let mut t = set_up();
    t.write("defs.bzl", &[
        "# Component ######################################",
        "",
        "ComponentInfo = provider()",
        "",
        "def _component_impl(ctx):",
        "    return ComponentInfo()",
        "",
        "component = rule(",
        "    implementation = _component_impl,",
        "    provides = [ComponentInfo],",
        ")",
        "",
        "# Component selector #############################",
        "",
        "def _component_selector_impl(ctx):",
        "    selected = []",
        "    for c in ctx.attr.all_components_dormant:",
        "        if \"yes\" in str(c.label):",
        "            selected.append(c)",
        "    return MaterializedDepsInfo(deps = selected)",
        "",
        "component_selector = materializer_rule(",
        "    implementation = _component_selector_impl,",
        "    attrs = {",
        "        \"all_components_dormant\": attr.dormant_label_list(),",
        "    },",
        ")",
        "",
        "# Aspect #########################################",
        "",
        "AspectInfo = provider(fields = [\"info_artifact\"])",
        "",
        "def _mt_aspect_impl(target, ctx):",
        "    if ctx.rule.kind != \"component\":",
        "        return []",
        "    artifact = ctx.actions.declare_file(target.label.name + \".info\")",
        "    ctx.actions.write(artifact, str(target.label))",
        "    return AspectInfo(info_artifact = artifact)",
        "",
        "mt_aspect = aspect(",
        "    implementation = _mt_aspect_impl,",
        "    attrs = {",
        "        # Context creation code is shared between aspects and rules, so having this aspect",
        "        # dependency in the test ensure that materializer dependency validation is only performed",
        "        # for rules and not for aspects.",
        "        \"_tool\": attr.label(",
        "            default = Label(\"//:aspect_tool\"),",
        "            cfg = \"exec\",",
        "        ),",
        "    }",
        ")",
        "",
        "# Binary #########################################",
        "",
        "def _binary_impl(ctx):",
        "    for dep in ctx.attr.deps:",
        "        print(dep[AspectInfo])",
        "    return DefaultInfo(files = depset(direct = [dep[AspectInfo].info_artifact for dep in ctx.attr.deps]))",
        "",
        "binary = rule(",
        "    implementation = _binary_impl,",
        "    attrs = {",
        "        \"deps\": attr.label_list(providers = [ComponentInfo], aspects = [mt_aspect]),",
        "    },",
        ")",
    ]);
    t.write(
        "BUILD",
        &[
            "load(\":defs.bzl\", \"component\", \"component_selector\", \"binary\")",
            "",
            "binary(",
            "    name = \"bin_dormant\",",
            "    deps = [",
            "        \":aaa\",",
            "        \":component_selector_dormant\",",
            "        \":zzz\",",
            "    ],",
            ")",
            "",
            "component_selector(",
            "    name = \"component_selector_dormant\",",
            "    all_components_dormant = [",
            "        \":a_yes\",",
            "        \":b_yes\",",
            "        \":a_no\",",
            "        \":b_no\",",
            "    ],",
            ")",
            "",
            "component(name = \"aaa\")",
            "component(name = \"a_yes\")",
            "component(name = \"b_yes\")",
            "component(name = \"a_no\")",
            "component(name = \"b_no\")",
            "component(name = \"zzz\")",
            "",
            "genrule(",
            "    name = \"aspect_tool\",",
            "    outs = [\"tool\"],",
            "    executable = True,",
            "    cmd = \"echo 'touch $$1' > $@\",",
            ")",
        ],
    );
    t.update(&["//:bin_dormant"]).expect("update");
    let target_dormant = "//:bin_dormant";
    let files_to_build_dormant = t.files_to_build(target_dormant);
    // The .info files come from the aspect, and only the files from the selected dormant deps
    // should be returned.
    assert_contains_exactly(
        &base_artifact_names(&files_to_build_dormant),
        &["aaa.info", "a_yes.info", "b_yes.info", "zzz.info"],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesTest.java:787 materializerToMaterializer_throwsError
#[test]
fn materializer_to_materializer_throws_error() {
    let mut t = set_up();
    t.write(
        "defs.bzl",
        &[
            "",
            "#################################################",
            "# Component impl",
            "",
            "ComponentImplInfo = provider()",
            "",
            "def _component_impl_impl(ctx):",
            "    return ComponentImplInfo()",
            "",
            "component_impl = rule(",
            "    implementation = _component_impl_impl,",
            ")",
            "",
            "#################################################",
            "# Component selector",
            "",
            "def _component_selector_impl(ctx):",
            "    return MaterializedDepsInfo(deps = ctx.attr.all_components)",
            "",
            "component_selector = materializer_rule(",
            "    implementation = _component_selector_impl,",
            "    attrs = {",
            "        \"all_components\": attr.dormant_label_list(),",
            "    },",
            ")",
            "",
            "#################################################",
            "# Binary",
            "",
            "def _binary_impl(ctx):",
            "    return DefaultInfo()",
            "",
            "binary = rule(",
            "    implementation = _binary_impl,",
            "    attrs = {",
            "        \"deps\": attr.label_list(providers = [ComponentImplInfo]),",
            "    },",
            ")",
        ],
    );
    t.write(
        "BUILD",
        &[
            "load(\":defs.bzl\", \"component_impl\", \"component_selector\", \"binary\")",
            "",
            "binary(",
            "    name = \"bin\",",
            "    deps = [\":component_selector\"],",
            ")",
            "",
            "component_selector(",
            "    name = \"component_selector\",",
            "    all_components = [\":impl_selector\"],",
            ")",
            "",
            "component_selector(",
            "    name = \"impl_selector\",",
            "    all_components = [\":a_impl\"],",
            ")",
            "",
            "component_impl(name=\"a_impl\")",
        ],
    );
    // reporter.removeHandler(failFastHandler): a failing update returns an error.
    // assertThrows(ViewCreationFailedException)
    assert!(t.update(&["//:bin"]).is_err());
    t.assert_contains_event("Error while evaluating materializer target in attribute 'deps' of target '//:bin': Materializer target //:component_selector depends on another materializer target //:impl_selector, which is not supported.");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesTest.java:864 materializerToMaterializerToMaterializer_throwsError
#[test]
fn materializer_to_materializer_to_materializer_throws_error() {
    let mut t = set_up();
    t.write(
        "defs.bzl",
        &[
            "#################################################",
            "# Component",
            "",
            "ComponentInfo = provider(fields = [\"output\"])",
            "",
            "def _component_impl(ctx):",
            "    f = ctx.actions.declare_file(ctx.label.name + \".txt\")",
            "    ctx.actions.write(f, ctx.label.name)",
            "    return ComponentInfo(output = f)",
            "",
            "component = rule(",
            "    implementation = _component_impl,",
            "    provides = [ComponentInfo],",
            ")",
            "",
            "#################################################",
            "# Component selector",
            "",
            "def _component_selector_impl(ctx):",
            "    selected = []",
            "    for c in ctx.attr.all_components:",
            "        if \"yes\" in str(c.label):",
            "            selected.append(c)",
            "    return MaterializedDepsInfo(deps = selected)",
            "",
            "component_selector = materializer_rule(",
            "    implementation = _component_selector_impl,",
            "    attrs = {",
            "        \"all_components\": attr.dormant_label_list(),",
            "    },",
            ")",
            "",
            "#################################################",
            "# Binary",
            "",
            "def _binary_impl(ctx):",
            "    files = [dep[ComponentInfo].output for dep in ctx.attr.deps]",
            "    return DefaultInfo(files = depset(direct = files))",
            "",
            "binary = rule(",
            "    implementation = _binary_impl,",
            "    attrs = {",
            "        \"deps\": attr.label_list(providers = [ComponentInfo]),",
            "    },",
            ")",
        ],
    );
    t.write("BUILD", &[
        "load(\":defs.bzl\", \"component\", \"component_selector\", \"binary\")",
        "",
        "binary(",
        "    name = \"bin\",",
        "    deps = [\":component_selector\"],",
        ")",
        "",
        "component_selector(",
        "    name = \"component_selector\",",
        "    all_components = [\":component_selector2_yes\", \":a_yes\"],",
        ")",
        "",
        "component_selector(",
        "    name = \"component_selector2_yes\",",
        "    all_components = [\":component_selector3_yes\", \":b_yes\", \":component_selector4_yes\", \":x_no\"],",
        ")",
        "",
        "component_selector(",
        "    name = \"component_selector3_yes\",",
        "    all_components = [\":c_yes\", \":d_yes\", \"e_yes\", \":f_yes\", \":y_no\"],",
        ")",
        "",
        "component_selector(",
        "    name = \"component_selector4_yes\",",
        "    all_components = [\":g_yes\", \":h_yes\", \":z_no\"],",
        ")",
        "",
        "component(name = \"a_yes\")",
        "component(name = \"b_yes\")",
        "component(name = \"c_yes\")",
        "component(name = \"d_yes\")",
        "component(name = \"e_yes\")",
        "component(name = \"f_yes\")",
        "component(name = \"g_yes\")",
        "component(name = \"h_yes\")",
        "component(name = \"x_no\")",
        "component(name = \"y_no\")",
        "component(name = \"z_no\")",
    ]);
    // reporter.removeHandler(failFastHandler): a failing update returns an error.
    // assertThrows(ViewCreationFailedException)
    assert!(t.update(&["//:bin"]).is_err());
    t.assert_contains_event("Error while evaluating materializer target in attribute 'deps' of target '//:bin': Materializer target //:component_selector depends on another materializer target //:component_selector2_yes, which is not supported.");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesTest.java:974 aliasToMaterializerRule_works
#[test]
fn alias_to_materializer_rule_works() {
    let mut t = set_up();
    t.write("defs.bzl", &[
        "# Component ######################################",
        "",
        "ComponentInfo = provider()",
        "",
        "def _component_impl(ctx):",
        "    return ComponentInfo()",
        "",
        "component = rule(",
        "    implementation = _component_impl,",
        "    provides = [ComponentInfo],",
        ")",
        "",
        "# Component selector #############################",
        "",
        "def _component_selector_impl(ctx):",
        "    selected = []",
        "    for c in ctx.attr.all_components_dormant:",
        "        if \"yes\" in str(c.label):",
        "            selected.append(c)",
        "    return MaterializedDepsInfo(deps = selected)",
        "",
        "component_selector = materializer_rule(",
        "    implementation = _component_selector_impl,",
        "    attrs = {",
        "        \"all_components_dormant\": attr.dormant_label_list(),",
        "    },",
        ")",
        "",
        "# Aspect #########################################",
        "",
        "AspectInfo = provider(fields = [\"info_artifact\"])",
        "",
        "def _mt_aspect_impl(target, ctx):",
        "    if ctx.rule.kind != \"component\":",
        "        return []",
        "    print(\"aspect visiting target: \" + str(target.label))",
        "    artifact = ctx.actions.declare_file(target.label.name + \".info\")",
        "    ctx.actions.write(artifact, str(target.label))",
        "    return AspectInfo(info_artifact = artifact)",
        "",
        "mt_aspect = aspect(",
        "    implementation = _mt_aspect_impl,",
        ")",
        "",
        "# Binary #########################################",
        "",
        "def _binary_impl(ctx):",
        "    return DefaultInfo(files = depset(direct = [dep[AspectInfo].info_artifact for dep in ctx.attr.deps]))",
        "",
        "binary = rule(",
        "    implementation = _binary_impl,",
        "    attrs = {",
        "        \"deps\": attr.label_list(providers = [ComponentInfo], aspects = [mt_aspect]),",
        "    },",
        ")",
    ]);
    t.write(
        "BUILD",
        &[
            "load(\":defs.bzl\", \"component\", \"component_selector\", \"binary\")",
            "",
            "# Dormant through single alias ####################################",
            "",
            "binary(",
            "    name = \"bin\",",
            "    deps = [",
            "        \":aaa\",",
            "        \":component_selector_alias\",",
            "        \":zzz\",",
            "    ],",
            ")",
            "",
            "alias(",
            "    name = \"component_selector_alias\",",
            "    actual = \":component_selector\",",
            ")",
            "",
            "# Dormant through alias chain ####################################",
            "",
            "binary(",
            "    name = \"bin_alias_chain\",",
            "    deps = [",
            "        \":aaa\",",
            "        \":component_selector_alias_alias\",",
            "        \":zzz\",",
            "    ],",
            ")",
            "",
            "alias(",
            "    name = \"component_selector_alias_alias\",",
            "    actual = \":component_selector_alias\",",
            ")",
            "",
            "# Materializer rules #############################",
            "",
            "component_selector(",
            "    name = \"component_selector\",",
            "    all_components_dormant = [",
            "        \":a_yes\",",
            "        \":b_yes\",",
            "        \":a_no\",",
            "        \":b_no\",",
            "    ],",
            ")",
            "",
            "component(name = \"aaa\")",
            "component(name = \"a_yes\")",
            "component(name = \"b_yes\")",
            "component(name = \"a_no\")",
            "component(name = \"b_no\")",
            "component(name = \"zzz\")",
        ],
    );
    t.update(&["//:bin"]).expect("update");
    let target = "//:bin";
    let files_to_build = t.files_to_build(target);
    assert_contains_exactly(
        &base_artifact_names(&files_to_build),
        &["aaa.info", "a_yes.info", "b_yes.info", "zzz.info"],
    );
    t.clear_events();
    t.clean_skyframe();
    t.update(&["//:bin_alias_chain"]).expect("update");
    let target_alias_chain = "//:bin_alias_chain";
    let files_to_build_alias_chain = t.files_to_build(target_alias_chain);
    assert_contains_exactly(
        &base_artifact_names(&files_to_build_alias_chain),
        &["aaa.info", "a_yes.info", "b_yes.info", "zzz.info"],
    );
    // Especially when going through an alias chain, an aspect should still visit the nodes once.
    t.assert_contains_event_with_frequency("aspect visiting target: @@//:aaa", 1);
    t.assert_contains_event_with_frequency("aspect visiting target: @@//:zzz", 1);
    t.assert_contains_event_with_frequency("aspect visiting target: @@//:b_yes", 1);
    t.assert_contains_event_with_frequency("aspect visiting target: @@//:a_yes", 1);
    t.assert_does_not_contain_event("aspect visiting target: @@//:a_no");
    t.assert_does_not_contain_event("aspect visiting target: @@//:b_no");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesTest.java:1118 materializerToAlias_works
#[test]
fn materializer_to_alias_works() {
    let mut t = set_up();
    t.write(
        "defs.bzl",
        &[
            "# Component ######################################",
            "",
            "ComponentInfo = provider(fields = [\"output\"])",
            "",
            "def _component_impl(ctx):",
            "    f = ctx.actions.declare_file(ctx.label.name + \".txt\")",
            "    ctx.actions.write(f, ctx.label.name)",
            "    return ComponentInfo(output = f)",
            "",
            "component = rule(",
            "    implementation = _component_impl,",
            "    provides = [ComponentInfo],",
            ")",
            "",
            "# Component selector #############################",
            "",
            "def _component_selector_impl(ctx):",
            "    selected = []",
            "    for c in ctx.attr.all_components_dormant:",
            "        if \"yes\" in str(c.label):",
            "            selected.append(c)",
            "    return MaterializedDepsInfo(deps = selected)",
            "",
            "component_selector = materializer_rule(",
            "    implementation = _component_selector_impl,",
            "    attrs = {",
            "        \"all_components_dormant\": attr.dormant_label_list(),",
            "    },",
            ")",
            "",
            "# Binary #########################################",
            "",
            "def _binary_impl(ctx):",
            "    files = [dep[ComponentInfo].output for dep in ctx.attr.deps]",
            "    return DefaultInfo(files = depset(direct = files))",
            "",
            "binary = rule(",
            "    implementation = _binary_impl,",
            "    attrs = {",
            "        \"deps\": attr.label_list(providers = [ComponentInfo]),",
            "    },",
            ")",
        ],
    );
    t.write(
        "BUILD",
        &[
            "load(\":defs.bzl\", \"component\", \"component_selector\", \"binary\")",
            "",
            "binary(",
            "    name = \"bin\",",
            "    deps = [",
            "        \":aaa\",",
            "        \":component_selector\",",
            "        \":zzz\",",
            "    ],",
            ")",
            "",
            "component_selector(",
            "    name = \"component_selector\",",
            "    all_components_dormant = [",
            "        \":a_yes_alias\",",
            "        \":b_yes_alias\",",
            "        \":a_no\",",
            "        \":b_no\",",
            "        \":b_no_alias\",",
            "    ],",
            ")",
            "",
            "alias(name = \"a_yes_alias\", actual = \"a\")",
            "alias(name = \"b_yes_alias\", actual = \"b\")",
            "alias(name = \"b_no_alias\", actual = \"b_no\")",
            "",
            "component(name = \"aaa\")",
            "component(name = \"a\")",
            "component(name = \"b\")",
            "component(name = \"a_no\")",
            "component(name = \"b_no\")",
            "component(name = \"zzz\")",
        ],
    );
    t.update(&["//:bin"]).expect("update");
    let target = "//:bin";
    let files_to_build = t.files_to_build(target);
    assert_contains_exactly(
        &base_artifact_names(&files_to_build),
        &["aaa.txt", "a.txt", "b.txt", "zzz.txt"],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesTest.java:1215 aliasToMaterializerToAliasToMaterializerToAlias_throwsError
#[test]
fn alias_to_materializer_to_alias_to_materializer_to_alias_throws_error() {
    let mut t = set_up();
    t.write(
        "defs.bzl",
        &[
            "# Component ######################################",
            "",
            "ComponentInfo = provider(fields = [\"output\"])",
            "",
            "def _component_impl(ctx):",
            "    f = ctx.actions.declare_file(ctx.label.name + \".txt\")",
            "    ctx.actions.write(f, ctx.label.name)",
            "    return ComponentInfo(output = f)",
            "",
            "component = rule(",
            "    implementation = _component_impl,",
            "    provides = [ComponentInfo],",
            ")",
            "",
            "# Component selector #############################",
            "",
            "def _component_selector_impl(ctx):",
            "    return MaterializedDepsInfo(deps = [ctx.attr.component])",
            "",
            "component_selector = materializer_rule(",
            "    implementation = _component_selector_impl,",
            "    attrs = {",
            "        \"component\": attr.dormant_label(),",
            "    },",
            ")",
            "",
            "# Binary #########################################",
            "",
            "def _binary_impl(ctx):",
            "    files = [dep[ComponentInfo].output for dep in ctx.attr.deps]",
            "    return DefaultInfo(files = depset(direct = files))",
            "",
            "binary = rule(",
            "    implementation = _binary_impl,",
            "    attrs = {",
            "        \"deps\": attr.label_list(providers = [ComponentInfo]),",
            "    },",
            ")",
        ],
    );
    t.write(
        "BUILD",
        &[
            "load(\":defs.bzl\", \"component\", \"component_selector\", \"binary\")",
            "",
            "binary(",
            "    name = \"bin\",",
            "    deps = [",
            "        \":aaa\",",
            "        \":alias_to_materializer_to_alias_to_materializer_to_alias\",",
            "        \":zzz\",",
            "    ],",
            ")",
            "",
            "alias(",
            "    name = \"alias_to_materializer_to_alias_to_materializer_to_alias\",",
            "    actual = \":materializer_to_alias_to_materializer_to_alias\",",
            ")",
            "",
            "component_selector(",
            "    name = \"materializer_to_alias_to_materializer_to_alias\",",
            "    component = \":alias_to_materializer_to_alias\",",
            ")",
            "",
            "alias(",
            "    name = \"alias_to_materializer_to_alias\",",
            "    actual = \":materializer_to_alias\",",
            ")",
            "",
            "component_selector(",
            "    name = \"materializer_to_alias\",",
            "    component = \":a_alias\",",
            ")",
            "",
            "alias(",
            "    name = \"a_alias\",",
            "    actual = \":a\",",
            ")",
            "",
            "component(name = \"aaa\")",
            "component(name = \"a\")",
            "component(name = \"zzz\")",
        ],
    );
    // reporter.removeHandler(failFastHandler): a failing update returns an error.
    // assertThrows(ViewCreationFailedException)
    assert!(t.update(&["//:bin"]).is_err());
    t.assert_contains_event("Error while evaluating materializer target in attribute 'deps' of target '//:bin': Materializer target //:materializer_to_alias_to_materializer_to_alias depends on another materializer target //:materializer_to_alias, which is not supported");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesTest.java:1388 materializerRulesUnderSplitTransition_works
#[test]
fn materializer_rules_under_split_transition_works() {
    let mut t = set_up();
    write_materializer_split_transition_bzl_file(&mut t);
    t.write("BUILD", &[
        "load(\":defs.bzl\", \"component\", \"component_selector\", \"binary\", \"component_selector_setting\")",
        "",
        "component_selector_setting(",
        "    name = \"component_selector_setting\",",
        "    build_setting_default = \"foo\",",
        ")",
        "",
        "binary(",
        "    name = \"bin\",",
        "    # has a split transition!",
        "    deps = [\":component_selector\"],",
        ")",
        "",
        "component_selector(",
        "    name = \"component_selector\",",
        "    all_components = [",
        "        \":yes_a\",",
        "        \":yes_b\",",
        "        \":no_c\",",
        "        \":no_d\",",
        "    ],",
        ")",
        "",
        "component(name = \"yes_a\")",
        "component(name = \"yes_b\")",
        "component(name = \"no_c\")",
        "component(name = \"no_d\")",
    ]);
    t.update(&["//:bin"]).expect("update");
    let target = "//:bin";
    let files_to_build = t.files_to_build(target);
    let artifact_names = base_artifact_names(&files_to_build);
    assert_contains_at_least(&artifact_names, &["yes_a.txt", "yes_b.txt"]);
    assert_contains_none_of(&artifact_names, &["no_c.txt", "no_d.txt"]);
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesTest.java:1436 materializerRulesUnderSplitTransitionAndSelect_works
#[test]
fn materializer_rules_under_split_transition_and_select_works() {
    let mut t = set_up();
    write_materializer_split_transition_bzl_file(&mut t);
    t.write("BUILD", &[
        "load(\":defs.bzl\", \"component\", \"component_selector\", \"binary\", \"component_selector_setting\")",
        "",
        "component_selector_setting(",
        "    name = \"component_selector_setting\",",
        "    build_setting_default = \"foo\",",
        ")",
        "",
        "config_setting(",
        "    name = \"config_setting_foo\",",
        "    flag_values = {\"//:component_selector_setting\": \"foo\"},",
        ")",
        "",
        "config_setting(",
        "    name = \"config_setting_bar\",",
        "    flag_values = {\"//:component_selector_setting\": \"bar\"},",
        ")",
        "",
        "binary(",
        "    name = \"bin\",",
        "    # has a split transition!",
        "    deps = [\":component_selector\"],",
        ")",
        "",
        "component_selector(",
        "    name = \"component_selector\",",
        "    all_components = select({",
        "        \":config_setting_foo\": [",
        "            \":yes_a\",",
        "            \":yes_b\",",
        "            \":no_f\",",
        "            \":no_g\",",
        "        ],",
        "        \":config_setting_bar\": [",
        "            \":yes_c\",",
        "            \":yes_d\",",
        "            \":yes_e\",",
        "            \":no_f\",",
        "            \":no_g\",",
        "        ],",
        "    }),",
        ")",
        "",
        "component(name = \"yes_a\")",
        "component(name = \"yes_b\")",
        "component(name = \"yes_c\")",
        "component(name = \"yes_d\")",
        "component(name = \"yes_e\")",
        "component(name = \"no_f\")",
        "component(name = \"no_g\")",
    ]);
    t.update(&["//:bin"]).expect("update");
    let target = "//:bin";
    let files_to_build = t.files_to_build(target);
    let artifact_names = base_artifact_names(&files_to_build);
    assert_contains_at_least(
        &artifact_names,
        &[
            "yes_a.txt",
            "yes_b.txt",
            "yes_c.txt",
            "yes_d.txt",
            "yes_e.txt",
        ],
    );
    assert_contains_none_of(&artifact_names, &["no_f.txt", "no_g.txt"]);
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesTest.java:1504 materializerRulesPropagatesValidationActions_works
#[test]
fn materializer_rules_propagates_validation_actions_works() {
    let mut t = set_up();
    t.write(
        "defs.bzl",
        &[
            "# Component ######################################",
            "",
            "ComponentInfo = provider(fields = [\"output\"])",
            "",
            "def _component_impl(ctx):",
            "    f = ctx.actions.declare_file(ctx.label.name + \".txt\")",
            "    ctx.actions.write(f, ctx.label.name)",
            "",
            "    validation_output = ctx.actions.declare_file(ctx.attr.name + \".validation\")",
            "    ctx.actions.run_shell(",
            "        inputs = [f],",
            "        outputs = [validation_output],",
            "        command = \"touch $1\",",
            "        arguments = [validation_output.path],",
            "    )",
            "",
            "    return [",
            "        ComponentInfo(output = f),",
            "        OutputGroupInfo(_validation = depset([validation_output])),",
            "    ]",
            "",
            "component = rule(",
            "    implementation = _component_impl,",
            "    provides = [ComponentInfo],",
            ")",
            "",
            "# Component selector #############################",
            "",
            "def _component_selector_impl(ctx):",
            "    selected = []",
            "    for c in ctx.attr.all_components:",
            "        if \"yes\" in str(c.label):",
            "            selected.append(c)",
            "    return MaterializedDepsInfo(deps = selected)",
            "",
            "component_selector = materializer_rule(",
            "    implementation = _component_selector_impl,",
            "    attrs = {",
            "        \"all_components\": attr.dormant_label_list(),",
            "    },",
            ")",
            "",
            "# Binary #########################################",
            "",
            "def _binary_impl(ctx):",
            "    files = [dep[ComponentInfo].output for dep in ctx.attr.deps]",
            "    return DefaultInfo(files = depset(direct = files))",
            "",
            "binary = rule(",
            "    implementation = _binary_impl,",
            "    attrs = {",
            "        \"deps\": attr.label_list(providers = [ComponentInfo]),",
            "    },",
            ")",
        ],
    );
    t.write(
        "BUILD",
        &[
            "load(\":defs.bzl\", \"component\", \"component_selector\", \"binary\")",
            "",
            "binary(",
            "    name = \"bin\",",
            "    deps = [",
            "        \":aaa\",",
            "        \":component_selector\",",
            "        \":zzz\",",
            "    ],",
            ")",
            "",
            "component_selector(",
            "    name = \"component_selector\",",
            "    all_components = [",
            "        \":a_yes\",",
            "        \":b_yes\",",
            "        \":a_no\",",
            "        \":b_no\",",
            "    ],",
            ")",
            "",
            "component(name = \"aaa\")",
            "component(name = \"a_yes\")",
            "component(name = \"b_yes\")",
            "component(name = \"a_no\")",
            "component(name = \"b_no\")",
            "component(name = \"zzz\")",
        ],
    );
    t.update(&["//:bin"]).expect("update");
    let target = "//:bin";
    let validation_outputs = t.output_group(target, "_validation");
    let artifact_names = base_artifact_names(&validation_outputs);
    assert_contains_exactly(
        &artifact_names,
        &[
            "aaa.validation",
            "a_yes.validation",
            "b_yes.validation",
            "zzz.validation",
        ],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesTest.java:1612 materializerTargetInMaterializerAttribute_works
#[test]
fn materializer_target_in_materializer_attribute_works() {
    let mut t = set_up();
    t.write("defs.bzl", &[
        "ComponentInfo = provider(fields = [\"components\"])",
        "",
        "def _component_impl(ctx):",
        "    current = struct(label=ctx.label, impl = ctx.attr.impl)",
        "    transitive = [d[ComponentInfo].components for d in ctx.attr.deps]",
        "    return [",
        "        ComponentInfo(components = depset(direct = [current], transitive = transitive)),",
        "    ]",
        "",
        "component = rule(",
        "    implementation = _component_impl,",
        "    attrs = {",
        "        \"deps\": attr.label_list(providers = [ComponentInfo]),",
        "        \"impl\": attr.dormant_label(),",
        "    },",
        "    provides = [ComponentInfo],",
        "    dependency_resolution_rule = True,",
        ")",
        "",
        "######################################",
        "",
        "def _component_selector_impl(ctx):",
        "    selected = []",
        "    for c in ctx.attr.all_components:",
        "        if \"yes\" in str(c.label):",
        "            selected.append(c)",
        "    return MaterializedDepsInfo(deps = selected)",
        "",
        "component_selector = materializer_rule(",
        "    implementation = _component_selector_impl,",
        "    attrs = {",
        "        \"all_components\": attr.dormant_label_list(),",
        "    },",
        ")",
        "",
        "######################################",
        "",
        "def _binary_impl(ctx):",
        "    return [DefaultInfo(files=depset(ctx.files._impls))]",
        "",
        "def _materializer(ctx):",
        "    for c in ctx.attr.components:",
        "        print(c)",
        "    return []",
        "",
        "binary = rule(",
        "    implementation = _binary_impl,",
        "    attrs = {",
        "        \"components\": attr.label_list(for_dependency_resolution = True),",
        "        \"_impls\": attr.label_list(materializer = _materializer),",
        "    }",
        ")",
    ]);
    t.write(
        "BUILD",
        &[
            "load(\":defs.bzl\", \"component_selector\", \"component\", \"binary\")",
            "",
            "binary(",
            "    name = \"materializer_attr_bin\",",
            "    components = [\":a_yes\", \":b_no\", \":component_selector\"],",
            ")",
            "",
            "component_selector(",
            "    name = \"component_selector\",",
            "    all_components = [",
            "        \":c_yes\",",
            "        \":d_no\",",
            "    ],",
            ")",
            "",
            "component(name=\"a_yes\")",
            "component(name=\"b_no\")",
            "component(name=\"c_yes\")",
            "component(name=\"d_no\")",
        ],
    );
    t.update(&["//:materializer_attr_bin"]).expect("update");
    // no ending ">" because after the label are the providers which are not important here.
    t.assert_contains_event("<target //:a_yes");
    t.assert_contains_event("<target //:b_no");
    t.assert_contains_event("<target //:c_yes");
    // This was not selected by the materializer target, so it should not show up to the
    // materializer attribute.
    t.assert_does_not_contain_event("<target //:d_no");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesTest.java:1705 singletonListOfMaterializedDepsInfo_works
#[test]
fn singleton_list_of_materialized_deps_info_works() {
    let mut t = set_up();
    t.write(
        "defs.bzl",
        &[
            "# Component ######################################",
            "",
            "ComponentInfo = provider(fields = [\"output\"])",
            "",
            "def _component_impl(ctx):",
            "    f = ctx.actions.declare_file(ctx.label.name + \".txt\")",
            "    ctx.actions.write(f, ctx.label.name)",
            "    return ComponentInfo(output = f)",
            "",
            "component = rule(",
            "    implementation = _component_impl,",
            "    provides = [ComponentInfo],",
            ")",
            "",
            "# Component selector #############################",
            "",
            "def _component_selector_impl(ctx):",
            "    selected = []",
            "    for c in ctx.attr.all_components:",
            "        if \"yes\" in str(c.label):",
            "            selected.append(c)",
            "    return [MaterializedDepsInfo(deps = selected)]",
            "",
            "component_selector = materializer_rule(",
            "    implementation = _component_selector_impl,",
            "    attrs = {",
            "        \"all_components\": attr.dormant_label_list(),",
            "    },",
            ")",
            "",
            "# Binary #########################################",
            "",
            "def _binary_impl(ctx):",
            "    files = [dep[ComponentInfo].output for dep in ctx.attr.deps]",
            "    return DefaultInfo(files = depset(direct = files))",
            "",
            "binary = rule(",
            "    implementation = _binary_impl,",
            "    attrs = {",
            "        \"deps\": attr.label_list(providers = [ComponentInfo]),",
            "    },",
            ")",
        ],
    );
    t.write(
        "BUILD",
        &[
            "load(\":defs.bzl\", \"component\", \"component_selector\", \"binary\")",
            "",
            "binary(",
            "    name = \"bin\",",
            "    deps = [",
            "        \":aaa\",",
            "        \":component_selector\",",
            "        \":zzz\",",
            "    ],",
            ")",
            "",
            "component_selector(",
            "    name = \"component_selector\",",
            "    all_components = [\":a_yes\", \":b_yes\", \":c_no\", \":d_no\"],",
            ")",
            "",
            "component(name = \"aaa\")",
            "component(name = \"a_yes\")",
            "component(name = \"b_yes\")",
            "component(name = \"c_no\")",
            "component(name = \"d_no\")",
            "component(name = \"zzz\")",
        ],
    );
    t.update(&["//:bin"]).expect("update");
    let target = "//:bin";
    let files_to_build = t.files_to_build(target);
    assert_contains_exactly(
        &base_artifact_names(&files_to_build),
        &["aaa.txt", "a_yes.txt", "b_yes.txt", "zzz.txt"],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesTest.java:1787 materializerRuleDocsAttr_works
#[test]
fn materializer_rule_docs_attr_works() {
    let mut t = set_up();
    t.write(
        "defs.bzl",
        &[
            "materializer_rule_with_doc = materializer_rule(",
            "    implementation = lambda ctx: MaterializedDepsInfo(deps = []),",
            "    doc = \"This is a doc string\",",
            ")",
        ],
    );
    t.write(
        "BUILD",
        &[
            "load(\":defs.bzl\", \"materializer_rule_with_doc\")",
            "",
            "materializer_rule_with_doc(",
            "    name = \"materializer_rule_with_doc\",",
            ")",
        ],
    );
    t.update(&["//:materializer_rule_with_doc"])
        .expect("update");
    let target = t.get_target("//:materializer_rule_with_doc");
    assert_eq!(
        target
            .as_rule()
            .expect("a rule")
            .get_rule_class_object()
            .get_starlark_documentation(),
        Some("This is a doc string")
    );
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesTest.java:1814 materializerRuleInWithDefaultApplicableLicenses_works
#[test]
fn materializer_rule_in_with_default_applicable_licenses_works() {
    let mut t = set_up();
    t.write(
        "fake_licenses/BUILD",
        &[
            "filegroup(",
            "    name = \"license\",",
            "    srcs = [\"LICENSE\"],",
            ")",
        ],
    );
    t.write(
        "defs.bzl",
        &[
            "mr = materializer_rule(",
            "    implementation = lambda ctx: MaterializedDepsInfo(deps = []),",
            ")",
        ],
    );
    t.write(
        "BUILD",
        &[
            "load(\":defs.bzl\", \"mr\")",
            "",
            "package(",
            "    default_applicable_licenses = [\"//fake_licenses:license\"],",
            ")",
            "",
            "mr(",
            "    name = \"materializer_rule\",",
            ")",
        ],
    );
    t.update(&["//:materializer_rule"]).expect("update");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesTest.java:1851 materializerAllowList_nonAllowedThrowsError
#[test]
fn materializer_allow_list_non_allowed_throws_error() {
    let mut t = set_up();
    write_materializer_rules_allowlist(&mut t, false, false);
    t.write(
        "defs.bzl",
        &[
            "# Component ######################################",
            "",
            "ComponentInfo = provider(fields = [\"output\"])",
            "",
            "def _component_impl(ctx):",
            "    f = ctx.actions.declare_file(ctx.label.name + \".txt\")",
            "    ctx.actions.write(f, ctx.label.name)",
            "    return ComponentInfo(output = f)",
            "",
            "component = rule(",
            "    implementation = _component_impl,",
            "    provides = [ComponentInfo],",
            ")",
            "",
            "# Component selector #############################",
            "",
            "def _component_selector_impl(ctx):",
            "    selected = []",
            "    for c in ctx.attr.all_components:",
            "        if \"yes\" in str(c.label):",
            "            selected.append(c)",
            "    return MaterializedDepsInfo(deps = selected)",
            "",
            "component_selector = materializer_rule(",
            "    implementation = _component_selector_impl,",
            "    attrs = {",
            "        \"all_components\": attr.dormant_label_list(),",
            "    },",
            ")",
            "",
            "# Binary #########################################",
            "",
            "def _binary_impl(ctx):",
            "    files = [dep[ComponentInfo].output for dep in ctx.attr.deps]",
            "    return DefaultInfo(files = depset(direct = files))",
            "",
            "binary = rule(",
            "    implementation = _binary_impl,",
            "    attrs = {",
            "        \"deps\": attr.label_list(providers = [ComponentInfo]),",
            "    },",
            ")",
        ],
    );
    t.write(
        "BUILD",
        &[
            "load(\":defs.bzl\", \"component\", \"component_selector\", \"binary\")",
            "",
            "binary(",
            "    name = \"bin\",",
            "    deps = [",
            "        \":aaa\",",
            "        \":component_selector\",",
            "        \":zzz\",",
            "    ],",
            ")",
            "",
            "component_selector(",
            "    name = \"component_selector\",",
            "    all_components = [\":a_yes\", \":b_no\", \":c_no\", \":d_no\"],",
            ")",
            "",
            "component(name = \"aaa\")",
            "component(name = \"a_yes\")",
            "component(name = \"b_no\")",
            "component(name = \"c_no\")",
            "component(name = \"d_no\")",
            "component(name = \"zzz\")",
        ],
    );
    // reporter.removeHandler(failFastHandler): a failing update returns an error.
    // assertThrows(ViewCreationFailedException)
    assert!(t.update(&["//:bin"]).is_err());
    t.assert_contains_event("in component_selector rule //:component_selector: Non-allowlisted use of materializer rule");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesTest.java:1941 materializerReturnsNonMaterializedDepsInfo_throwsError
#[test]
fn materializer_returns_non_materialized_deps_info_throws_error() {
    let mut t = set_up();
    t.write("defs.bzl", &[
        "# Component ######################################",
        "",
        "ComponentInfo = provider(fields = [\"output\"])",
        "",
        "def _component_impl(ctx):",
        "    f = ctx.actions.declare_file(ctx.label.name + \".txt\")",
        "    ctx.actions.write(f, ctx.label.name)",
        "    return ComponentInfo(output = f)",
        "",
        "component = rule(",
        "    implementation = _component_impl,",
        "    provides = [ComponentInfo],",
        ")",
        "",
        "# Component selector #############################",
        "",
        "def make_materializer_rule(impl):",
        "    return materializer_rule(",
        "        implementation = impl,",
        "        attrs = {",
        "            \"all_components\": attr.dormant_label_list(),",
        "        },",
        "    )",
        "",
        "materializer_wrong_object_DefaultInfo = make_materializer_rule(lambda ctx: DefaultInfo())",
        "materializer_wrong_object_int = make_materializer_rule(lambda ctx: 1)",
        "materializer_wrong_object_in_list = make_materializer_rule(lambda ctx: [DefaultInfo()])",
        "materializer_wrong_object_in_list_int = make_materializer_rule(lambda ctx: [1])",
        "materializer_wrong_list_size = make_materializer_rule(lambda ctx: [DefaultInfo(), ComponentInfo()])",
        "materializer_wrong_list_size_DefaultInfo_int = make_materializer_rule(lambda ctx: [DefaultInfo(), 1])",
        "",
        "# Binary #########################################",
        "",
        "def _binary_impl(ctx):",
        "    return DefaultInfo()",
        "",
        "binary = rule(",
        "    implementation = _binary_impl,",
        "    attrs = {",
        "        \"deps\": attr.label_list(providers = [ComponentInfo]),",
        "    },",
        ")",
    ]);
    t.write("BUILD", &[
        "load(",
        "    \":defs.bzl\",",
        "    \"component\",",
        "    \"binary\",",
        "    \"materializer_wrong_object_DefaultInfo\",",
        "    \"materializer_wrong_object_int\",",
        "    \"materializer_wrong_object_in_list\",",
        "    \"materializer_wrong_object_in_list_int\",",
        "    \"materializer_wrong_list_size\",",
        "    \"materializer_wrong_list_size_DefaultInfo_int\",",
        ")",
        "",
        "binary(name = \"bin_materializer_wrong_object_DefaultInfo\", deps = [\":materializer_wrong_object_DefaultInfo\"])",
        "materializer_wrong_object_DefaultInfo(name = \"materializer_wrong_object_DefaultInfo\", all_components = [\":a_yes\", \":b_yes\"])",
        "",
        "binary(name = \"bin_materializer_wrong_object_int\", deps = [\":materializer_wrong_object_int\"])",
        "materializer_wrong_object_int(name = \"materializer_wrong_object_int\", all_components = [\":a_yes\", \":b_yes\"])",
        "",
        "binary(name = \"bin_materializer_wrong_object_in_list\", deps = [\":materializer_wrong_object_in_list\"])",
        "materializer_wrong_object_in_list(name = \"materializer_wrong_object_in_list\", all_components = [\":a_yes\", \":b_yes\"])",
        "",
        "binary(name = \"bin_materializer_wrong_object_in_list_int\", deps = [\":materializer_wrong_object_in_list_int\"])",
        "materializer_wrong_object_in_list_int(name = \"materializer_wrong_object_in_list_int\", all_components = [\":a_yes\", \":b_yes\"])",
        "",
        "binary(name = \"bin_materializer_wrong_list_size\", deps = [\":materializer_wrong_list_size\"])",
        "materializer_wrong_list_size(name = \"materializer_wrong_list_size\", all_components = [\":a_yes\", \":b_yes\"])",
        "",
        "binary(name = \"bin_materializer_wrong_list_size_DefaultInfo_int\", deps = [\":materializer_wrong_list_size_DefaultInfo_int\"])",
        "materializer_wrong_list_size_DefaultInfo_int(name = \"materializer_wrong_list_size_DefaultInfo_int\", all_components = [\":a_yes\", \":b_yes\"])",
        "",
        "component(name = \"a_yes\")",
        "component(name = \"b_yes\")",
    ]);
    // reporter.removeHandler(failFastHandler): a failing update returns an error.
    // assertThrows(ViewCreationFailedException)
    assert!(
        t.update(&["//:bin_materializer_wrong_object_DefaultInfo"])
            .is_err()
    );
    t.assert_contains_event("Materializer rules must return exactly one MaterializedDepsInfo provider, but got [DefaultInfo]");
    t.clear_events();
    // assertThrows(ViewCreationFailedException)
    assert!(t.update(&["//:bin_materializer_wrong_object_int"]).is_err());
    t.assert_contains_event("Rule should return a struct or a list, but got int");
    t.clear_events();
    // assertThrows(ViewCreationFailedException)
    assert!(
        t.update(&["//:bin_materializer_wrong_object_in_list"])
            .is_err()
    );
    t.assert_contains_event("Materializer rules must return exactly one MaterializedDepsInfo provider, but got [DefaultInfo]");
    t.clear_events();
    // assertThrows(ViewCreationFailedException)
    assert!(
        t.update(&["//:bin_materializer_wrong_object_in_list_int"])
            .is_err()
    );
    t.assert_contains_event(
        "at index 0 of result of rule implementation function, got element of type int, want Info",
    );
    t.clear_events();
    // assertThrows(ViewCreationFailedException)
    assert!(t.update(&["//:bin_materializer_wrong_list_size"]).is_err());
    t.assert_contains_event("Materializer rules must return exactly one MaterializedDepsInfo provider, but got [DefaultInfo, ComponentInfo]");
    t.clear_events();
    // assertThrows(ViewCreationFailedException)
    assert!(
        t.update(&["//:bin_materializer_wrong_list_size_DefaultInfo_int"])
            .is_err()
    );
    t.assert_contains_event(
        "at index 1 of result of rule implementation function, got element of type int, want Info",
    );
    t.clear_events();
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesTest.java:2073 materializerRuleInSingleLabelAttribute_throwsError
#[test]
fn materializer_rule_in_single_label_attribute_throws_error() {
    let mut t = set_up();
    t.write(
        "defs.bzl",
        &[
            "# Component ######################################",
            "",
            "ComponentInfo = provider(fields = [\"output\"])",
            "",
            "def _component_impl(ctx):",
            "    f = ctx.actions.declare_file(ctx.label.name + \".txt\")",
            "    ctx.actions.write(f, ctx.label.name)",
            "    return ComponentInfo(output = f)",
            "",
            "component = rule(",
            "    implementation = _component_impl,",
            "    provides = [ComponentInfo],",
            ")",
            "",
            "# Component selector #############################",
            "",
            "def _component_selector_impl(ctx):",
            "    selected = []",
            "    for c in ctx.attr.all_components:",
            "        if \"yes\" in str(c.label):",
            "            selected.append(c)",
            "    return MaterializedDepsInfo(deps = selected)",
            "",
            "component_selector = materializer_rule(",
            "    implementation = _component_selector_impl,",
            "    attrs = {",
            "        \"all_components\": attr.dormant_label_list(),",
            "    },",
            ")",
            "",
            "# Binary #########################################",
            "",
            "def _binary_impl(ctx):",
            "    files = [ctx.attr.dep[ComponentInfo].output]",
            "    return DefaultInfo(files = depset(direct = files))",
            "",
            "binary = rule(",
            "    implementation = _binary_impl,",
            "    attrs = {",
            "        \"dep\": attr.label(providers = [ComponentInfo]),",
            "    },",
            ")",
        ],
    );
    t.write(
        "BUILD",
        &[
            "load(\":defs.bzl\", \"component\", \"component_selector\", \"binary\")",
            "",
            "binary(",
            "    name = \"bin\",",
            "    dep = \":component_selector\",",
            ")",
            "",
            "component_selector(",
            "    name = \"component_selector\",",
            "    all_components = [\":a_yes\", \":b_yes\", \":c_no\", \":d_no\"],",
            ")",
            "",
            "component(name = \"aaa\")",
            "component(name = \"a_yes\")",
            "component(name = \"b_yes\")",
            "component(name = \"c_no\")",
            "component(name = \"d_no\")",
            "component(name = \"zzz\")",
        ],
    );
    // reporter.removeHandler(failFastHandler): a failing update returns an error.
    // assertThrows(ViewCreationFailedException)
    assert!(t.update(&["//:bin"]).is_err());
    t.assert_contains_event("Error while evaluating materializer target in attribute 'dep' of target '//:bin': Target //:component_selector is a materializer target but attribute 'dep' is a label, not a label list");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesTest.java:2153 usingActionsInMaterializerRule_throwsError
#[test]
fn using_actions_in_materializer_rule_throws_error() {
    let mut t = set_up();
    t.write(
        "defs.bzl",
        &[
            "def _component_selector_impl(ctx):",
            "    f = ctx.actions.declare_file(ctx.label.name + \".txt\")",
            "    return MaterializedDepsInfo(deps = [])",
            "",
            "component_selector = materializer_rule(",
            "    implementation = _component_selector_impl,",
            ")",
        ],
    );
    t.write(
        "BUILD",
        &[
            "load(\":defs.bzl\", \"component_selector\")",
            "component_selector(",
            "    name = \"component_selector\",",
            ")",
        ],
    );
    // reporter.removeHandler(failFastHandler): a failing update returns an error.
    // assertThrows(ViewCreationFailedException)
    assert!(t.update(&["//:component_selector"]).is_err());
    t.assert_contains_event("ctx.actions is not available in materializer rules");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesTest.java:2181 wrongObjectInMaterializedDepsInfo_throwsError
#[test]
fn wrong_object_in_materialized_deps_info_throws_error() {
    let mut t = set_up();
    t.write(
        "defs.bzl",
        &[
            "def _component_impl(ctx):",
            "    return []",
            "",
            "component = rule(",
            "    implementation = _component_impl,",
            ")",
            "",
            "def _component_selector_impl(ctx):",
            "    deps = ctx.attr.components + [1]",
            "    return MaterializedDepsInfo(deps = deps)",
            "",
            "component_selector = materializer_rule(",
            "    implementation = _component_selector_impl,",
            "    attrs = {",
            "        \"components\": attr.dormant_label_list(),",
            "    },",
            ")",
        ],
    );
    t.write(
        "BUILD",
        &[
            "load(\":defs.bzl\", \"component_selector\", \"component\")",
            "",
            "component_selector(",
            "    name = \"component_selector\",",
            "    components = [\":a\", \":b\"],",
            ")",
            "",
            "component(name = \"a\")",
            "component(name = \"b\")",
        ],
    );
    // reporter.removeHandler(failFastHandler): a failing update returns an error.
    // assertThrows(ViewCreationFailedException)
    assert!(t.update(&["//:component_selector"]).is_err());
    t.assert_contains_event("MaterializedDepsInfo dependencies must be Target objects (retrieved from ctx.attr) or DormantDependency objects (from attr.dormant_label() or attr.dormant_label_list() attributes), but got int at index 2");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesTest.java:2227 materializedRuleWithWrongProvider_throwsError
#[test]
fn materialized_rule_with_wrong_provider_throws_error() {
    let mut t = set_up();
    t.write(
        "defs.bzl",
        &[
            "# Component ######################################",
            "",
            "ComponentInfo = provider()",
            "",
            "def _component_impl(ctx):",
            "    return DefaultInfo()",
            "",
            "component = rule(",
            "    implementation = _component_impl,",
            ")",
            "",
            "# Component selector #############################",
            "",
            "def _component_selector_impl(ctx):",
            "    selected = []",
            "    for c in ctx.attr.all_components:",
            "        if \"yes\" in str(c.label):",
            "            selected.append(c)",
            "    return MaterializedDepsInfo(deps = selected)",
            "",
            "component_selector = materializer_rule(",
            "    implementation = _component_selector_impl,",
            "    attrs = {",
            "        \"all_components\": attr.dormant_label_list(),",
            "    },",
            ")",
            "",
            "# Binary #########################################",
            "",
            "def _binary_impl(ctx):",
            "    return DefaultInfo()",
            "",
            "binary = rule(",
            "    implementation = _binary_impl,",
            "    attrs = {",
            "        \"deps\": attr.label_list(providers = [ComponentInfo]),",
            "    },",
            ")",
        ],
    );
    t.write(
        "BUILD",
        &[
            "load(\":defs.bzl\", \"component\", \"component_selector\", \"binary\")",
            "",
            "binary(",
            "    name = \"bin\",",
            "    deps = [\":component_selector\"],",
            ")",
            "",
            "component_selector(",
            "    name = \"component_selector\",",
            "    all_components = [\":a_yes\", \":b_yes\"],",
            ")",
            "",
            "component(name = \"a_yes\")",
            "component(name = \"b_yes\")",
        ],
    );
    // reporter.removeHandler(failFastHandler): a failing update returns an error.
    // assertThrows(ViewCreationFailedException)
    assert!(t.update(&["//:bin"]).is_err());
    t.assert_contains_event("in deps attribute of binary rule //:bin: '//:a_yes' does not have mandatory providers: 'ComponentInfo'");
    t.assert_contains_event("in deps attribute of binary rule //:bin: '//:b_yes' does not have mandatory providers: 'ComponentInfo'");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesTest.java:2347 materializerRuleVisibilityViolation_throwsError
#[test]
fn materializer_rule_visibility_violation_throws_error() {
    let mut t = set_up();
    write_visibility_defs_bzl_file(&mut t);
    t.write("BUILD", &[""]);
    t.write(
        "binary1/BUILD",
        &[
            "load(\"//:defs.bzl\", \"binary\")",
            "",
            "binary(",
            "    name = \"bin1\",",
            "    deps = [",
            "        \"//components:component_selector\",",
            "    ],",
            ")",
        ],
    );
    t.write(
        "binary2/BUILD",
        &[
            "load(\"//:defs.bzl\", \"binary\")",
            "",
            "binary(",
            "    name = \"bin2\",",
            "    deps = [",
            "        \"//components:component_selector\",",
            "    ],",
            ")",
        ],
    );
    t.write(
        "components/BUILD",
        &[
            "load(\"//:defs.bzl\", \"component\", \"component_selector\", \"binary\")",
            "",
            "component_selector(",
            "    name = \"component_selector\",",
            "    all_components = [",
            "        \":foo_a\",",
            "        \":foo_b\",",
            "        \":bar_a\",",
            "        \":bar_b\",",
            "    ],",
            "    visibility = [\"//binary1:__pkg__\"],",
            ")",
            "",
            "component(name = \"foo_a\", visibility = [\"//:__subpackages__\"])",
            "component(name = \"foo_b\", visibility = [\"//:__subpackages__\"])",
            "component(name = \"bar_a\", visibility = [\"//:__subpackages__\"])",
            "component(name = \"bar_b\", visibility = [\"//:__subpackages__\"])",
        ],
    );
    // The materializer target is visible to bin1.
    t.update(&["//binary1:bin1"]).expect("update");
    // The materializer target is not visible to bin2.
    // reporter.removeHandler(failFastHandler): a failing update returns an error.
    // assertThrows(ViewCreationFailedException)
    assert!(t.update(&["//binary2:bin2"]).is_err());
    t.assert_contains_event("ERROR /workspace/binary2/BUILD:3:7: in binary rule //binary2:bin2: Visibility error:\ntarget '//components:component_selector' is not visible from\ntarget '//binary2:bin2'\n");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesTest.java:2414 materializerRuleMaterializedTargetVisibilityViolation_throwsError
#[test]
fn materializer_rule_materialized_target_visibility_violation_throws_error() {
    let mut t = set_up();
    t.write("BUILD", &[""]);
    write_visibility_defs_bzl_file(&mut t);
    t.write(
        "binary/BUILD",
        &[
            "load(\"//:defs.bzl\", \"binary\")",
            "",
            "binary(",
            "    name = \"bin\",",
            "    deps = [",
            "        \"//components:component_selector\",",
            "    ],",
            ")",
        ],
    );
    t.write(
        "components/BUILD",
        &[
            "load(\"//:defs.bzl\", \"component\", \"component_selector\", \"binary\")",
            "",
            "component_selector(",
            "    name = \"component_selector\",",
            "    all_components = [",
            "        \":foo_a\",",
            "        \":foo_b\",",
            "        \":bar_a\",",
            "        \":bar_b\",",
            "    ],",
            "    visibility = [\"//binary:__pkg__\"],",
            ")",
            "",
            "component(name = \"foo_a\", visibility = [\"//visibility:private\"])",
            "component(name = \"foo_b\", visibility = [\"//visibility:private\"])",
            "component(name = \"bar_a\", visibility = [\"//visibility:private\"])",
            "component(name = \"bar_b\", visibility = [\"//visibility:private\"])",
        ],
    );
    // The materializer target is visible to bin, but the materialized targets are not.
    // reporter.removeHandler(failFastHandler): a failing update returns an error.
    // assertThrows(ViewCreationFailedException)
    assert!(t.update(&["//binary:bin"]).is_err());
    t.assert_contains_event("ERROR /workspace/binary/BUILD:3:7: in binary rule //binary:bin: Visibility error:\ntarget '//components:foo_a' is not visible from\ntarget '//binary:bin'\n");
    t.assert_contains_event("ERROR /workspace/binary/BUILD:3:7: in binary rule //binary:bin: Visibility error:\ntarget '//components:foo_b' is not visible from\ntarget '//binary:bin'\n");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesTest.java:1311 writeMaterializerSplitTransitionBzlFile
fn write_materializer_split_transition_bzl_file(t: &mut Fixture) {
    t.write("defs.bzl", &[
        "# Component selector setting ###########################################",
        "",
        "ComponentSelectorProvider = provider(fields = [\"selector\"])",
        "",
        "def _component_selector_setting_impl(ctx):",
        "    return ComponentSelectorProvider(selector = ctx.build_setting_value)",
        "",
        "component_selector_setting = rule(",
        "    implementation = _component_selector_setting_impl,",
        "    build_setting = config.string()",
        ")",
        "",
        "# Component transition",
        "",
        "def _component_selector_setting_transition_impl(settings, attr):",
        "    return [",
        "        {\"//:component_selector_setting\" : \"foo\"},",
        "        {\"//:component_selector_setting\" : \"bar\"},",
        "    ]",
        "",
        "component_transition = transition(",
        "    implementation = _component_selector_setting_transition_impl,",
        "    inputs = [],",
        "    outputs = [\"//:component_selector_setting\"],",
        ")",
        "",
        "# Component ############################################################",
        "",
        "ComponentInfo = provider(fields = [\"output\"])",
        "",
        "def _component_impl(ctx):",
        "    f = ctx.actions.declare_file(ctx.label.name + \".txt\")",
        "    ctx.actions.write(f, ctx.label.name)",
        "    return ComponentInfo(output = f)",
        "",
        "component = rule(",
        "    implementation = _component_impl,",
        "    provides = [ComponentInfo],",
        ")",
        "",
        "# Component selector ###################################################",
        "",
        "def _component_selector_impl(ctx):",
        "    selected = []",
        "    for c in ctx.attr.all_components:",
        "        if \"yes\" in str(c.label):",
        "            selected.append(c)",
        "    return MaterializedDepsInfo(deps = selected)",
        "",
        "component_selector = materializer_rule(",
        "    implementation = _component_selector_impl,",
        "    attrs = {",
        "        \"all_components\": attr.dormant_label_list(),",
        "    },",
        ")",
        "",
        "# Binary ###############################################################",
        "",
        "def _binary_impl(ctx):",
        "    files = [dep[ComponentInfo].output for dep in ctx.attr.deps]",
        "    return DefaultInfo(files = depset(direct = files))",
        "",
        "binary = rule(",
        "    implementation = _binary_impl,",
        "    attrs = {",
        "        \"deps\": attr.label_list(providers = [ComponentInfo], cfg = component_transition),",
        "    },",
        ")",
    ]);
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesTest.java:2300 writeVisibilityDefsBzlFile
fn write_visibility_defs_bzl_file(t: &mut Fixture) {
    t.write(
        "defs.bzl",
        &[
            "# Component ######################################",
            "",
            "ComponentInfo = provider()",
            "",
            "def _component_impl(ctx):",
            "    return ComponentInfo()",
            "",
            "component = rule(",
            "    implementation = _component_impl,",
            "    provides = [ComponentInfo],",
            ")",
            "",
            "# Component selector #############################",
            "",
            "def _component_selector_impl(ctx):",
            "    selected = []",
            "    for c in ctx.attr.all_components:",
            "        if \"foo\" in str(c.label):",
            "            selected.append(c)",
            "    return MaterializedDepsInfo(deps = selected)",
            "",
            "component_selector = materializer_rule(",
            "    implementation = _component_selector_impl,",
            "    attrs = {",
            "        \"all_components\": attr.dormant_label_list(),",
            "    },",
            ")",
            "",
            "# Binary #########################################",
            "",
            "def _binary_impl(ctx):",
            "    return DefaultInfo()",
            "",
            "binary = rule(",
            "    implementation = _binary_impl,",
            "    attrs = {",
            "        \"deps\": attr.label_list(providers = [ComponentInfo]),",
            "    },",
            ")",
        ],
    );
}
