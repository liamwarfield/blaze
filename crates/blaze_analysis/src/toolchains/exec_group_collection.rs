//! Port of `analysis/ExecGroupCollection.java`.

use blaze_packages::attribute::string_util::did_you_mean;
use blaze_packages::rule_class::{DEFAULT_EXEC_GROUP_NAME, DeclaredExecGroup};
use indexmap::{IndexMap, IndexSet};

use crate::platform::PlatformInfo;
use crate::toolchains::resolved_toolchain_context::ResolvedToolchainContext;
use crate::toolchains::toolchain_collection::ToolchainCollection;
use crate::toolchains::toolchain_context::ToolchainContext;

/// Java `Table<String, String, String>` (row, column, value) with Guava's insertion-ordered
/// `HashBasedTable`/`ImmutableTable` iteration.
pub type ExecPropertiesTable = IndexMap<String, IndexMap<String, String>>;

/// A container class for groups of [`DeclaredExecGroup`] instances. This correctly handles exec
/// group inheritance between rules and targets. See https://bazel.build/reference/exec-groups
/// for further details.
// Java: com/google/devtools/build/lib/analysis/ExecGroupCollection.java:45 ExecGroupCollection
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ExecGroupCollection {
    exec_groups: IndexMap<String, DeclaredExecGroup>,
    exec_properties: ExecPropertiesTable,
}

/// Builder class for correctly constructing [`ExecGroupCollection`] instances (Java's abstract
/// `ExecGroupCollection.Builder`, whose only state is `execGroups()`).
// Java: com/google/devtools/build/lib/analysis/ExecGroupCollection.java:49 ExecGroupCollection.Builder
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct ExecGroupCollectionBuilder {
    pub exec_groups: IndexMap<String, DeclaredExecGroup>,
}

impl ExecGroupCollectionBuilder {
    pub fn exec_groups(&self) -> &IndexMap<String, DeclaredExecGroup> {
        &self.exec_groups
    }

    // Java: com/google/devtools/build/lib/analysis/ExecGroupCollection.java:52 ExecGroupCollection.Builder.build
    pub fn build(
        &self,
        toolchain_contexts: Option<&ToolchainCollection<ResolvedToolchainContext>>,
        raw_exec_properties: &IndexMap<String, String>,
        target_label: &str,
    ) -> Result<ExecGroupCollection, InvalidExecGroupException> {
        // For each exec group, compute the combined execution properties.
        let combined_exec_properties = ExecGroupCollection::compute_combined_exec_properties(
            toolchain_contexts,
            raw_exec_properties,
            target_label,
        )?;
        Ok(ExecGroupCollection {
            exec_groups: self.exec_groups.clone(),
            exec_properties: combined_exec_properties,
        })
    }
}

impl ExecGroupCollection {
    /// Gets the combined exec properties of the platform and the target's exec properties. If a
    /// property is set in both, the target properties take precedence.
    // Java: com/google/devtools/build/lib/analysis/ExecGroupCollection.java:70 ExecGroupCollection.computeCombinedExecProperties
    fn compute_combined_exec_properties(
        toolchain_contexts: Option<&ToolchainCollection<ResolvedToolchainContext>>,
        raw_exec_properties: &IndexMap<String, String>,
        target_label: &str,
    ) -> Result<ExecPropertiesTable, InvalidExecGroupException> {
        let exec_group_names: Vec<String> = match toolchain_contexts {
            None => vec![DEFAULT_EXEC_GROUP_NAME.to_owned()],
            Some(c) => c
                .get_exec_group_names()
                .into_iter()
                .map(str::to_owned)
                .collect(),
        };

        // Parse the target-level exec properties.
        let parsed_target_properties = Self::parse_exec_properties(raw_exec_properties)?;
        // Validate the exec group names in the properties.
        if toolchain_contexts.is_some() {
            let unknown_target_exec_group_names: IndexSet<String> = parsed_target_properties
                .keys()
                .filter(|name| name.as_str() != DEFAULT_EXEC_GROUP_NAME)
                .filter(|name| !exec_group_names.contains(name))
                .cloned()
                .collect();
            if !unknown_target_exec_group_names.is_empty() {
                let mut valid_names = exec_group_names.clone();
                valid_names.push(DEFAULT_EXEC_GROUP_NAME.to_owned());
                let invalid_names: Vec<String> =
                    unknown_target_exec_group_names.into_iter().collect();
                return Err(InvalidExecGroupException::new(
                    "exec_properties",
                    target_label,
                    invalid_names.as_slice(),
                    valid_names.as_slice(),
                ));
            }
        }

        // Parse each execution platform's exec properties.
        let mut execution_platforms: Vec<&PlatformInfo> = Vec::new();
        if let Some(contexts) = toolchain_contexts {
            for name in &exec_group_names {
                let p: &PlatformInfo = contexts
                    .get_toolchain_context(name)
                    .expect("exec group context")
                    .execution_platform();
                if !execution_platforms.contains(&p) {
                    execution_platforms.push(p);
                }
            }
        }
        let mut parsed_platform_properties: Vec<(&PlatformInfo, ExecPropertiesTable)> = Vec::new();
        for execution_platform in execution_platforms {
            // Platforms may have exec properties per exec group.
            let parsed = Self::parse_exec_properties(execution_platform.exec_properties())?;
            parsed_platform_properties.push((execution_platform, parsed));
        }

        // First, get the defaults.
        let default_exec_properties: IndexMap<String, String> = parsed_target_properties
            .get(DEFAULT_EXEC_GROUP_NAME)
            .cloned()
            .unwrap_or_default();
        let mut result: ExecPropertiesTable = IndexMap::new();
        put_all(
            &mut result,
            DEFAULT_EXEC_GROUP_NAME,
            &default_exec_properties,
        );

        for exec_group_name in &exec_group_names {
            let combined = Self::compute_properties(
                exec_group_name,
                &default_exec_properties,
                toolchain_contexts,
                &parsed_platform_properties,
                &parsed_target_properties,
            );
            put_all(&mut result, exec_group_name, &combined);
        }

        Ok(result)
    }

    // Java: com/google/devtools/build/lib/analysis/ExecGroupCollection.java:147 ExecGroupCollection.computeProperties
    fn compute_properties(
        exec_group_name: &str,
        default_exec_properties: &IndexMap<String, String>,
        toolchain_contexts: Option<&ToolchainCollection<ResolvedToolchainContext>>,
        parsed_platform_properties: &[(&PlatformInfo, ExecPropertiesTable)],
        parsed_target_properties: &ExecPropertiesTable,
    ) -> IndexMap<String, String> {
        let empty = IndexMap::new();
        let (default_exec_group_platform_properties, platform_properties) = match toolchain_contexts
        {
            None => (&empty, &empty),
            Some(contexts) => {
                let execution_platform: &PlatformInfo = contexts
                    .get_toolchain_context(exec_group_name)
                    .expect("exec group context")
                    .execution_platform();
                let parsed = &parsed_platform_properties
                    .iter()
                    .find(|(p, _)| *p == execution_platform)
                    .expect("parsed platform properties")
                    .1;
                (
                    parsed.get(DEFAULT_EXEC_GROUP_NAME).unwrap_or(&empty),
                    parsed.get(exec_group_name).unwrap_or(&empty),
                )
            }
        };
        let mut target_properties: IndexMap<String, String> = parsed_target_properties
            .get(exec_group_name)
            .cloned()
            .unwrap_or_default();
        for (property_name, value) in default_exec_properties {
            // If the property exists in the default and not in the target, copy it.
            target_properties
                .entry(property_name.clone())
                .or_insert_with(|| value.clone());
        }

        // Combine the target and exec platform properties. Target properties take precedence.
        // Use a Map instead of an ImmutableMap.Builder because we expect duplicate keys.
        let mut combined: IndexMap<String, String> = IndexMap::new();
        for m in [
            default_exec_group_platform_properties,
            default_exec_properties,
            platform_properties,
            &target_properties,
        ] {
            for (k, v) in m {
                combined.insert(k.clone(), v.clone());
            }
        }
        combined
    }

    // Java: com/google/devtools/build/lib/analysis/ExecGroupCollection.java:187 ExecGroupCollection.getExecGroup
    pub fn get_exec_group(&self, exec_group_name: &str) -> Option<&DeclaredExecGroup> {
        self.exec_groups.get(exec_group_name)
    }

    pub fn exec_groups(&self) -> &IndexMap<String, DeclaredExecGroup> {
        &self.exec_groups
    }

    // Java: com/google/devtools/build/lib/analysis/ExecGroupCollection.java:191 ExecGroupCollection.getExecProperties
    pub fn get_exec_properties(&self, exec_group_name: &str) -> IndexMap<String, String> {
        self.exec_properties
            .get(exec_group_name)
            .cloned()
            .unwrap_or_default()
    }

    /// Parse raw exec properties attribute value into a map of exec group names to their
    /// properties. The raw map can have keys of two forms: (1) 'property' and (2)
    /// 'exec_group_name.property'. The former get parsed into the default exec group, the latter
    /// get parsed into their relevant exec groups.
    // Java: com/google/devtools/build/lib/analysis/ExecGroupCollection.java:201 ExecGroupCollection.parseExecProperties
    fn parse_exec_properties(
        raw_exec_properties: &IndexMap<String, String>,
    ) -> Result<ExecPropertiesTable, InvalidExecGroupException> {
        let mut exec_properties: ExecPropertiesTable = IndexMap::new();
        for (raw_property, value) in raw_exec_properties {
            let (exec_group, property) = match raw_property.find('.') {
                None => (DEFAULT_EXEC_GROUP_NAME, raw_property.as_str()),
                Some(i) => (&raw_property[..i], &raw_property[i + 1..]),
            };
            let row = exec_properties.entry(exec_group.to_owned()).or_default();
            if row.contains_key(property) {
                // Java: ImmutableTable.Builder.buildOrThrow on a duplicate cell.
                panic!(
                    "IllegalArgumentException: Duplicate key: ({exec_group}, {property}), values: [{}, {}]",
                    row[property], value
                );
            }
            row.insert(property.to_owned(), value.clone());
        }
        Ok(exec_properties)
    }
}

// Java: com/google/devtools/build/lib/analysis/ExecGroupCollection.java:141 ExecGroupCollection.putAll
fn put_all(builder: &mut ExecPropertiesTable, row: &str, values: &IndexMap<String, String>) {
    for (k, v) in values {
        builder
            .entry(row.to_owned())
            .or_default()
            .insert(k.clone(), v.clone());
    }
}

/// An error for when the user tries to access a non-existent exec group.
// Java: com/google/devtools/build/lib/analysis/ExecGroupCollection.java:219 ExecGroupCollection.InvalidExecGroupException
#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct InvalidExecGroupException {
    message: String,
}

impl InvalidExecGroupException {
    // Java: com/google/devtools/build/lib/analysis/ExecGroupCollection.java:221 ExecGroupCollection.InvalidExecGroupException.<init>
    pub fn new<S: AsRef<str>>(
        what: &str,
        target_label: &str,
        invalid_names: &[S],
        valid_names: &[S],
    ) -> Self {
        let suggestion = invalid_names
            .iter()
            .map(|invalid_name| did_you_mean(invalid_name.as_ref(), valid_names))
            .find(|s| !s.is_empty())
            .unwrap_or_default();
        InvalidExecGroupException {
            message: format!(
                "Tried to set {what} for non-existent exec groups on {target_label}: {}{}",
                invalid_names
                    .iter()
                    .map(|s| s.as_ref())
                    .collect::<Vec<_>>()
                    .join(","),
                suggestion
            ),
        }
    }

    pub fn get_message(&self) -> &str {
        &self.message
    }

    /// Java `getDetailedExitCode()`: `Analysis.Code.EXEC_GROUP_MISSING`.
    // Java: com/google/devtools/build/lib/analysis/ExecGroupCollection.java:240 ExecGroupCollection.InvalidExecGroupException.getDetailedExitCode
    pub fn get_detailed_code(&self) -> &'static str {
        "EXEC_GROUP_MISSING"
    }
}
