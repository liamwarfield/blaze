//! Port of `packages/LabelVisitationUtils.java`: visiting the labels of the loading-phase deps of
//! a target entailed by its attribute values (not aspect-entailed deps).

use std::sync::LazyLock;

use blaze_cmdline::Label;

use crate::attribute::AggregatingAttributeMapper;
use crate::attribute::Attribute;
use crate::attribute::AttributeMap;
use crate::attribute::DependencyFilter;
use crate::attribute::attr;
use crate::target::OutputFile;
use crate::target::PackageGroup;
use crate::target::Rule;
use crate::target::Target;
use crate::types::FileTypeSet;
use crate::types::Type;

// An attribute which symbolizes the "toolchains" parameter of rule class definitions
// (user-specified via the `toolchains` parameter of the starlark rule() function). This is so
// that labels specified in this `toolchains` parameter may be treated the same as dependencies
// defined on an implicit rule attribute. This "fake" attribute uses an obscure placeholder name
// to prevent dependencies on this implementation detail.
// Java: com/google/devtools/build/lib/packages/LabelVisitationUtils.java:36 LabelVisitationUtils.TOOLCHAIN_TYPE_ATTR_FOR_FILTERING
static TOOLCHAIN_TYPE_ATTR_FOR_FILTERING: LazyLock<Attribute> = LazyLock::new(|| {
    attr("_hidden_toolchain_types", Type::LabelList)
        .allowed_file_types(FileTypeSet::NoFile)
        .build()
});

/// Processes the label of one dep (Java `LabelVisitationUtils.LabelProcessor.process`): the
/// target that has the dep, the attribute whose value entailed it (if any), and the dep's label.
// Java: com/google/devtools/build/lib/packages/LabelVisitationUtils.java:44 LabelVisitationUtils.LabelProcessor
pub type LabelProcessor<'a> = dyn FnMut(&Target, Option<&Attribute>, &Label) + 'a;

/// Visits the loading-phase deps of `target` that satisfy `edge_filter`.
// Java: com/google/devtools/build/lib/packages/LabelVisitationUtils.java:59 LabelVisitationUtils.visitTarget
pub fn visit_target(
    target: &Target,
    edge_filter: &DependencyFilter,
    label_processor: &mut LabelProcessor<'_>,
) {
    match target {
        Target::OutputFile(output_file) => {
            visit_output_file(target, output_file, label_processor);
        }
        Target::InputFile(_) => {
            visit_target_visibility(target, /* attribute= */ None, label_processor);
        }
        Target::Rule(rule) => {
            visit_rule_visibility(target, rule, edge_filter, label_processor);
            visit_rule(target, rule, edge_filter, label_processor);
            visit_rule_toolchains(target, rule, edge_filter, label_processor);
        }
        Target::PackageGroup(package_group) => {
            visit_package_group(target, package_group, label_processor);
        }
        Target::EnvironmentGroup(_) | Target::FakeLoadTarget(_) => {}
    }
}

fn visit_output_file(
    target: &Target,
    output_file: &OutputFile,
    label_processor: &mut LabelProcessor<'_>,
) {
    label_processor(
        target,
        /* attribute= */ None,
        output_file.get_generating_rule().get_label(),
    );
    visit_target_visibility(target, /* attribute= */ None, label_processor);
}

// Java: com/google/devtools/build/lib/packages/LabelVisitationUtils.java:83 LabelVisitationUtils.visitTargetVisibility
fn visit_target_visibility(
    target: &Target,
    attribute: Option<&Attribute>,
    label_processor: &mut LabelProcessor<'_>,
) {
    for label in target.get_visibility_dependency_labels() {
        label_processor(target, attribute, &label);
    }
}

// Java: com/google/devtools/build/lib/packages/LabelVisitationUtils.java:90 LabelVisitationUtils.visitRuleVisibility
fn visit_rule_visibility(
    target: &Target,
    rule: &Rule,
    edge_filter: &DependencyFilter,
    label_processor: &mut LabelProcessor<'_>,
) {
    let rule_class = rule.get_rule_class_object();
    let Some(index) = rule_class
        .get_attribute_provider()
        .get_attribute_index("visibility")
    else {
        return;
    };
    let visibility_attribute = rule_class.get_attribute_provider().get_attribute(index);
    if visibility_attribute.get_type() != Type::NodepLabelList {
        return;
    }
    if edge_filter.test(rule, visibility_attribute) {
        visit_target_visibility(target, Some(visibility_attribute), label_processor);
    }
}

// Java: com/google/devtools/build/lib/packages/LabelVisitationUtils.java:106 LabelVisitationUtils.visitRuleToolchains
fn visit_rule_toolchains(
    target: &Target,
    rule: &Rule,
    edge_filter: &DependencyFilter,
    label_processor: &mut LabelProcessor<'_>,
) {
    let rule_class = rule.get_rule_class_object();
    if edge_filter.test(rule, &TOOLCHAIN_TYPE_ATTR_FOR_FILTERING) {
        for t in rule_class.get_toolchain_types() {
            label_processor(
                target,
                Some(&TOOLCHAIN_TYPE_ATTR_FOR_FILTERING),
                &t.toolchain_type,
            );
        }
    }
}

// Java: com/google/devtools/build/lib/packages/LabelVisitationUtils.java:116 LabelVisitationUtils.visitRule
fn visit_rule(
    target: &Target,
    rule: &Rule,
    edge_filter: &DependencyFilter,
    label_processor: &mut LabelProcessor<'_>,
) {
    AggregatingAttributeMapper::of(rule).visit_labels_with_filter(
        edge_filter,
        &mut |attribute, label| {
            label_processor(target, Some(attribute), label);
        },
    );
}

// Java: com/google/devtools/build/lib/packages/LabelVisitationUtils.java:128 LabelVisitationUtils.visitPackageGroup
fn visit_package_group(
    target: &Target,
    package_group: &PackageGroup,
    label_processor: &mut LabelProcessor<'_>,
) {
    for label in package_group.get_includes() {
        label_processor(target, /* attribute= */ None, label);
    }
}
