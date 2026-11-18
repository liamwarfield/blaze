//! Port of `packages/ConfigurationFragmentPolicy.java`. Fragment classes are named by their
//! Java class simple name.

use indexmap::IndexMap;
use indexmap::IndexSet;

/// Java `ConfigurationFragmentPolicy.MissingFragmentPolicy`.
// Java: com/google/devtools/build/lib/packages/ConfigurationFragmentPolicy.java:41 ConfigurationFragmentPolicy.MissingFragmentPolicy
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum MissingFragmentPolicy {
    Ignore,
    CreateFailActions,
    FailAnalysis,
}

/// Which configuration fragments a rule may access (Java `ConfigurationFragmentPolicy`).
// Java: com/google/devtools/build/lib/packages/ConfigurationFragmentPolicy.java:35 ConfigurationFragmentPolicy
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ConfigurationFragmentPolicy {
    required_configuration_fragments: IndexSet<String>,
    starlark_required_configuration_fragments: IndexSet<String>,
    missing_fragment_policy: IndexMap<String, MissingFragmentPolicy>,
}

impl ConfigurationFragmentPolicy {
    // Java: com/google/devtools/build/lib/packages/ConfigurationFragmentPolicy.java:164 ConfigurationFragmentPolicy.getRequiredConfigurationFragments
    pub fn get_required_configuration_fragments(&self) -> &IndexSet<String> {
        &self.required_configuration_fragments
    }

    // Java: com/google/devtools/build/lib/packages/ConfigurationFragmentPolicy.java:176 ConfigurationFragmentPolicy.getRequiredStarlarkFragments
    pub fn get_required_starlark_fragments(&self) -> &IndexSet<String> {
        &self.starlark_required_configuration_fragments
    }

    // Java: com/google/devtools/build/lib/packages/ConfigurationFragmentPolicy.java:184 ConfigurationFragmentPolicy.isLegalConfigurationFragment
    /// `configuration_fragment` is the fragment's Java class simple name;
    /// `starlark_builtin_name` its `@StarlarkBuiltin` name, if any (Java reads the annotation).
    pub fn is_legal_configuration_fragment(
        &self,
        configuration_fragment: &str,
        starlark_builtin_name: Option<&str>,
    ) -> bool {
        self.required_configuration_fragments
            .contains(configuration_fragment)
            // Java: com/google/devtools/build/lib/packages/ConfigurationFragmentPolicy.java:193 ConfigurationFragmentPolicy.hasLegalFragmentName
            || starlark_builtin_name
                .is_some_and(|n| self.starlark_required_configuration_fragments.contains(n))
    }

    // Java: com/google/devtools/build/lib/packages/ConfigurationFragmentPolicy.java:205 ConfigurationFragmentPolicy.getMissingFragmentPolicy
    pub fn get_missing_fragment_policy(&self, fragment_class: &str) -> MissingFragmentPolicy {
        self.missing_fragment_policy
            .get(fragment_class)
            .copied()
            .unwrap_or(MissingFragmentPolicy::FailAnalysis)
    }
}

/// Java `ConfigurationFragmentPolicy.Builder`.
// Java: com/google/devtools/build/lib/packages/ConfigurationFragmentPolicy.java:67 ConfigurationFragmentPolicy.Builder
#[derive(Debug, Clone, Default)]
pub struct ConfigurationFragmentPolicyBuilder {
    required_configuration_fragments: IndexSet<String>,
    starlark_required_configuration_fragments: IndexSet<String>,
    missing_fragment_policy: IndexMap<String, MissingFragmentPolicy>,
}

impl ConfigurationFragmentPolicyBuilder {
    pub fn new() -> Self {
        ConfigurationFragmentPolicyBuilder::default()
    }

    // Java: com/google/devtools/build/lib/packages/ConfigurationFragmentPolicy.java:89 ConfigurationFragmentPolicy.Builder.requiresConfigurationFragments
    pub fn requires_configuration_fragments(
        &mut self,
        configuration_fragments: &[&str],
    ) -> &mut Self {
        for f in configuration_fragments {
            self.required_configuration_fragments
                .insert((*f).to_owned());
        }
        self
    }

    // Java: com/google/devtools/build/lib/packages/ConfigurationFragmentPolicy.java:105 ConfigurationFragmentPolicy.Builder.requiresConfigurationFragmentsByStarlarkBuiltinName
    pub fn requires_configuration_fragments_by_starlark_builtin_name(
        &mut self,
        configuration_fragment_names: &[String],
    ) -> &mut Self {
        for f in configuration_fragment_names {
            self.starlark_required_configuration_fragments
                .insert(f.clone());
        }
        self
    }

    // Java: com/google/devtools/build/lib/packages/ConfigurationFragmentPolicy.java:117 ConfigurationFragmentPolicy.Builder.includeConfigurationFragmentsFrom
    pub fn include_configuration_fragments_from(
        &mut self,
        other: &ConfigurationFragmentPolicy,
    ) -> &mut Self {
        self.required_configuration_fragments
            .extend(other.required_configuration_fragments.iter().cloned());
        self.starlark_required_configuration_fragments.extend(
            other
                .starlark_required_configuration_fragments
                .iter()
                .cloned(),
        );
        self.missing_fragment_policy.extend(
            other
                .missing_fragment_policy
                .iter()
                .map(|(k, v)| (k.clone(), *v)),
        );
        self
    }

    // Java: com/google/devtools/build/lib/packages/ConfigurationFragmentPolicy.java:129 ConfigurationFragmentPolicy.Builder.setMissingFragmentPolicy
    pub fn set_missing_fragment_policy(
        &mut self,
        fragment_class: &str,
        missing_fragment_policy: MissingFragmentPolicy,
    ) -> &mut Self {
        self.missing_fragment_policy
            .insert(fragment_class.to_owned(), missing_fragment_policy);
        self
    }

    // Java: com/google/devtools/build/lib/packages/ConfigurationFragmentPolicy.java:135 ConfigurationFragmentPolicy.Builder.build
    pub fn build(&self) -> ConfigurationFragmentPolicy {
        ConfigurationFragmentPolicy {
            required_configuration_fragments: self.required_configuration_fragments.clone(),
            starlark_required_configuration_fragments: self
                .starlark_required_configuration_fragments
                .clone(),
            missing_fragment_policy: self.missing_fragment_policy.clone(),
        }
    }
}
