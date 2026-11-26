//! Port of `rules/extra/ActionListener.java`.

use std::collections::{BTreeMap, HashSet};

use blaze_actions::analysis::runfiles::Runfiles;
use blaze_analysis::configured_rule_class_provider::{
    CreateConfiguredTargetError, RuleConfiguredTargetFactory,
};
use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::configuredtargets::get_provider;
use blaze_analysis::extra::extra_action_map_provider::ExtraActionMapProvider;
use blaze_analysis::extra::extra_action_spec::ExtraActionSpec;
use blaze_analysis::providers::file_provider::RunfilesProvider;
use blaze_analysis::rule::rule_error_consumer::RuleErrorConsumer as _;
use blaze_analysis::rule::{RuleConfiguredTargetBuilder, RuleContext};
use blaze_packages::attribute::AttributeMap;
use blaze_packages::types::Type;

use super::build_error;

/// Implementation for the 'action_listener' rule.
// Java: com/google/devtools/build/lib/rules/extra/ActionListener.java:38 ActionListener
pub struct ActionListener;

impl RuleConfiguredTargetFactory for ActionListener {
    // Java: com/google/devtools/build/lib/rules/extra/ActionListener.java:41 ActionListener.create
    fn create(
        &self,
        rule_context: &mut RuleContext,
    ) -> Result<Option<ConfiguredTarget>, CreateConfiguredTargetError> {
        // This rule doesn't produce any output when listed as a build target.
        // Only when used via the --experimental_action_listener flag,
        // this rule instructs the build system to add additional outputs.

        // Sets.newHashSet(...): the iteration order does not matter, the map's keys are sorted.
        let mnemonics: HashSet<String> = rule_context
            .attributes()
            .get("mnemonics", Type::StringList)
            .and_then(|v| v.as_string_list().map(|l| l.to_vec()))
            .unwrap_or_else(|| panic!("IllegalArgumentException: no attribute 'mnemonics'"))
            .into_iter()
            .collect();
        let extra_actions = retrieve_and_validate_extra_actions(rule_context);
        // ImmutableSortedKeyListMultimap.builder()
        let mut extra_action_map: BTreeMap<String, Vec<ExtraActionSpec>> = BTreeMap::new();
        for mnemonic in mnemonics {
            extra_action_map
                .entry(mnemonic)
                .or_default()
                .extend(extra_actions.iter().cloned());
        }
        let mut builder = RuleConfiguredTargetBuilder::new(rule_context);
        builder
            .add_provider(RunfilesProvider::simple(Runfiles::empty()))
            .add_provider(ExtraActionMapProvider::new(extra_action_map));
        builder.build().map_err(build_error)
    }
}

/// Loads the targets listed in the 'extra_actions' attribute of this rule. Validates these
/// targets to be extra_actions indeed. And checks if the blaze version number is in the range of
/// the blaze_version restrictions on the rule.
// Java: com/google/devtools/build/lib/rules/extra/ActionListener.java:71 ActionListener.retrieveAndValidateExtraActions
fn retrieve_and_validate_extra_actions(rule_context: &RuleContext) -> Vec<ExtraActionSpec> {
    let mut extra_actions = Vec::new();
    for prerequisite in rule_context.get_prerequisites("extra_actions") {
        match get_provider::<ExtraActionSpec>(&prerequisite) {
            None => rule_context.attribute_error(
                "extra_actions",
                &format!(
                    "target {} is not an extra_action rule",
                    prerequisite.get_label()
                ),
            ),
            Some(spec) => extra_actions.push(spec.clone()),
        }
    }
    if extra_actions.is_empty() {
        rule_context.attribute_warning(
            "extra_actions",
            "No extra_action is specified for this version of bazel.",
        );
    }
    extra_actions
}
