//! Port of `src/test/java/com/google/devtools/build/lib/analysis/ResolvedToolchainContextTest.java`
//! (a `ToolchainTestCase`): `ResolvedToolchainContext.load`.

use std::sync::Arc;

use blaze_analysis::configured_target_and_data::ConfiguredTargetAndData;
use blaze_analysis::info::downcast_info;
use blaze_analysis::platform::template_variable_info::TemplateVariableInfo;
use blaze_analysis::platform::toolchain_info::FrozenToolchainInfo;
use blaze_analysis::platform::{PlatformInfo, ToolchainTypeInfo};
use blaze_analysis::toolchains::{
    ResolvedToolchainContext, ToolchainContextKey, ToolchainTypeRequirement,
    UnloadedToolchainContextImpl,
};
use blaze_cmdline::Label;
use indexmap::IndexMap;
use starlark::values::OwnedFrozenValue;

use crate::tests::toolchain_test_case::{ToolchainTestCase, assert_has_toolchain_type, label};

/// `getConfiguredTargetAndData(label, targetConfig)`.
fn get_configured_target_and_data(
    t: &mut ToolchainTestCase,
    label: &str,
) -> ConfiguredTargetAndData {
    let config = t.get_target_configuration();
    t.get_configured_target_and_data_in(label, Some(&config))
        .unwrap_or_else(|| panic!("no configured target {label}"))
}

/// `assertThat(toolchainContext).forToolchainType(label).getValue(name).isEqualTo(expected)`
/// (`ToolchainInfoSubject.getValue`).
fn assert_toolchain_value(
    toolchain_context: &ResolvedToolchainContext,
    toolchain_type: &Label,
    name: &str,
    expected: &str,
) {
    let toolchain = toolchain_context
        .for_toolchain_type(toolchain_type)
        .unwrap_or_else(|| panic!("forToolchainType({toolchain_type}): expected not null"));
    let info = downcast_info::<FrozenToolchainInfo>(&toolchain)
        .expect("ClassCastException: ToolchainInfo");
    let value = info
        .get_value(name)
        .unwrap_or_else(|| panic!("no value {name}"));
    assert_eq!(value.unpack_str(), Some(expected), "getValue({name})");
}

/// `assertThat(toolchainContext).forToolchainType(label).isNull()`.
fn assert_toolchain_is_null(toolchain_context: &ResolvedToolchainContext, toolchain_type: &Label) {
    let toolchain: Option<OwnedFrozenValue> = toolchain_context.for_toolchain_type(toolchain_type);
    assert!(
        toolchain.is_none(),
        "forToolchainType({toolchain_type}): expected null"
    );
}

/// `ToolchainContextKey.key().configurationKey(targetConfigKey).toolchainTypes(types).build()`.
fn toolchain_context_key(
    t: &mut ToolchainTestCase,
    toolchain_types: &[ToolchainTypeRequirement],
) -> ToolchainContextKey {
    ToolchainContextKey::key()
        .configuration_key(t.target_config_key())
        .toolchain_types(toolchain_types.iter().cloned())
        .build()
}

/// `UnloadedToolchainContextImpl.builder(key)...build()` with the linux platform as execution
/// and target platform.
fn unloaded_toolchain_context(
    t: &ToolchainTestCase,
    key: ToolchainContextKey,
    toolchain_types: &[ToolchainTypeRequirement],
    requested_label_to_toolchain_type: &[(Label, ToolchainTypeInfo)],
    toolchain_type_to_resolved: Option<Vec<(ToolchainTypeInfo, Label)>>,
) -> UnloadedToolchainContextImpl {
    let linux_platform: Arc<PlatformInfo> = Arc::new(t.linux_platform.clone());
    let mut builder = UnloadedToolchainContextImpl::builder(key);
    builder
        .set_execution_platform(linux_platform.clone())
        .set_target_platform(linux_platform)
        .set_toolchain_types(toolchain_types.to_vec())
        .set_requested_label_to_toolchain_type(
            requested_label_to_toolchain_type
                .iter()
                .cloned()
                .collect::<IndexMap<_, _>>(),
        );
    if let Some(resolved) = toolchain_type_to_resolved {
        builder.set_toolchain_type_to_resolved(resolved);
    }
    builder.build()
}

// Java: ResolvedToolchainContextTest.load
#[test]
fn load() {
    let mut t = ToolchainTestCase::new();
    t.add_toolchain(
        "extra",
        "extra_toolchain_linux",
        &["//constraints:linux"],
        &["//constraints:linux"],
        "baz",
    );

    let test_toolchain_type = t.test_toolchain_type.clone();
    let toolchain_context_key =
        toolchain_context_key(&mut t, std::slice::from_ref(&test_toolchain_type));

    // Create a static UnloadedToolchainContext.
    let unloaded_toolchain_context = unloaded_toolchain_context(
        &t,
        toolchain_context_key,
        &[test_toolchain_type],
        &[(
            t.test_toolchain_type_label.clone(),
            t.test_toolchain_type_info.clone(),
        )],
        Some(vec![(
            t.test_toolchain_type_info.clone(),
            label("//extra:extra_toolchain_linux_impl"),
        )]),
    );

    // Create the prerequisites.
    let toolchain = get_configured_target_and_data(&mut t, "//extra:extra_toolchain_linux_impl");

    // Resolve toolchains.
    let toolchain_context =
        ResolvedToolchainContext::load(&unloaded_toolchain_context, "test", &[toolchain])
            .unwrap_or_else(|e| panic!("{}", e.get_message()));
    assert_has_toolchain_type(&toolchain_context, &t.test_toolchain_type_label);
    assert_toolchain_value(
        &toolchain_context,
        &t.test_toolchain_type_label,
        "data",
        "baz",
    );
}

// Java: ResolvedToolchainContextTest.load_mandatory_missing
#[test]
fn load_mandatory_missing() {
    let mut t = ToolchainTestCase::new();
    let test_toolchain_type = t.test_toolchain_type.clone();
    let toolchain_context_key =
        toolchain_context_key(&mut t, std::slice::from_ref(&test_toolchain_type));

    // Create a static UnloadedToolchainContext.
    let unloaded_toolchain_context = unloaded_toolchain_context(
        &t,
        toolchain_context_key,
        &[test_toolchain_type],
        &[(
            t.test_toolchain_type_label.clone(),
            t.test_toolchain_type_info.clone(),
        )],
        None,
    );

    // Resolve toolchains.
    // Java: assertThrows(ToolchainException.class, ...).
    assert!(ResolvedToolchainContext::load(&unloaded_toolchain_context, "test", &[]).is_err());
}

// Java: ResolvedToolchainContextTest.load_optional_present
#[test]
fn load_optional_present() {
    let mut t = ToolchainTestCase::new();
    t.add_optional_toolchain(
        "extra",
        "extra_toolchain_linux",
        &["//constraints:linux"],
        &["//constraints:linux"],
        "baz",
    );

    let optional_toolchain_type = t.optional_toolchain_type.clone();
    let toolchain_context_key =
        toolchain_context_key(&mut t, std::slice::from_ref(&optional_toolchain_type));

    // Create a static UnloadedToolchainContext.
    let unloaded_toolchain_context = unloaded_toolchain_context(
        &t,
        toolchain_context_key,
        &[optional_toolchain_type],
        &[(
            t.optional_toolchain_type_label.clone(),
            t.optional_toolchain_type_info.clone(),
        )],
        Some(vec![(
            t.optional_toolchain_type_info.clone(),
            label("//extra:extra_toolchain_linux_impl"),
        )]),
    );

    // Create the prerequisites.
    let toolchain = get_configured_target_and_data(&mut t, "//extra:extra_toolchain_linux_impl");

    // Resolve toolchains.
    let toolchain_context =
        ResolvedToolchainContext::load(&unloaded_toolchain_context, "test", &[toolchain])
            .unwrap_or_else(|e| panic!("{}", e.get_message()));
    assert_has_toolchain_type(&toolchain_context, &t.optional_toolchain_type_label);
    assert_toolchain_value(
        &toolchain_context,
        &t.optional_toolchain_type_label,
        "data",
        "baz",
    );
}

// Java: ResolvedToolchainContextTest.load_optional_missing
#[test]
fn load_optional_missing() {
    let mut t = ToolchainTestCase::new();
    let optional_toolchain_type = t.optional_toolchain_type.clone();
    let toolchain_context_key =
        toolchain_context_key(&mut t, std::slice::from_ref(&optional_toolchain_type));

    // Create a static UnloadedToolchainContext.
    let unloaded_toolchain_context = unloaded_toolchain_context(
        &t,
        toolchain_context_key,
        &[optional_toolchain_type],
        &[(
            t.optional_toolchain_type_label.clone(),
            t.optional_toolchain_type_info.clone(),
        )],
        None,
    );

    // Resolve toolchains.
    let toolchain_context =
        ResolvedToolchainContext::load(&unloaded_toolchain_context, "test", &[])
            .unwrap_or_else(|e| panic!("{}", e.get_message()));

    // Missing optional toolchain type requirement is present.
    assert_has_toolchain_type(&toolchain_context, &t.optional_toolchain_type_label);
    // Missing optional toolchain implementation is null.
    assert_toolchain_is_null(&toolchain_context, &t.optional_toolchain_type_label);
}

// Java: ResolvedToolchainContextTest.load_mixed
#[test]
fn load_mixed() {
    let mut t = ToolchainTestCase::new();
    t.add_toolchain(
        "extra",
        "extra_toolchain_linux",
        &["//constraints:linux"],
        &["//constraints:linux"],
        "baz",
    );

    let types = [
        t.test_toolchain_type.clone(),
        t.optional_toolchain_type.clone(),
    ];
    let toolchain_context_key = toolchain_context_key(&mut t, &types);

    // Create a static UnloadedToolchainContext.
    let unloaded_toolchain_context = unloaded_toolchain_context(
        &t,
        toolchain_context_key,
        &types,
        &[
            (
                t.test_toolchain_type_label.clone(),
                t.test_toolchain_type_info.clone(),
            ),
            (
                t.optional_toolchain_type_label.clone(),
                t.optional_toolchain_type_info.clone(),
            ),
        ],
        Some(vec![(
            t.test_toolchain_type_info.clone(),
            label("//extra:extra_toolchain_linux_impl"),
        )]),
    );

    // Create the prerequisites.
    let test_toolchain =
        get_configured_target_and_data(&mut t, "//extra:extra_toolchain_linux_impl");

    // Resolve toolchains.
    let toolchain_context =
        ResolvedToolchainContext::load(&unloaded_toolchain_context, "test", &[test_toolchain])
            .unwrap_or_else(|e| panic!("{}", e.get_message()));

    // Test toolchain is present.
    assert_has_toolchain_type(&toolchain_context, &t.test_toolchain_type_label);
    assert_toolchain_value(
        &toolchain_context,
        &t.test_toolchain_type_label,
        "data",
        "baz",
    );

    // Missing optional toolchain type requirement is present.
    assert_has_toolchain_type(&toolchain_context, &t.optional_toolchain_type_label);
    // Missing optional toolchain implementation is null.
    assert_toolchain_is_null(&toolchain_context, &t.optional_toolchain_type_label);
}

// Java: ResolvedToolchainContextTest.load_aliasedToolchain
#[test]
fn load_aliased_toolchain() {
    let mut t = ToolchainTestCase::new();
    t.scratch.file(
        "alias/BUILD",
        &["alias(name = 'toolchain', actual = '//extra:extra_toolchain_linux_impl')"],
    );
    t.add_toolchain(
        "extra",
        "extra_toolchain_linux",
        &["//constraints:linux"],
        &["//constraints:linux"],
        "baz",
    );

    let test_toolchain_type = t.test_toolchain_type.clone();
    let toolchain_context_key =
        toolchain_context_key(&mut t, std::slice::from_ref(&test_toolchain_type));

    // Create a static UnloadedToolchainContext.
    let unloaded_toolchain_context = unloaded_toolchain_context(
        &t,
        toolchain_context_key,
        &[test_toolchain_type],
        &[(
            t.test_toolchain_type_label.clone(),
            t.test_toolchain_type_info.clone(),
        )],
        Some(vec![(
            t.test_toolchain_type_info.clone(),
            label("//alias:toolchain"),
        )]),
    );

    // Create the prerequisites.
    let toolchain = get_configured_target_and_data(&mut t, "//alias:toolchain");

    // Resolve toolchains.
    let toolchain_context =
        ResolvedToolchainContext::load(&unloaded_toolchain_context, "test", &[toolchain])
            .unwrap_or_else(|e| panic!("{}", e.get_message()));
    assert_has_toolchain_type(&toolchain_context, &t.test_toolchain_type_label);
    assert_toolchain_value(
        &toolchain_context,
        &t.test_toolchain_type_label,
        "data",
        "baz",
    );
}

// Java: ResolvedToolchainContextTest.load_notToolchain
#[test]
fn load_not_toolchain() {
    let mut t = ToolchainTestCase::new();
    t.scratch
        .file("foo/BUILD", &["filegroup(name = 'not_a_toolchain')"]);

    let test_toolchain_type = t.test_toolchain_type.clone();
    let toolchain_context_key =
        toolchain_context_key(&mut t, std::slice::from_ref(&test_toolchain_type));

    // Create a static UnloadedToolchainContext.
    let unloaded_toolchain_context = unloaded_toolchain_context(
        &t,
        toolchain_context_key,
        &[test_toolchain_type],
        &[(
            t.test_toolchain_type_label.clone(),
            t.test_toolchain_type_info.clone(),
        )],
        Some(vec![(
            t.test_toolchain_type_info.clone(),
            label("//foo:not_a_toolchain"),
        )]),
    );

    // Create the prerequisites, which is not actually a valid toolchain.
    let toolchain = get_configured_target_and_data(&mut t, "//foo:not_a_toolchain");
    // Java: assertThrows(ToolchainException.class, ...).
    assert!(
        ResolvedToolchainContext::load(&unloaded_toolchain_context, "test", &[toolchain]).is_err()
    );
}

// Java: ResolvedToolchainContextTest.load_withTemplateVariables
#[test]
fn load_with_template_variables() {
    let mut t = ToolchainTestCase::new();
    // Add new toolchain rule that provides template variables.
    let variable_toolchain_type_label = label("//variable:variable_toolchain_type");
    let variable_toolchain_type =
        ToolchainTypeRequirement::create(variable_toolchain_type_label.clone());
    let variable_toolchain_type_info =
        ToolchainTypeInfo::create(variable_toolchain_type_label.clone());
    t.scratch.file(
        "variable/variable_toolchain_def.bzl",
        &[r#"def _impl(ctx):
    value = ctx.attr.value
    toolchain = platform_common.ToolchainInfo()
    template_variables = platform_common.TemplateVariableInfo({"VALUE": value})
    return [toolchain, template_variables]

variable_toolchain = rule(
    implementation = _impl,
    attrs = {"value": attr.string()},
)
"#],
    );

    // Create instance of new toolchain and register it.
    t.scratch.file(
        "variable/BUILD",
        &[
            r#"load("//variable:variable_toolchain_def.bzl", "variable_toolchain")

toolchain_type(name = "variable_toolchain_type")

variable_toolchain(
    name = "variable_toolchain_impl",
    value = "foo",
)
"#,
        ],
    );

    let test_toolchain_type = t.test_toolchain_type.clone();
    let toolchain_context_key = toolchain_context_key(&mut t, &[test_toolchain_type]);

    // Create a static UnloadedToolchainContext.
    let unloaded_toolchain_context = unloaded_toolchain_context(
        &t,
        toolchain_context_key,
        &[variable_toolchain_type],
        &[(
            variable_toolchain_type_label.clone(),
            variable_toolchain_type_info.clone(),
        )],
        Some(vec![(
            variable_toolchain_type_info,
            label("//variable:variable_toolchain_impl"),
        )]),
    );

    // Create the prerequisites.
    let toolchain = get_configured_target_and_data(&mut t, "//variable:variable_toolchain_impl");
    let toolchain_context =
        ResolvedToolchainContext::load(&unloaded_toolchain_context, "test", &[toolchain])
            .unwrap_or_else(|e| panic!("{}", e.get_message()));
    assert_has_toolchain_type(&toolchain_context, &variable_toolchain_type_label);
    assert_eq!(toolchain_context.template_variable_providers().len(), 1);
    let variables =
        downcast_info::<TemplateVariableInfo>(&toolchain_context.template_variable_providers()[0])
            .expect("ClassCastException: TemplateVariableInfo")
            .get_variables()
            .clone();
    let expected: IndexMap<String, String> = [("VALUE".to_string(), "foo".to_string())]
        .into_iter()
        .collect();
    assert_eq!(variables, expected);
}
