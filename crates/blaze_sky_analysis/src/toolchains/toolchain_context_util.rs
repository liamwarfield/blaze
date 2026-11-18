//! Port of `skyframe/toolchains/ToolchainContextUtil.java`: utility methods for toolchain
//! context creation.

use blaze_analysis::toolchains::{
    InvalidExecGroupException, ToolchainContextKey, ToolchainTypeRequirement,
};
use blaze_build_config::build_configuration_key::BuildConfigurationKey;
use blaze_build_config::fragments::platform_configuration::PlatformConfiguration;
use blaze_cmdline::{Label, LabelPackageContext};
use blaze_optdefs::fragment::core_options::CoreOptions;
use blaze_packages::attribute::{AttributeMap, NonconfigurableAttributeMapper, RawAttributeMapper};
use blaze_packages::rule_class::declared_exec_group::is_valid_identifier;
use blaze_packages::rule_class::rule_class::{
    EXEC_COMPATIBLE_WITH_ATTR, EXEC_GROUP_COMPATIBLE_WITH_ATTR,
};
use blaze_packages::rule_class::{DEFAULT_EXEC_GROUP_NAME, DeclaredExecGroup};
use blaze_packages::target::{Rule, Target};
use blaze_packages::types::Type;
use indexmap::{IndexMap, IndexSet};

use blaze_analysis::toolchains::UnloadedToolchainContextsInputs;

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainContextUtil.java:42 ToolchainContextUtil.getUnloadedToolchainContextsInputs
pub fn get_unloaded_toolchain_contexts_inputs(
    target: &Target,
    core_options: &CoreOptions,
    platform_config: Option<&PlatformConfiguration>,
    parent_execution_platform_label: Option<&Label>,
    toolchain_configuration_key: BuildConfigurationKey,
) -> Result<UnloadedToolchainContextsInputs, InvalidExecGroupException> {
    let Some(rule) = target.get_associated_rule() else {
        return Ok(UnloadedToolchainContextsInputs::empty());
    };

    let rule_class = rule.get_rule_class_object();
    let use_auto_exec_groups = rule_class.get_auto_exec_groups_mode().is_enabled(
        &RawAttributeMapper::of(&**rule),
        core_options.use_auto_exec_groups,
    );

    let mut toolchain_types: Vec<ToolchainTypeRequirement> =
        rule_class.get_toolchain_types().iter().cloned().collect();
    if !rule_class.is_starlark() && rule_class.get_name() == "genrule" {
        // Override the toolchain types based on the target-level "toolchains" attribute.
        toolchain_types = update_toolchain_types_from_attribute(rule, toolchain_types);
    }

    let default_exec_constraint_labels = get_execution_platform_constraints(rule, platform_config);
    let per_exec_group_exec_constraint_labels = get_per_exec_group_execution_platform_constraints(
        rule,
        platform_config,
        &toolchain_types,
        use_auto_exec_groups,
    )?;

    let processed_exec_groups = process_declared_exec_groups(
        rule_class.get_declared_exec_groups(),
        &default_exec_constraint_labels,
        &per_exec_group_exec_constraint_labels,
        &toolchain_types,
        use_auto_exec_groups,
    );

    let Some(platform_config) = platform_config.filter(|_| rule.use_toolchain_resolution()) else {
        return Ok(UnloadedToolchainContextsInputs::create(
            processed_exec_groups,
            None,
        ));
    };

    Ok(UnloadedToolchainContextsInputs::create(
        processed_exec_groups,
        Some(create_default_toolchain_context_key(
            toolchain_configuration_key,
            default_exec_constraint_labels,
            platform_config.debug_toolchain_resolution_for_label(rule.get_label()),
            use_auto_exec_groups,
            toolchain_types,
            parent_execution_platform_label,
        )),
    ))
}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainContextUtil.java:95 ToolchainContextUtil.updateToolchainTypesFromAttribute
fn update_toolchain_types_from_attribute(
    rule: &Rule,
    toolchain_types: Vec<ToolchainTypeRequirement>,
) -> Vec<ToolchainTypeRequirement> {
    let attributes = NonconfigurableAttributeMapper::of(rule);
    let target_toolchain_types: Vec<Label> = attributes
        .get("toolchains", Type::LabelList)
        .and_then(|v| v.as_label_list().map(<[Label]>::to_vec))
        .unwrap_or_default();
    if target_toolchain_types.is_empty() {
        // No need to update.
        return toolchain_types;
    }

    let mut updated_toolchain_types: Vec<ToolchainTypeRequirement> = Vec::new();
    for t in toolchain_types {
        if !updated_toolchain_types.contains(&t) {
            updated_toolchain_types.push(t);
        }
    }
    for toolchain_type_label in target_toolchain_types {
        let t = ToolchainTypeRequirement {
            toolchain_type: toolchain_type_label,
            // Legacy, so all toolchain types from the attribute are optional and can be
            // ignored if invalid.
            mandatory: false,
            ignore_if_invalid: true,
        };
        if !updated_toolchain_types.contains(&t) {
            updated_toolchain_types.push(t);
        }
    }
    updated_toolchain_types
}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainContextUtil.java:120 ToolchainContextUtil.createDefaultToolchainContextKey
pub fn create_default_toolchain_context_key(
    configuration_key: BuildConfigurationKey,
    default_exec_constraint_labels: Vec<Label>,
    debug_target: bool,
    use_auto_exec_groups: bool,
    toolchain_types: Vec<ToolchainTypeRequirement>,
    parent_execution_platform_label: Option<&Label>,
) -> ToolchainContextKey {
    let mut toolchain_context_key_builder = ToolchainContextKey::key();
    toolchain_context_key_builder
        .configuration_key(configuration_key)
        .exec_constraint_labels(default_exec_constraint_labels)
        .debug_target(debug_target);

    // Add toolchain types only if automatic exec groups are not created for this target.
    if !use_auto_exec_groups {
        toolchain_context_key_builder.toolchain_types(toolchain_types);
    }

    if let Some(parent_execution_platform_label) = parent_execution_platform_label {
        // Find out what execution platform the parent used, and force that.
        // This should only be set for direct toolchain dependencies.
        toolchain_context_key_builder
            .force_execution_platform(parent_execution_platform_label.clone());
    }
    toolchain_context_key_builder.build()
}

/// Returns the target-specific execution platform constraints, based on the rule definition and
/// any constraints added by the target, including those added for the target on the command
/// line.
// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainContextUtil.java:150 ToolchainContextUtil.getExecutionPlatformConstraints
fn get_execution_platform_constraints(
    rule: &Rule,
    platform_configuration: Option<&PlatformConfiguration>,
) -> Vec<Label> {
    if platform_configuration.is_none() {
        return Vec::new(); // See NoConfigTransition.
    }
    let mapper = NonconfigurableAttributeMapper::of(rule);
    let mut exec_constraint_labels: IndexSet<Label> = IndexSet::new();

    exec_constraint_labels.extend(
        rule.get_rule_class_object()
            .get_execution_platform_constraints()
            .iter()
            .cloned(),
    );
    if rule
        .get_rule_class_object()
        .get_attribute_provider()
        .has_attr(EXEC_COMPATIBLE_WITH_ATTR, Type::LabelList)
    {
        if let Some(labels) = mapper
            .get(EXEC_COMPATIBLE_WITH_ATTR, Type::LabelList)
            .as_ref()
            .and_then(|v| v.as_label_list())
        {
            exec_constraint_labels.extend(labels.iter().cloned());
        }
    }

    exec_constraint_labels.into_iter().collect()
}

/// Java `StarlarkExecGroupCollection.isValidGroupName`.
// PORT: duplicate of agent S's port in analysis/starlark (kept private to avoid coupling).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkExecGroupCollection.java:78 StarlarkExecGroupCollection.isValidGroupName
fn is_valid_group_name(exec_group_name: &str) -> bool {
    exec_group_name != DEFAULT_EXEC_GROUP_NAME && is_valid_identifier(exec_group_name)
}

// Java: com/google/devtools/build/lib/skyframe/toolchains/ToolchainContextUtil.java:169 ToolchainContextUtil.getPerExecGroupExecutionPlatformConstraints
fn get_per_exec_group_execution_platform_constraints(
    rule: &Rule,
    platform_configuration: Option<&PlatformConfiguration>,
    toolchain_types: &[ToolchainTypeRequirement],
    use_auto_exec_groups: bool,
) -> Result<IndexMap<String, Vec<Label>>, InvalidExecGroupException> {
    if platform_configuration.is_none() {
        return Ok(IndexMap::new()); // See NoConfigTransition.
    }
    if !rule
        .get_rule_class_object()
        .get_attribute_provider()
        .has_attr(EXEC_GROUP_COMPATIBLE_WITH_ATTR, Type::LabelListDict)
    {
        return Ok(IndexMap::new());
    }
    let mapper = NonconfigurableAttributeMapper::of(rule);
    let mut exec_group_constraints: IndexMap<String, Vec<Label>> = IndexMap::new();

    let metadata = rule.get_package_metadata();
    let package_context = LabelPackageContext::of(
        &metadata.package_identifier,
        metadata.repository_mapping.clone(),
    );
    let display_form_label = display_form_label(rule);
    let declared_exec_groups = rule.get_rule_class_object().get_declared_exec_groups();
    let value = mapper.get(EXEC_GROUP_COMPATIBLE_WITH_ATTR, Type::LabelListDict);
    let entries: IndexMap<String, Vec<Label>> = match value {
        Some(blaze_packages::types::AttrValue::LabelListDict(m)) => m,
        _ => IndexMap::new(),
    };
    for (entry_key, entry_value) in entries {
        let canonical_key: String;
        if is_valid_group_name(&entry_key) {
            canonical_key = entry_key.clone();
            if !declared_exec_groups.contains_key(&canonical_key) {
                let valid: Vec<String> = declared_exec_groups.keys().cloned().collect();
                return Err(InvalidExecGroupException::new(
                    "execution constraints",
                    &display_form_label,
                    std::slice::from_ref(&canonical_key),
                    valid.as_slice(),
                ));
            }
        } else if use_auto_exec_groups {
            let label = match Label::parse_with_package_context(&entry_key, &package_context) {
                Ok(l) => l,
                Err(_) => {
                    let empty: Vec<String> = Vec::new();
                    return Err(InvalidExecGroupException::new(
                        "execution constraints",
                        &display_form_label,
                        std::slice::from_ref(&entry_key),
                        empty.as_slice(),
                    ));
                }
            };
            if !toolchain_types.iter().any(|t| t.toolchain_type == label) {
                let mut suggested_labels: Vec<String> = Vec::new();
                // Only suggest labels for the main repository.
                if rule.get_label().get_repository().is_main() {
                    for t in toolchain_types {
                        let s = t
                            .toolchain_type
                            .get_display_form(Some(&metadata.repository_mapping));
                        if !suggested_labels.contains(&s) {
                            suggested_labels.push(s);
                        }
                    }
                }
                return Err(InvalidExecGroupException::new(
                    "execution constraints",
                    &display_form_label,
                    std::slice::from_ref(&entry_key),
                    suggested_labels.as_slice(),
                ));
            }
            canonical_key = label.get_unambiguous_canonical_form();
        } else {
            let empty: Vec<String> = Vec::new();
            return Err(InvalidExecGroupException::new(
                "execution constraints",
                &display_form_label,
                std::slice::from_ref(&entry_key),
                empty.as_slice(),
            ));
        }
        exec_group_constraints
            .entry(canonical_key)
            .or_default()
            .extend(entry_value);
    }

    Ok(exec_group_constraints)
}

/// Java `Rule.getDisplayFormLabel()`.
// Java: com/google/devtools/build/lib/packages/Target.java:243 Target.getDisplayFormLabel
fn display_form_label(rule: &Rule) -> String {
    let label = rule.get_label();
    let mapping = if label.get_repository().is_main() {
        Some(&rule.get_package_metadata().repository_mapping)
    } else {
        None
    };
    label.get_display_form(mapping)
}

/// Prepares the input exec groups: adds auto exec groups when `use_auto_exec_groups` is true.
// PORT: move to blaze_packages (DeclaredExecGroup.process).
// Java: com/google/devtools/build/lib/packages/DeclaredExecGroup.java:94 DeclaredExecGroup.process
pub fn process_declared_exec_groups(
    exec_groups: &IndexMap<String, DeclaredExecGroup>,
    default_exec_with: &[Label],
    exec_group_exec_with: &IndexMap<String, Vec<Label>>,
    default_toolchain_types: &[ToolchainTypeRequirement],
    use_auto_exec_groups: bool,
) -> IndexMap<String, DeclaredExecGroup> {
    let mut processed_groups: IndexMap<String, DeclaredExecGroup> = IndexMap::new();
    let put =
        |groups: &mut IndexMap<String, DeclaredExecGroup>, name: String, g: DeclaredExecGroup| {
            if groups.contains_key(&name) {
                // Java: ImmutableMap.Builder.buildOrThrow on a duplicate key.
                panic!("IllegalArgumentException: Multiple entries with same key: {name}");
            }
            groups.insert(name, g);
        };
    for (name, declared_exec_group) in exec_groups {
        let mut declared_exec_group = declared_exec_group.clone();

        if declared_exec_group.copy_from_default {
            declared_exec_group =
                declared_exec_group_of(default_exec_with, default_toolchain_types);
        }
        if let Some(extra_exec_with) = exec_group_exec_with.get(name).filter(|v| !v.is_empty()) {
            let mut exec_compatible_with = declared_exec_group.exec_compatible_with.clone();
            exec_compatible_with.extend(extra_exec_with.iter().cloned());
            declared_exec_group.exec_compatible_with = exec_compatible_with;
        }

        put(&mut processed_groups, name.clone(), declared_exec_group);
    }

    if use_auto_exec_groups {
        // Creates one exec group for each toolchain (automatic exec groups).
        for toolchain_type in default_toolchain_types {
            let mut exec_compatible_with: IndexSet<Label> =
                default_exec_with.iter().cloned().collect();
            if let Some(extra_exec_with) = exec_group_exec_with
                .get(
                    &toolchain_type
                        .toolchain_type
                        .get_unambiguous_canonical_form(),
                )
                .filter(|v| !v.is_empty())
            {
                exec_compatible_with.extend(extra_exec_with.iter().cloned());
            }
            let mut toolchain_types_map = IndexMap::new();
            toolchain_types_map.insert(
                toolchain_type.toolchain_type.clone(),
                toolchain_type.clone(),
            );
            put(
                &mut processed_groups,
                toolchain_type.toolchain_type.to_string(),
                DeclaredExecGroup {
                    toolchain_types_map,
                    exec_compatible_with,
                    copy_from_default: false,
                },
            );
        }
    }
    processed_groups
}

/// Java `DeclaredExecGroup.builder().execCompatibleWith(...).toolchainTypes(...).build()`.
fn declared_exec_group_of(
    exec_compatible_with: &[Label],
    toolchain_types: &[ToolchainTypeRequirement],
) -> DeclaredExecGroup {
    let mut toolchain_types_map = IndexMap::new();
    for t in toolchain_types {
        if toolchain_types_map.contains_key(&t.toolchain_type) {
            // Java: ImmutableMap.Builder.buildOrThrow on a duplicate key.
            panic!(
                "IllegalArgumentException: Multiple entries with same key: {}",
                t.toolchain_type
            );
        }
        toolchain_types_map.insert(t.toolchain_type.clone(), t.clone());
    }
    DeclaredExecGroup {
        toolchain_types_map,
        exec_compatible_with: exec_compatible_with.iter().cloned().collect(),
        copy_from_default: false,
    }
}
