//! Port of
//! `src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesRealDepsTest.java`:
//! materializer rules allowed to have real dependencies, on the in-process runtime (see
//! `analysis_common`).

mod analysis_common;

#[allow(unused_imports)]
use analysis_common::{
    Fixture, assert_contains_at_least, assert_contains_exactly, assert_contains_none_of,
    base_artifact_names,
};

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesRealDepsTest.java:35 MaterializerRulesRealDepsTest.enableDormantDeps
// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesRealDepsTest.java:41 MaterializerRulesRealDepsTest.writeMaterializerRulesAllowlist
fn set_up() -> Fixture {
    let mut t = Fixture::new();
    t.use_configuration(&[
        "--experimental_dormant_deps",
        "--incompatible_package_group_has_public_syntax",
    ]);
    write_materializer_rules_allowlist(&mut t, true, true);
    t
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesRealDepsTest.java:45 MaterializerRulesRealDepsTest.writeMaterializerRulesAllowlist
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

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesRealDepsTest.java:121 basicMaterializerRuleRealDeps_works
#[test]
fn basic_materializer_rule_real_deps_works() {
    let mut t = set_up();
    write_basic_materializer_rule(&mut t);
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
            "        \":a\",",
            "        \":b\",",
            "        \":c\",",
            "        \":d\",",
            "    ],",
            ")",
            "",
            "component(name = \"aaa\", info = \"yes\")",
            "component(name = \"a\", info = \"yes\")",
            "component(name = \"b\", info = \"yes\")",
            "component(name = \"c\", info = \"no\")",
            "component(name = \"d\", info = \"no\")",
            "component(name = \"zzz\", info = \"yes\")",
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

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesRealDepsTest.java:165 multipleMaterializerRules_works
#[test]
fn multiple_materializer_rules_works() {
    let mut t = set_up();
    write_basic_materializer_rule(&mut t);
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
            "        \":a\",",
            "        \":b\",",
            "        \":c\",",
            "        \":d\",",
            "    ],",
            ")",
            "",
            "component_selector(",
            "    name = \"component_selector_2\",",
            "    all_components = [",
            "        \":e\",",
            "        \":f\",",
            "        \":g\",",
            "        \":h\",",
            "    ],",
            ")",
            "",
            "component(name = \"aaa\", info = \"yes\")",
            "component(name = \"a\", info = \"yes\")",
            "component(name = \"b\", info = \"yes\")",
            "component(name = \"c\", info = \"no\")",
            "component(name = \"d\", info = \"no\")",
            "component(name = \"e\", info = \"yes\")",
            "component(name = \"f\", info = \"yes\")",
            "component(name = \"g\", info = \"no\")",
            "component(name = \"h\", info = \"no\")",
            "component(name = \"zzz\", info = \"yes\")",
        ],
    );
    t.update(&["//:bin"]).expect("update");
    let target = "//:bin";
    let files_to_build = t.files_to_build(target);
    assert_contains_exactly(
        &base_artifact_names(&files_to_build),
        &["aaa.txt", "a.txt", "b.txt", "e.txt", "f.txt", "zzz.txt"],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesRealDepsTest.java:223 multipleMaterializersReturnSameTarget_works
#[test]
fn multiple_materializers_return_same_target_works() {
    let mut t = set_up();
    write_basic_materializer_rule(&mut t);
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
            "        \":a\",",
            "        \":b\", # <- overlap",
            "        \":c\", # <- overlap",
            "        \":d\",",
            "        \":e\",",
            "    ],",
            ")",
            "",
            "component_selector(",
            "    name = \"component_selector_2\",",
            "    all_components = [",
            "        \":b\", # <- overlap",
            "        \":c\", # <- overlap",
            "        \":f\",",
            "        \":g\",",
            "        \":h\",",
            "    ],",
            ")",
            "",
            "component(name = \"aaa\", info = \"yes\")",
            "component(name = \"a\", info = \"yes\")",
            "component(name = \"b\", info = \"yes\")",
            "component(name = \"c\", info = \"yes\")",
            "component(name = \"d\", info = \"no\")",
            "component(name = \"e\", info = \"no\")",
            "component(name = \"f\", info = \"yes\")",
            "component(name = \"g\", info = \"no\")",
            "component(name = \"h\", info = \"no\")",
            "component(name = \"zzz\", info = \"yes\")",
        ],
    );
    t.update(&["//:bin"]).expect("update");
    let target = "//:bin";
    let files_to_build = t.files_to_build(target);
    assert_contains_exactly(
        &base_artifact_names(&files_to_build),
        &["aaa.txt", "a.txt", "b.txt", "c.txt", "f.txt", "zzz.txt"],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesRealDepsTest.java:283 materializerWithRealDepsNotInAllowlist_throwsError
#[test]
fn materializer_with_real_deps_not_in_allowlist_throws_error() {
    let mut t = set_up();
    write_materializer_rules_allowlist(&mut t, true, false);
    write_basic_materializer_rule(&mut t);
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
            "    all_components = [\":a\"],",
            ")",
            "",
            "component(name = \"a\", info = \"yes\")",
        ],
    );
    // reporter.removeHandler(failFastHandler): a failing update returns an error.
    // assertThrows(ViewCreationFailedException)
    assert!(t.update(&["//:bin"]).is_err());
    t.assert_contains_event("in component_selector rule //:component_selector: Non-allowlisted use of real deps in materializer target");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesRealDepsTest.java:315 aspectsThroughMaterializerRules_works
#[test]
fn aspects_through_materializer_rules_works() {
    let mut t = set_up();
    t.write("defs.bzl", &[
        "# Component ######################################",
        "",
        "ComponentInfo = provider(fields = [\"info\"])",
        "",
        "def _component_impl(ctx):",
        "    return ComponentInfo(info = ctx.attr.info)",
        "",
        "component = rule(",
        "    implementation = _component_impl,",
        "    provides = [ComponentInfo],",
        "    attrs = {",
        "        \"info\": attr.string(),",
        "    }",
        ")",
        "",
        "# Component selector #############################",
        "",
        "def _component_selector_impl(ctx):",
        "    selected = []",
        "    for c in ctx.attr.all_components:",
        "        if \"yes\" in c[ComponentInfo].info:",
        "            selected.append(c)",
        "    return MaterializedDepsInfo(deps = selected)",
        "",
        "component_selector = materializer_rule(",
        "    implementation = _component_selector_impl,",
        "    allow_real_deps = True,",
        "    attrs = {",
        "        \"all_components\": attr.label_list(),",
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
            "        \":a\",",
            "        \":b\",",
            "        \":c\",",
            "        \":d\",",
            "    ],",
            ")",
            "",
            "component(name = \"aaa\", info = \"yes\")",
            "component(name = \"a\", info = \"yes\")",
            "component(name = \"b\", info = \"yes\")",
            "component(name = \"c\", info = \"no\")",
            "component(name = \"d\", info = \"no\")",
            "component(name = \"zzz\", info = \"yes\")",
            "",
            "genrule(",
            "    name = \"aspect_tool\",",
            "    outs = [\"tool\"],",
            "    executable = True,",
            "    cmd = \"echo 'touch $$1' > $@\",",
            ")",
        ],
    );
    t.update(&["//:bin"]).expect("update");
    let target = "//:bin";
    let files_to_build = t.files_to_build(target);
    // The .info files come from the aspect, and only the files from the selected deps
    // should be returned.
    assert_contains_exactly(
        &base_artifact_names(&files_to_build),
        &["aaa.info", "a.info", "b.info", "zzz.info"],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesRealDepsTest.java:439 aspectsOriginatingFromMaterializerRules_works
#[test]
fn aspects_originating_from_materializer_rules_works() {
    let mut t = set_up();
    t.write(
        "defs.bzl",
        &[
            "# Component ######################################",
            "",
            "ComponentInfo = provider(fields = [\"output\", \"deps_outputs\", \"include\"])",
            "",
            "def _component_impl(ctx):",
            "    f = ctx.actions.declare_file(ctx.label.name + \".txt\")",
            "    ctx.actions.write(f, ctx.label.name)",
            "    return ComponentInfo(",
            "        output = f,",
            "        deps_outputs = depset(",
            "            direct = [f],",
            "            transitive = [dep[ComponentInfo].deps_outputs for dep in ctx.attr.deps],",
            "        ),",
            "        include = ctx.attr.include,",
            "    )",
            "",
            "component = rule(",
            "    implementation = _component_impl,",
            "    attrs = {",
            "        \"deps\": attr.label_list(providers = [ComponentInfo]),",
            "        \"include\": attr.bool(),",
            "    },",
            "    provides = [ComponentInfo],",
            ")",
            "",
            "# Aspect #########################################",
            "",
            "AspectInfo = provider(fields = [\"include\"])",
            "",
            "def _mt_aspect_impl(target, ctx):",
            "    include = False",
            "    for dep in ctx.rule.attr.deps:",
            "        if dep[AspectInfo].include or dep[ComponentInfo].include:",
            "            include = True",
            "            break",
            "    return AspectInfo(include = include)",
            "",
            "mt_aspect = aspect(",
            "    implementation = _mt_aspect_impl,",
            "    attr_aspects = [\"deps\"],",
            ")",
            "",
            "# Component selector #############################",
            "",
            "def _component_selector_impl(ctx):",
            "",
            "    components = []",
            "    for component in ctx.attr.components:",
            "        if AspectInfo in component:",
            "            if component[AspectInfo].include:",
            "                components.append(component)",
            "",
            "    return MaterializedDepsInfo(deps = components)",
            "",
            "component_selector = materializer_rule(",
            "    implementation = _component_selector_impl,",
            "    allow_real_deps = True,",
            "    attrs = {",
            "        \"components\": attr.label_list(aspects = [mt_aspect]),",
            "    },",
            ")",
            "",
            "# Binary #########################################",
            "",
            "def _binary_impl(ctx):",
            "    files = [impl[ComponentInfo].deps_outputs for impl in ctx.attr.deps]",
            "    return DefaultInfo(files = depset(transitive = files))",
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
            "load(\":defs.bzl\", \"binary\", \"component_selector\", \"component\")",
            "",
            "binary(",
            "    name = \"bin\",",
            "    deps = [\":component_selector\"],",
            ")",
            "",
            "component_selector(",
            "    name = \"component_selector\",",
            "    components = [\":a\", \":d\", \":g\"],",
            ")",
            "",
            "component(name = \"a\", deps = [\":b\", \":c\"])",
            "component(name = \"b\")",
            "component(name = \"c\")",
            "",
            "component(name = \"d\", deps = [\":e\", \":f\"])",
            "component(name = \"e\")",
            "component(name = \"f\", include = True)",
            "",
            "component(name = \"g\", deps = [\":h\", \":i\"])",
            "component(name = \"h\")",
            "component(name = \"i\")",
        ],
    );
    t.update(&["//:bin"]).expect("update");
    let target = "//:bin";
    let files_to_build = t.files_to_build(target);
    // Only the artifacts from branches that have "include = True" somewhere in the DAG as collected
    // by the aspect are included.
    assert_contains_exactly(
        &base_artifact_names(&files_to_build),
        &["d.txt", "e.txt", "f.txt"],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesRealDepsTest.java:563 materializerToMaterializer_works
#[test]
fn materializer_to_materializer_works() {
    let mut t = set_up();
    t.write(
        "defs.bzl",
        &[
            "#################################################",
            "# Component",
            "",
            "ComponentInfo = provider(fields = [\"output\", \"info\"])",
            "",
            "def _component_impl(ctx):",
            "    f = ctx.actions.declare_file(ctx.label.name + \".txt\")",
            "    ctx.actions.write(f, ctx.label.name)",
            "    return ComponentInfo(output = f, info = ctx.attr.info)",
            "",
            "component = rule(",
            "    implementation = _component_impl,",
            "    provides = [ComponentInfo],",
            "    attrs = {",
            "        \"info\": attr.string(),",
            "    }",
            ")",
            "",
            "#################################################",
            "# Component selector",
            "",
            "def _component_selector_impl(ctx):",
            "    selected = []",
            "    for c in ctx.attr.all_components:",
            "        if \"yes\" in c[ComponentInfo].info:",
            "            selected.append(c)",
            "    return MaterializedDepsInfo(deps = selected)",
            "",
            "component_selector = materializer_rule(",
            "    implementation = _component_selector_impl,",
            "    allow_real_deps = True,",
            "    attrs = {",
            "        \"all_components\": attr.label_list(),",
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
        "# Because the materializer handler code codes doesn't account for materializers materializing",
        "# other materializers, component_selector2 would materialize \"b\" and \"c\" and \"a\" would be",
        "# overridden here. So we throw an error.",
        "component_selector(",
        "    name = \"component_selector\",",
        "    all_components = [\":component_selector2\", \":a\"],",
        ")",
        "",
        "component_selector(",
        "    name = \"component_selector2\",",
        "    all_components = [\":b\", \":c\", \":d\"],",
        ")",
        "",
        "component(name = \"a\", info = \"yes\")",
        "component(name = \"b\", info = \"yes\")",
        "component(name = \"c\", info = \"yes\")",
        "component(name = \"d\", info = \"yes\")",
    ]);
    t.update(&["//:bin"]).expect("update");
    let target = "//:bin";
    let files_to_build = t.files_to_build(target);
    assert_contains_exactly(
        &base_artifact_names(&files_to_build),
        &["a.txt", "b.txt", "c.txt", "d.txt"],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesRealDepsTest.java:660 aliasToMaterializerRule_works
#[test]
fn alias_to_materializer_rule_works() {
    let mut t = set_up();
    t.write("defs.bzl", &[
        "# Component ######################################",
        "",
        "ComponentInfo = provider(fields = [\"info\"])",
        "",
        "def _component_impl(ctx):",
        "    return ComponentInfo(info = ctx.attr.info)",
        "",
        "component = rule(",
        "    implementation = _component_impl,",
        "    provides = [ComponentInfo],",
        "    attrs = {",
        "        \"info\": attr.string(),",
        "    }",
        ")",
        "",
        "# Component selector #############################",
        "",
        "def _component_selector_impl(ctx):",
        "    selected = []",
        "    for c in ctx.attr.all_components:",
        "        if \"yes\" in c[ComponentInfo].info:",
        "            selected.append(c)",
        "    return MaterializedDepsInfo(deps = selected)",
        "",
        "component_selector = materializer_rule(",
        "    implementation = _component_selector_impl,",
        "    allow_real_deps = True,",
        "    attrs = {",
        "        \"all_components\": attr.label_list(),",
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
    t.write("BUILD", &[
        "load(\":defs.bzl\", \"component\", \"component_selector\", \"binary\")",
        "",
        "# Real deps through single alias ####################################",
        "",
        "binary(",
        "    name = \"bin\",",
        "    deps = [",
        "        \":AAA\",",
        "        \":component_selector_alias\",",
        "        \":ZZZ\",",
        "    ],",
        ")",
        "",
        "alias(",
        "    name = \"component_selector_alias\",",
        "    actual = \":component_selector\",",
        ")",
        "",
        "# Real deps through alias chain ####################################",
        "",
        "binary(",
        "    name = \"bin_alias_chain\",",
        "    deps = [",
        "        \":AAA\",",
        "        \":component_selector_alias_alias\",",
        "        \":ZZZ\",",
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
        "    all_components = [",
        "        \":a\",",
        "        \":b\",",
        "        \":c\",",
        "        \":d\",",
        "    ],",
        ")",
        "",
        "# capitalized because the \"a\" component will also match the \"aaa\" component in the assertions below",
        "component(name = \"AAA\", info = \"yes\")",
        "component(name = \"a\", info = \"yes\")",
        "component(name = \"b\", info = \"yes\")",
        "component(name = \"c\", info = \"no\")",
        "component(name = \"d\", info = \"no\")",
        "component(name = \"ZZZ\", info = \"yes\")",
    ]);
    t.update(&["//:bin"]).expect("update");
    let target = "//:bin";
    let files_to_build = t.files_to_build(target);
    assert_contains_exactly(
        &base_artifact_names(&files_to_build),
        &["AAA.info", "a.info", "b.info", "ZZZ.info"],
    );
    t.clear_events();
    t.clean_skyframe();
    t.update(&["//:bin_alias_chain"]).expect("update");
    let target_alias_chain = "//:bin_alias_chain";
    let files_to_build_alias_chain = t.files_to_build(target_alias_chain);
    assert_contains_exactly(
        &base_artifact_names(&files_to_build_alias_chain),
        &["AAA.info", "a.info", "b.info", "ZZZ.info"],
    );
    // Especially when going through an alias chain, an aspect should still visit the nodes once.
    t.assert_contains_event_with_frequency("aspect visiting target: @@//:AAA", 1);
    t.assert_contains_event_with_frequency("aspect visiting target: @@//:ZZZ", 1);
    t.assert_contains_event_with_frequency("aspect visiting target: @@//:b", 1);
    t.assert_contains_event_with_frequency("aspect visiting target: @@//:a", 1);
    t.assert_does_not_contain_event("aspect visiting target: @@//:c");
    t.assert_does_not_contain_event("aspect visiting target: @@//:d");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesRealDepsTest.java:810 materializerToAlias_works
#[test]
fn materializer_to_alias_works() {
    let mut t = set_up();
    write_basic_materializer_rule(&mut t);
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
            "        \":a_alias\",",
            "        \":b_alias\",",
            "        \":c\",",
            "        \":d\",",
            "        \":d_alias\",",
            "    ],",
            ")",
            "",
            "alias(name = \"a_alias\", actual = \"a\")",
            "alias(name = \"b_alias\", actual = \"b\")",
            "alias(name = \"d_alias\", actual = \"d\")",
            "",
            "component(name = \"aaa\", info = \"yes\")",
            "component(name = \"a\", info = \"yes\")",
            "component(name = \"b\", info = \"yes\")",
            "component(name = \"c\", info = \"no\")",
            "component(name = \"d\", info = \"no\")",
            "component(name = \"zzz\", info = \"yes\")",
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

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesRealDepsTest.java:860 aliasToMaterializerToAliasToMaterializerToAlias_works
#[test]
fn alias_to_materializer_to_alias_to_materializer_to_alias_works() {
    let mut t = set_up();
    t.write(
        "defs.bzl",
        &[
            "# Component ######################################",
            "",
            "ComponentInfo = provider(fields = [\"output\", \"info\"])",
            "",
            "def _component_impl(ctx):",
            "    f = ctx.actions.declare_file(ctx.label.name + \".txt\")",
            "    ctx.actions.write(f, ctx.label.name)",
            "    return ComponentInfo(output = f, info = ctx.attr.info)",
            "",
            "component = rule(",
            "    implementation = _component_impl,",
            "    provides = [ComponentInfo],",
            "    attrs = {",
            "        \"info\": attr.string(),",
            "    }",
            ")",
            "",
            "# Component selector #############################",
            "",
            "def _component_selector_impl(ctx):",
            "    return MaterializedDepsInfo(deps = ctx.attr.components)",
            "",
            "component_selector = materializer_rule(",
            "    implementation = _component_selector_impl,",
            "    allow_real_deps = True,",
            "    attrs = {",
            "        \"components\": attr.label_list(),",
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
            "    components = [\":alias_to_materializer_to_alias\"],",
            ")",
            "",
            "alias(",
            "    name = \"alias_to_materializer_to_alias\",",
            "    actual = \":materializer_to_alias\",",
            ")",
            "",
            "component_selector(",
            "    name = \"materializer_to_alias\",",
            "    components = [\":a_alias\"],",
            ")",
            "",
            "alias(",
            "    name = \"a_alias\",",
            "    actual = \":a\",",
            ")",
            "",
            "component(name = \"aaa\", info = \"yes\")",
            "component(name = \"a\", info = \"yes\")",
            "component(name = \"zzz\", info = \"yes\")",
        ],
    );
    t.update(&["//:bin"]).expect("update");
    let target = "//:bin";
    let files_to_build = t.files_to_build(target);
    let artifact_names = base_artifact_names(&files_to_build);
    assert_contains_exactly(&artifact_names, &["aaa.txt", "a.txt", "zzz.txt"]);
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesRealDepsTest.java:1040 materializerRulesUnderSplitTransition_works
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
        "        \":a\",",
        "        \":b\",",
        "        \":c\",",
        "        \":d\",",
        "    ],",
        ")",
        "",
        "component(name = \"a\", info = \"yes\")",
        "component(name = \"b\", info = \"yes\")",
        "component(name = \"c\", info = \"no\")",
        "component(name = \"d\", info = \"no\")",
    ]);
    t.update(&["//:bin"]).expect("update");
    let target = "//:bin";
    let files_to_build = t.files_to_build(target);
    let artifact_names = base_artifact_names(&files_to_build);
    assert_contains_at_least(&artifact_names, &["a.txt", "b.txt"]);
    assert_contains_none_of(&artifact_names, &["c.txt", "d.txt"]);
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesRealDepsTest.java:1088 materializerRulesUnderSplitTransitionAndSelect_works
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
        "            \":a\",",
        "            \":b\",",
        "            \":f\",",
        "            \":g\",",
        "        ],",
        "        \":config_setting_bar\": [",
        "            \":c\",",
        "            \":d\",",
        "            \":e\",",
        "            \":f\",",
        "            \":g\",",
        "        ],",
        "    }),",
        ")",
        "",
        "component(name = \"a\", info = \"yes\")",
        "component(name = \"b\", info = \"yes\")",
        "component(name = \"c\", info = \"yes\")",
        "component(name = \"d\", info = \"yes\")",
        "component(name = \"e\", info = \"yes\")",
        "component(name = \"f\", info = \"no\")",
        "component(name = \"g\", info = \"no\")",
    ]);
    t.update(&["//:bin"]).expect("update");
    let target = "//:bin";
    let files_to_build = t.files_to_build(target);
    let artifact_names = base_artifact_names(&files_to_build);
    assert_contains_at_least(
        &artifact_names,
        &["a.txt", "b.txt", "c.txt", "d.txt", "e.txt"],
    );
    assert_contains_none_of(&artifact_names, &["f.txt", "g.txt"]);
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesRealDepsTest.java:1155 materializerRulesPropagatesValidationActions_works
#[test]
fn materializer_rules_propagates_validation_actions_works() {
    let mut t = set_up();
    t.write(
        "defs.bzl",
        &[
            "# Component ######################################",
            "",
            "ComponentInfo = provider(fields = [\"output\", \"info\"])",
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
            "        ComponentInfo(output = f, info = ctx.attr.info),",
            "        OutputGroupInfo(_validation = depset([validation_output])),",
            "    ]",
            "",
            "component = rule(",
            "    implementation = _component_impl,",
            "    provides = [ComponentInfo],",
            "    attrs = {",
            "        \"info\": attr.string(),",
            "    }",
            ")",
            "",
            "# Component selector #############################",
            "",
            "def _component_selector_impl(ctx):",
            "    selected = []",
            "    for c in ctx.attr.all_components:",
            "        if \"yes\" in c[ComponentInfo].info:",
            "            selected.append(c)",
            "    return MaterializedDepsInfo(deps = selected)",
            "",
            "component_selector = materializer_rule(",
            "    implementation = _component_selector_impl,",
            "    allow_real_deps = True,",
            "    attrs = {",
            "        \"all_components\": attr.label_list(),",
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
            "        \":a\",",
            "        \":b\",",
            "        \":c\",",
            "        \":d\",",
            "    ],",
            ")",
            "",
            "component(name = \"aaa\", info = \"yes\")",
            "component(name = \"a\", info = \"yes\")",
            "component(name = \"b\", info = \"yes\")",
            "component(name = \"c\", info = \"no\")",
            "component(name = \"d\", info = \"no\")",
            "component(name = \"zzz\", info = \"yes\")",
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
            "a.validation",
            "b.validation",
            "zzz.validation",
        ],
    );
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesRealDepsTest.java:1312 materializerRuleVisibilityViolation_throwsError
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
            "        \":a\",",
            "        \":b\",",
            "        \":c\",",
            "        \":d\",",
            "    ],",
            "    visibility = [\"//binary1:__pkg__\"],",
            ")",
            "",
            "component(name = \"a\", info = \"yes\", visibility = [\"//:__subpackages__\"])",
            "component(name = \"b\", info = \"yes\", visibility = [\"//:__subpackages__\"])",
            "component(name = \"c\", info = \"no\", visibility = [\"//:__subpackages__\"])",
            "component(name = \"d\", info = \"no\", visibility = [\"//:__subpackages__\"])",
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

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesRealDepsTest.java:1379 materializerRuleMaterializedTargetVisibilityViolation_throwsError
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
            "        \":a\",",
            "        \":b\",",
            "        \":c\",",
            "        \":d\",",
            "    ],",
            "    visibility = [\"//binary:__pkg__\"],",
            ")",
            "",
            "component(name = \"a\", info = \"yes\", visibility = [\"//visibility:private\"])",
            "component(name = \"b\", info = \"yes\", visibility = [\"//visibility:private\"])",
            "component(name = \"c\", info = \"no\", visibility = [\"//visibility:private\"])",
            "component(name = \"d\", info = \"no\", visibility = [\"//visibility:private\"])",
        ],
    );
    // The materializer target is visible to bin, but the materialized targets are not.
    // reporter.removeHandler(failFastHandler): a failing update returns an error.
    // assertThrows(ViewCreationFailedException)
    assert!(t.update(&["//binary:bin"]).is_err());
    t.assert_contains_event("ERROR /workspace/binary/BUILD:3:7: in binary rule //binary:bin: Visibility error:\ntarget '//components:a' is not visible from\ntarget '//binary:bin'\n");
    t.assert_contains_event("ERROR /workspace/binary/BUILD:3:7: in binary rule //binary:bin: Visibility error:\ntarget '//components:b' is not visible from\ntarget '//binary:bin'\n");
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesRealDepsTest.java:66 writeBasicMaterializerRule
fn write_basic_materializer_rule(t: &mut Fixture) {
    t.write(
        "defs.bzl",
        &[
            "# Component ######################################",
            "",
            "ComponentInfo = provider(fields = [\"output\", \"info\"])",
            "",
            "def _component_impl(ctx):",
            "    f = ctx.actions.declare_file(ctx.label.name + \".txt\")",
            "    ctx.actions.write(f, ctx.label.name)",
            "    return ComponentInfo(output = f, info = ctx.attr.info)",
            "",
            "component = rule(",
            "    implementation = _component_impl,",
            "    provides = [ComponentInfo],",
            "    attrs = {",
            "        \"info\": attr.string(),",
            "    }",
            ")",
            "",
            "# Component selector #############################",
            "",
            "def _component_selector_impl(ctx):",
            "    selected = []",
            "    for c in ctx.attr.all_components:",
            "        if \"yes\" in c[ComponentInfo].info:",
            "            selected.append(c)",
            "    return MaterializedDepsInfo(deps = selected)",
            "",
            "component_selector = materializer_rule(",
            "    implementation = _component_selector_impl,",
            "    allow_real_deps = True,",
            "    attrs = {",
            "        \"all_components\": attr.label_list(),",
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
}

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesRealDepsTest.java:959 writeMaterializerSplitTransitionBzlFile
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
        "ComponentInfo = provider(fields = [\"output\", \"info\"])",
        "",
        "def _component_impl(ctx):",
        "    f = ctx.actions.declare_file(ctx.label.name + \".txt\")",
        "    ctx.actions.write(f, ctx.label.name)",
        "    return ComponentInfo(output = f, info = ctx.attr.info)",
        "",
        "component = rule(",
        "    implementation = _component_impl,",
        "    provides = [ComponentInfo],",
        "    attrs = {",
        "        \"info\": attr.string(),",
        "    }",
        ")",
        "",
        "# Component selector ###################################################",
        "",
        "def _component_selector_impl(ctx):",
        "    selected = []",
        "    for c in ctx.attr.all_components:",
        "        if \"yes\" in c[ComponentInfo].info:",
        "            selected.append(c)",
        "    return MaterializedDepsInfo(deps = selected)",
        "",
        "component_selector = materializer_rule(",
        "    implementation = _component_selector_impl,",
        "    allow_real_deps = True,",
        "    attrs = {",
        "        \"all_components\": attr.label_list(),",
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

// Java: src/test/java/com/google/devtools/build/lib/analysis/MaterializerRulesRealDepsTest.java:1261 writeVisibilityDefsBzlFile
fn write_visibility_defs_bzl_file(t: &mut Fixture) {
    t.write(
        "defs.bzl",
        &[
            "# Component ######################################",
            "",
            "ComponentInfo = provider(fields = [\"info\"])",
            "",
            "def _component_impl(ctx):",
            "    return ComponentInfo(info = ctx.attr.info)",
            "",
            "component = rule(",
            "    implementation = _component_impl,",
            "    provides = [ComponentInfo],",
            "    attrs = {",
            "        \"info\": attr.string(),",
            "    }",
            ")",
            "",
            "# Component selector #############################",
            "",
            "def _component_selector_impl(ctx):",
            "    selected = []",
            "    for c in ctx.attr.all_components:",
            "        if \"yes\" in c[ComponentInfo].info:",
            "            selected.append(c)",
            "    return MaterializedDepsInfo(deps = selected)",
            "",
            "component_selector = materializer_rule(",
            "    implementation = _component_selector_impl,",
            "    allow_real_deps = True,",
            "    attrs = {",
            "        \"all_components\": attr.label_list(),",
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
