//! Port of `analysis/AliasProvider.java`.

use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_packages::target::{Rule, Target};
use blaze_util::check_argument;

use crate::configured_target::ConfiguredTarget;
use crate::configured_target_and_data::ConfiguredTargetAndData;
use crate::provider_map::TransitiveInfoProvider;
use crate::rule::configured_target_and_data_ext::ConfiguredTargetAndDataExt;

/// A provider that gives information about the aliases a rule was resolved through.
// Java: com/google/devtools/build/lib/analysis/AliasProvider.java:33 AliasProvider
#[derive(Clone, Debug)]
pub struct AliasProvider {
    /// The chain of aliases, starting with the alias the target was reached through.
    alias_chain: Vec<Label>,
}

/// How to describe a target in error messages.
// Java: com/google/devtools/build/lib/analysis/AliasProvider.java:110 AliasProvider.TargetMode
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum TargetMode {
    /// Specify the kind of the target.
    WithKind,
    /// Only say "target".
    WithoutKind,
}

/// The marker provider of late-bound aliases (Java `AliasProvider.LateBoundAliasProvider`).
// Java: com/google/devtools/build/lib/analysis/AliasProvider.java:155 AliasProvider.LateBoundAliasProvider
#[derive(Clone, Debug)]
pub struct LateBoundAliasProvider;

impl TransitiveInfoProvider for LateBoundAliasProvider {
    fn java_class_name(&self) -> &'static str {
        "LateBoundAliasProvider"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl AliasProvider {
    // Java: com/google/devtools/build/lib/analysis/AliasProvider.java:44 AliasProvider.fromAliasRule
    pub fn from_alias_rule(alias_rule: &Arc<Rule>, actual: &ConfiguredTarget) -> AliasProvider {
        check_argument!(
            Self::may_be_alias(&Target::Rule(alias_rule.clone())),
            "{} does not advertise AliasProvider",
            alias_rule
        );
        let chain = match crate::configuredtargets::get_provider::<AliasProvider>(actual) {
            None => vec![alias_rule.get_label().clone()],
            Some(dep) => {
                let mut c = Vec::with_capacity(dep.alias_chain.len() + 1);
                c.push(alias_rule.get_label().clone());
                c.extend(dep.alias_chain.iter().cloned());
                c
            }
        };
        AliasProvider { alias_chain: chain }
    }

    /// Returns the label by which it was referred to in the BUILD file.
    ///
    /// For non-alias rules, it's the label of the rule itself, for alias rules, it's the label
    /// of the alias rule.
    // Java: com/google/devtools/build/lib/analysis/AliasProvider.java:68 AliasProvider.getDependencyLabel
    pub fn get_dependency_label(dep: &ConfiguredTarget) -> Label {
        match crate::configuredtargets::get_provider::<AliasProvider>(dep) {
            Some(a) => a.alias_chain[0].clone(),
            None => crate::configuredtargets::get_label(dep),
        }
    }

    /// Returns all labels by which it can be referred to in the BUILD file.
    ///
    /// For non-alias rules, it's the label of the rule itself. For alias rules, they're the label
    /// of the alias and the label of alias' target rule.
    // Java: com/google/devtools/build/lib/analysis/AliasProvider.java:79 AliasProvider.getDependencyLabels
    pub fn get_dependency_labels(dep: &ConfiguredTarget) -> Vec<Label> {
        match crate::configuredtargets::get_provider::<AliasProvider>(dep) {
            Some(a) => vec![
                a.alias_chain[0].clone(),
                crate::configuredtargets::get_label(dep),
            ],
            None => vec![crate::configuredtargets::get_label(dep)],
        }
    }

    /// Returns the list of aliases from top to bottom (i.e. the last alias depends on the actual
    /// resolved target and the first alias is the one that was in the attribute of the rule
    /// currently being analyzed)
    // Java: com/google/devtools/build/lib/analysis/AliasProvider.java:91 AliasProvider.getAliasChain
    pub fn get_alias_chain(&self) -> &[Label] {
        &self.alias_chain
    }

    /// Prints a nice description of a target, also adding the alias chain it was reached
    /// through.
    // Java: com/google/devtools/build/lib/analysis/AliasProvider.java:115 AliasProvider.describeTargetWithAliases
    pub fn describe_target_with_aliases(
        target: &ConfiguredTargetAndData,
        target_mode: TargetMode,
    ) -> String {
        let kind = if target_mode == TargetMode::WithKind {
            target.get_target_kind()
        } else {
            "target".to_owned()
        };
        let Some(alias_provider) =
            crate::configuredtargets::get_provider::<AliasProvider>(target.get_configured_target())
        else {
            return format!("{kind} '{}'", target.get_target_label());
        };
        let alias_chain = &alias_provider.alias_chain;
        let mut result = String::new();
        result.push_str(&format!("alias '{}'", alias_chain[0]));
        result.push_str(&format!(
            " referring to {kind} '{}'",
            target.get_target_label()
        ));
        if alias_chain.len() > 1 {
            let rest: Vec<String> = alias_chain[1..].iter().map(|l| l.to_string()).collect();
            result.push_str(&format!(" through '{}'", rest.join("' -> '")));
        }
        result
    }

    // Java: com/google/devtools/build/lib/analysis/AliasProvider.java:140 AliasProvider.isAlias
    pub fn is_alias(dep: &ConfiguredTarget) -> bool {
        crate::configuredtargets::get_provider::<AliasProvider>(dep).is_some()
    }

    /// Returns whether the given target may be an alias (advertises `AliasProvider`).
    // Java: com/google/devtools/build/lib/analysis/AliasProvider.java:145 AliasProvider.mayBeAlias
    pub fn may_be_alias(target: &Target) -> bool {
        let Target::Rule(rule) = target else {
            return false;
        };
        let provider_set = rule.get_rule_class_object().get_advertised_providers();
        provider_set.can_have_any_provider()
            || provider_set
                .get_builtin_providers()
                .contains("AliasProvider")
            || provider_set
                .get_builtin_providers()
                .contains("LateBoundAliasProvider")
    }
}

impl TransitiveInfoProvider for AliasProvider {
    fn java_class_name(&self) -> &'static str {
        "AliasProvider"
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}
