//! Port of `src/test/java/com/google/devtools/build/lib/rules/platform/PlatformInfoApiTest.java`
//! (a `PlatformTestCase`): the Starlark API of `PlatformInfo` providers.

use blaze_analysis::platform::constraint_setting_info::ConstraintSettingInfo;
use blaze_analysis::platform::constraint_value_info::ConstraintValueInfo;
use blaze_cmdline::Label;
use indexmap::IndexMap;

use crate::testing::{
    BuildViewTestCase, assert_contains_exactly, assert_contains_exactly_in_order,
};
use crate::tests::platform_test_case::{constraint_builder, fetch_platform_info, platform_builder};

fn label(s: &str) -> Label {
    Label::parse_canonical_unchecked(s)
}

/// `ImmutableMap.of(k1, v1, ...)`.
fn map(entries: &[(&str, &str)]) -> IndexMap<String, String> {
    entries
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect()
}

/// `checkError(packageName, ruleName, expectedErrorMessage, lines.toArray(new String[] {}))`.
fn check_error_lines(
    t: &mut BuildViewTestCase,
    package_name: &str,
    rule_name: &str,
    expected_error_message: &str,
    lines: &[String],
) {
    let lines: Vec<&str> = lines.iter().map(String::as_str).collect();
    t.check_error(package_name, rule_name, expected_error_message, &lines);
}

// Java: PlatformInfoApiTest.constructor
#[test]
fn constructor() {
    let mut t = BuildViewTestCase::new();
    constraint_builder("//foo:basic")
        .add_constraint_value("value1")
        .write(&t);
    platform_builder("//foo:my_platform")
        .add_constraint("value1")
        .write(&t);
    t.assert_no_events();

    let platform_info = fetch_platform_info(&mut t, "//foo:my_platform").expect("isNotNull");
    let constraint_setting = ConstraintSettingInfo::create(label("//foo:basic"));
    let constraint_value =
        ConstraintValueInfo::create(constraint_setting.clone(), label("//foo:value1"));
    assert_eq!(
        platform_info.constraints().get(&constraint_setting),
        Some(constraint_value)
    );
    assert!(platform_info.remote_execution_properties().is_empty());
}

// Java: PlatformInfoApiTest.tooManyParentsError
#[test]
fn too_many_parents_error() {
    let mut t = BuildViewTestCase::new();
    let mut lines: Vec<String> = Vec::new();
    lines.extend(platform_builder("//foo:parent_platform1").lines());
    lines.extend(platform_builder("//foo:parent_platform2").lines());
    lines.extend(
        [
            "platform(name = 'my_platform',\n",
            "  parents = [\n",
            "    ':parent_platform1',\n",
            "    ':parent_platform2',\n",
            "  ])",
        ]
        .iter()
        .map(|s| s.to_string()),
    );

    check_error_lines(
        &mut t,
        "foo",
        "my_platform",
        "in parents attribute of platform rule //foo:my_platform: \
         parents attribute must have a single value",
        &lines,
    );
}

// Java: PlatformInfoApiTest.constraints_overlappingError
#[test]
fn constraints_overlapping_error() {
    let mut t = BuildViewTestCase::new();
    let mut lines: Vec<String> = Vec::new();
    lines.extend(
        constraint_builder("//foo:basic")
            .add_constraint_value("value1")
            .add_constraint_value("value2")
            .lines(),
    );
    lines.extend(
        platform_builder("//foo:my_platform")
            .add_constraint("value1")
            .add_constraint("value2")
            .lines(),
    );

    check_error_lines(
        &mut t,
        "foo",
        "my_platform",
        "Duplicate constraint values detected: \
         constraint_setting //foo:basic has [//foo:value1, //foo:value2]",
        &lines,
    );
}

// Java: PlatformInfoApiTest.constraints_repeatedViaAlias
#[test]
fn constraints_repeated_via_alias() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "foo/BUILD",
        &[r#"constraint_setting(name = "basic")

constraint_value(
    name = "value1",
    constraint_setting = ":basic",
)

alias(
    name = "value1_alias",
    actual = ":value1",
)

platform(
    name = "my_platform",
    constraint_values = [
        ":value1",
        ":value1_alias",
    ],
)
"#],
    );

    let platform_info = fetch_platform_info(&mut t, "//foo:my_platform");
    t.assert_no_events();
    let platform_info = platform_info.expect("isNotNull");
    let constraint_setting = ConstraintSettingInfo::create(label("//foo:basic"));
    let constraint_value =
        ConstraintValueInfo::create(constraint_setting.clone(), label("//foo:value1"));
    assert_eq!(
        platform_info.constraints().get(&constraint_setting),
        Some(constraint_value)
    );
}

const LIBC_BUILD: &str = r#"constraint_setting(name = "libc")

constraint_value(
    name = "glibc",
    constraint_setting = ":libc",
)
"#;

const LIBC_GLIBC_BUILD: &str = r#"constraint_setting(
    name = "version",
    refines_constraint_value = "//libc:glibc",
)

constraint_value(
    name = "2.42",
    constraint_setting = ":version",
)
"#;

// Java: PlatformInfoApiTest.constraints_refinesConstraintValue_satisfied
#[test]
fn constraints_refines_constraint_value_satisfied() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file("libc/BUILD", &[LIBC_BUILD]);
    t.scratch.file("libc/glibc/BUILD", &[LIBC_GLIBC_BUILD]);
    t.scratch.file(
        "platforms/BUILD",
        &[r#"platform(
    name = "glibc242",
    constraint_values = [
        "//libc:glibc",
        "//libc/glibc:2.42",
    ],
)
"#],
    );

    let platform_info = fetch_platform_info(&mut t, "//platforms:glibc242");
    assert!(platform_info.is_some());
}

// Java: PlatformInfoApiTest.constraints_refinesConstraintValue_alias_satisfied
#[test]
fn constraints_refines_constraint_value_alias_satisfied() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "libc/BUILD",
        &[r#"constraint_setting(name = "libc")

constraint_value(
    name = "glibc",
    constraint_setting = ":libc",
)

alias(
    name = "glibc_alias",
    actual = ":glibc",
)
"#],
    );
    t.scratch.file(
        "libc/glibc/BUILD",
        &[r#"constraint_setting(
    name = "version",
    refines_constraint_value = "//libc:glibc_alias",
)

constraint_value(
    name = "2.42",
    constraint_setting = ":version",
)
"#],
    );
    // The refined value is declared by its real label even though refines_constraint_value used
    // an alias.
    t.scratch.file(
        "platforms/BUILD",
        &[r#"platform(
    name = "glibc242",
    constraint_values = [
        "//libc:glibc",
        "//libc/glibc:2.42",
    ],
)
"#],
    );

    let platform_info = fetch_platform_info(&mut t, "//platforms:glibc242");
    assert!(platform_info.is_some());
}

// Java: PlatformInfoApiTest.constraints_refinesConstraintValue_missingRefined_error
#[test]
fn constraints_refines_constraint_value_missing_refined_error() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file("libc/BUILD", &[LIBC_BUILD]);
    t.scratch.file("libc/glibc/BUILD", &[LIBC_GLIBC_BUILD]);
    t.check_error(
        "platforms",
        "broken",
        "constraint_value //libc/glibc:2.42 refines //libc:glibc, but platform //platforms:broken \
         does not set the latter.",
        &[r#"platform(
    name = "broken",
    constraint_values = ["//libc/glibc:2.42"],
)
"#],
    );
    t.assert_contains_event("buildozer 'add constraint_values //libc:glibc' //platforms:broken");
}

// Java: PlatformInfoApiTest.constraints_refinesConstraintValue_wrongValue_error
#[test]
fn constraints_refines_constraint_value_wrong_value_error() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file(
        "libc/BUILD",
        &[r#"constraint_setting(name = "libc")

constraint_value(
    name = "glibc",
    constraint_setting = ":libc",
)

constraint_value(
    name = "musl",
    constraint_setting = ":libc",
)
"#],
    );
    t.scratch.file("libc/glibc/BUILD", &[LIBC_GLIBC_BUILD]);
    // The platform sets a conflicting value (musl) for the refined setting.
    t.check_error(
        "platforms",
        "wrong",
        "constraint_value //libc/glibc:2.42 refines //libc:glibc, but platform //platforms:wrong \
         sets the conflicting constraint_value //libc:musl for //libc",
        &[r#"platform(
    name = "wrong",
    constraint_values = [
        "//libc:musl",
        "//libc/glibc:2.42",
    ],
)
"#],
    );
    // A conflicting value is a likely-real contradiction, so no mechanical buildozer fixup is
    // suggested.
    t.assert_does_not_contain_event("buildozer");
}

// Java: PlatformInfoApiTest.constraints_refinesConstraintValue_defaultNotEnforced
#[test]
fn constraints_refines_constraint_value_default_not_enforced() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file("libc/BUILD", &[LIBC_BUILD]);
    t.scratch.file(
        "libc/glibc/BUILD",
        &[r#"constraint_setting(
    name = "version",
    default_constraint_value = ":unknown",
    refines_constraint_value = "//libc:glibc",
)

constraint_value(
    name = "unknown",
    constraint_setting = ":version",
)

constraint_value(
    name = "2.42",
    constraint_setting = ":version",
)
"#],
    );
    // Platform may explicitly set the *default* refining value without declaring the refined one.
    t.scratch.file(
        "platforms/BUILD",
        &[r#"platform(
    name = "default_only",
    constraint_values = ["//libc/glibc:unknown"],
)
"#],
    );

    let platform_info = fetch_platform_info(&mut t, "//platforms:default_only");
    assert!(platform_info.is_some());
}

// Java: PlatformInfoApiTest.constraints_refinesConstraintValue_inheritedFromParent
#[test]
fn constraints_refines_constraint_value_inherited_from_parent() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file("libc/BUILD", &[LIBC_BUILD]);
    t.scratch.file("libc/glibc/BUILD", &[LIBC_GLIBC_BUILD]);
    t.scratch.file(
        "platforms/BUILD",
        &[r#"platform(
    name = "parent",
    constraint_values = ["//libc:glibc"],
)

platform(
    name = "child",
    parents = [":parent"],
    constraint_values = ["//libc/glibc:2.42"],
)
"#],
    );

    let platform_info = fetch_platform_info(&mut t, "//platforms:child");
    assert!(platform_info.is_some());
}

// Java: PlatformInfoApiTest.constraints_invalidTarget_error
#[test]
fn constraints_invalid_target_error() {
    let mut t = BuildViewTestCase::new();
    t.check_error(
        "foo",
        "my_platform",
        // TODO: https://github.com/bazelbuild/bazel/issues/23126 - Have a better error message.
        // Something like "Invalid dependency :lib does not provide ConstraintValueInfo"
        "errors encountered while analyzing target",
        &[r#"filegroup(name = "lib")

platform(
    name = "my_platform",
    constraint_values = [
        ":lib",
    ],
)
"#],
    );
}

// Java: PlatformInfoApiTest.constraints_parent
#[test]
fn constraints_parent() {
    let mut t = BuildViewTestCase::new();
    constraint_builder("//foo:setting1")
        .add_constraint_value("value1")
        .write(&t);
    constraint_builder("//foo:setting2")
        .add_constraint_value("value2")
        .write(&t);
    platform_builder("//foo:parent_platform")
        .add_constraint("value1")
        .write(&t);
    platform_builder("//foo:my_platform")
        .set_parent("//foo:parent_platform")
        .add_constraint("value2")
        .write(&t);
    t.assert_no_events();

    let platform_info = fetch_platform_info(&mut t, "//foo:my_platform").expect("isNotNull");
    let constraint_setting1 = ConstraintSettingInfo::create(label("//foo:setting1"));
    let constraint_value1 =
        ConstraintValueInfo::create(constraint_setting1.clone(), label("//foo:value1"));
    assert_eq!(
        platform_info.constraints().get(&constraint_setting1),
        Some(constraint_value1)
    );
    let constraint_setting2 = ConstraintSettingInfo::create(label("//foo:setting2"));
    let constraint_value2 =
        ConstraintValueInfo::create(constraint_setting2.clone(), label("//foo:value2"));
    assert_eq!(
        platform_info.constraints().get(&constraint_setting2),
        Some(constraint_value2)
    );
}

// Java: PlatformInfoApiTest.constraints_parent_override
#[test]
fn constraints_parent_override() {
    let mut t = BuildViewTestCase::new();
    constraint_builder("//foo:setting1")
        .add_constraint_value("value1a")
        .add_constraint_value("value1b")
        .write(&t);
    platform_builder("//foo:parent_platform")
        .add_constraint("value1a")
        .write(&t);
    platform_builder("//foo:my_platform")
        .add_constraint("value1b")
        .write(&t);
    t.assert_no_events();

    let platform_info = fetch_platform_info(&mut t, "//foo:my_platform").expect("isNotNull");
    let constraint_setting1 = ConstraintSettingInfo::create(label("//foo:setting1"));
    let constraint_value1 =
        ConstraintValueInfo::create(constraint_setting1.clone(), label("//foo:value1b"));
    assert_eq!(
        platform_info.constraints().get(&constraint_setting1),
        Some(constraint_value1)
    );
}

// Java: PlatformInfoApiTest.remoteExecutionProperties
#[test]
fn remote_execution_properties() {
    let mut t = BuildViewTestCase::new();
    platform_builder("//foo:my_platform")
        .set_remote_execution_properties("foo: val1")
        .write(&t);
    t.assert_no_events();

    let platform_info = fetch_platform_info(&mut t, "//foo:my_platform").expect("isNotNull");
    assert_eq!(platform_info.remote_execution_properties(), "foo: val1");
}

// Java: PlatformInfoApiTest.remoteExecutionProperties_parent_entireOverride
#[test]
fn remote_execution_properties_parent_entire_override() {
    let mut t = BuildViewTestCase::new();
    platform_builder("//foo:parent_platform")
        .set_remote_execution_properties("parent props")
        .write(&t);
    platform_builder("//foo:my_platform")
        .set_parent("//foo:parent_platform")
        .set_remote_execution_properties("child props")
        .write(&t);
    t.assert_no_events();

    let platform_info = fetch_platform_info(&mut t, "//foo:my_platform").expect("isNotNull");
    assert_eq!(platform_info.remote_execution_properties(), "child props");
}

// Java: PlatformInfoApiTest.remoteExecutionProperties_parent_merge
#[test]
fn remote_execution_properties_parent_merge() {
    let mut t = BuildViewTestCase::new();
    platform_builder("//foo:parent_platform")
        .set_remote_execution_properties("parent props")
        .write(&t);
    platform_builder("//foo:my_platform")
        .set_parent("//foo:parent_platform")
        .set_remote_execution_properties("child ({PARENT_REMOTE_EXECUTION_PROPERTIES}) props")
        .write(&t);
    t.assert_no_events();

    let platform_info = fetch_platform_info(&mut t, "//foo:my_platform").expect("isNotNull");
    assert_eq!(
        platform_info.remote_execution_properties(),
        "child (parent props) props"
    );
}

// Java: PlatformInfoApiTest.remoteExecutionProperties_parentSpecifiesExecProperties_error
#[test]
fn remote_execution_properties_parent_specifies_exec_properties_error() {
    let mut t = BuildViewTestCase::new();
    let props_parent = map(&[("k1", "v1"), ("k2", "v2")]);
    platform_builder("//foo:parent_platform")
        .set_exec_properties(props_parent)
        .write(&t);
    let builder = platform_builder("//bar:my_platform")
        .set_parent("//foo:parent_platform")
        .set_remote_execution_properties("properties");

    check_error_lines(
        &mut t,
        "bar",
        "my_platform",
        "Platform specifies remote_execution_properties but its parent specifies exec_properties. \
         Prefer exec_properties over the deprecated remote_execution_properties.",
        &builder.lines(),
    );
}

// Java: PlatformInfoApiTest.execProperties
#[test]
fn exec_properties() {
    let mut t = BuildViewTestCase::new();
    let props = map(&[("k1", "v1"), ("k2", "v2")]);
    platform_builder("//foo:my_platform")
        .set_exec_properties(props.clone())
        .write(&t);
    t.assert_no_events();

    let platform_info = fetch_platform_info(&mut t, "//foo:my_platform").expect("isNotNull");
    assert_eq!(platform_info.exec_properties(), &props);
}

// Java: PlatformInfoApiTest.execProperties_conflictingProperties_error
#[test]
fn exec_properties_conflicting_properties_error() {
    let mut t = BuildViewTestCase::new();
    let props = map(&[("k1", "v1"), ("k2", "v2")]);
    let builder = platform_builder("//foo:my_platform")
        .set_exec_properties(props)
        .set_remote_execution_properties("child props");

    check_error_lines(
        &mut t,
        "foo",
        "my_platform",
        "Platform contains both remote_execution_properties and exec_properties. Prefer \
         exec_properties over the deprecated remote_execution_properties.",
        &builder.lines(),
    );
}

// Java: PlatformInfoApiTest.execProperties_parent
#[test]
fn exec_properties_parent() {
    let mut t = BuildViewTestCase::new();
    let props = map(&[("k1", "v1"), ("k2", "v2")]);
    platform_builder("//foo:parent_platform")
        .set_exec_properties(props.clone())
        .write(&t);
    platform_builder("//foo:my_platform")
        .set_parent("//foo:parent_platform")
        .write(&t);
    t.assert_no_events();

    let platform_info = fetch_platform_info(&mut t, "//foo:my_platform").expect("isNotNull");
    assert_eq!(platform_info.exec_properties(), &props);
}

// Java: PlatformInfoApiTest.execProperties_parent_merged
#[test]
fn exec_properties_parent_merged() {
    let mut t = BuildViewTestCase::new();
    let props_parent = map(&[("k1", "v1"), ("k2", "v2")]);
    let props_child = map(&[("k2", "child_v2"), ("k3", "child_v3")]);
    platform_builder("//foo:parent_platform")
        .set_exec_properties(props_parent)
        .write(&t);
    platform_builder("//foo:my_platform")
        .set_parent("//foo:parent_platform")
        .set_exec_properties(props_child)
        .write(&t);
    t.assert_no_events();

    let platform_info = fetch_platform_info(&mut t, "//foo:my_platform").expect("isNotNull");
    let expected = map(&[("k1", "v1"), ("k2", "child_v2"), ("k3", "child_v3")]);
    assert_eq!(platform_info.exec_properties(), &expected);
}

// Java: PlatformInfoApiTest.execProperties_parentSpecifiesRemoteExecutionProperties_error
#[test]
fn exec_properties_parent_specifies_remote_execution_properties_error() {
    let mut t = BuildViewTestCase::new();
    let props_child = map(&[("k2", "child_v2"), ("k3", "child_v3")]);
    platform_builder("//foo:parent_platform")
        .set_remote_execution_properties("properties")
        .write(&t);
    let builder = platform_builder("//bar:my_platform")
        .set_parent("//foo:parent_platform")
        .set_exec_properties(props_child);

    check_error_lines(
        &mut t,
        "bar",
        "my_platform",
        "Platform specifies exec_properties but its parent //foo:parent_platform specifies \
         remote_execution_properties. Prefer exec_properties over the deprecated \
         remote_execution_properties.",
        &builder.lines(),
    );
}

// Java: PlatformInfoApiTest.flags
#[test]
fn flags() {
    let mut t = BuildViewTestCase::new();
    platform_builder("//foo:basic")
        .add_flags(&["--cpu=k8", "--//starlark:flag=other"])
        .write(&t);

    let platform_info = fetch_platform_info(&mut t, "//foo:basic").expect("isNotNull");
    assert_contains_exactly(
        platform_info.flags(),
        &["--cpu=k8", "--//starlark:flag=other"],
    );
}

// Java: PlatformInfoApiTest.flags_parent
#[test]
fn flags_parent() {
    let mut t = BuildViewTestCase::new();
    platform_builder("//foo:parent")
        .add_flags(&["--cpu=k8"])
        .write(&t);
    platform_builder("//foo:basic")
        .set_parent("//foo:parent")
        .write(&t);

    let platform_info = fetch_platform_info(&mut t, "//foo:basic").expect("isNotNull");
    assert_contains_exactly(platform_info.flags(), &["--cpu=k8"]);
}

// Java: PlatformInfoApiTest.flags_parent_merged
#[test]
fn flags_parent_merged() {
    let mut t = BuildViewTestCase::new();
    platform_builder("//foo:parent")
        .add_flags(&["--cpu=k8"])
        .write(&t);
    platform_builder("//foo:basic")
        .set_parent("//foo:parent")
        .add_flags(&["--//starlark:flag=other"])
        .write(&t);

    let platform_info = fetch_platform_info(&mut t, "//foo:basic").expect("isNotNull");
    assert_contains_exactly(
        platform_info.flags(),
        &["--cpu=k8", "--//starlark:flag=other"],
    );
}

// Java: PlatformInfoApiTest.flags_parent_override
#[test]
fn flags_parent_override() {
    let mut t = BuildViewTestCase::new();
    platform_builder("//foo:parent")
        .add_flags(&["--cpu=arm"])
        .write(&t);
    platform_builder("//foo:basic")
        .set_parent("//foo:parent")
        .add_flags(&["--cpu=k8"])
        .write(&t);

    let platform_info = fetch_platform_info(&mut t, "//foo:basic").expect("isNotNull");
    assert_contains_exactly_in_order(platform_info.flags(), &["--cpu=arm", "--cpu=k8"]);
}
