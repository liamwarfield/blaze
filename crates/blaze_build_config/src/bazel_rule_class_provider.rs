//! The configuration parts of Bazel's rule class provider (`BazelRuleClassProvider.setup` and
//! `ConfiguredRuleClassProvider`): the fragment registry, the action environment, the runfiles
//! prefix and reserved mnemonics.
//!
//! PORT: move to the crate that ports `ConfiguredRuleClassProvider` (blaze_rules) when it exists.

use crate::action_environment::{ActionEnvironment, shell_action_env};
use crate::build_configuration_value::GlobalStateProvider;
use crate::build_options::BuildOptions;
use crate::fragment::FragmentRegistry;
use crate::fragment_options_class::{CORE_OPTIONS, FragmentOptionsClass};

/// `ConfiguredRuleClassProvider` as `BuildConfigurationValue.GlobalStateProvider`.
#[derive(Debug)]
pub struct BazelRuleClassProvider {
    fragment_registry: FragmentRegistry,
    reserved_action_mnemonics: Vec<String>,
    runfiles_prefix: String,
}

impl BazelRuleClassProvider {
    // Java: com/google/devtools/build/lib/bazel/rules/BazelRuleClassProvider.java:208 BazelRuleClassProvider.setup
    pub fn create() -> Self {
        let (all, universal) = crate::fragments::bazel_fragments();
        BazelRuleClassProvider {
            // BAZEL_SETUP: builder.addConfigurationOptions(CoreOptions.class)
            fragment_registry: FragmentRegistry::create(all, universal, vec![&CORE_OPTIONS]),
            // No rule set of Bazel calls addReservedActionMnemonic.
            reserved_action_mnemonics: Vec::new(),
            // BAZEL_SETUP: builder.setRunfilesPrefix("_main")
            runfiles_prefix: "_main".to_string(),
        }
    }

    /// `getFragmentRegistry().getOptionsClasses()`.
    pub fn get_options_classes(&self) -> &[FragmentOptionsClass] {
        self.fragment_registry.get_options_classes()
    }
}

impl GlobalStateProvider for BazelRuleClassProvider {
    // Java: com/google/devtools/build/lib/analysis/ConfiguredRuleClassProvider.java:878 ConfiguredRuleClassProvider.getActionEnvironment
    fn get_action_environment(&self, options: &BuildOptions) -> ActionEnvironment {
        // BAZEL_SETUP: setActionEnvironmentProvider(SHELL_ACTION_ENV)
        shell_action_env(options)
    }

    fn get_fragment_registry(&self) -> &FragmentRegistry {
        &self.fragment_registry
    }

    fn get_reserved_action_mnemonics(&self) -> &[String] {
        &self.reserved_action_mnemonics
    }

    fn get_runfiles_prefix(&self) -> &str {
        &self.runfiles_prefix
    }
}
