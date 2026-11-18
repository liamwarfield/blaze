//! Port of `query2/query/BlazeTargetAccessor.java`.
//!
//! PORT: Java's accessor holds its query environment. A Rust environment cannot hold a struct
//! that points back to itself, so environments implement `TargetAccessor<Target>` themselves (as
//! the `QueryEnvironment.getAccessor` javadoc allows) by delegating to these functions.

use std::collections::HashSet;
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_packages::attribute::{AggregatingAttributeMapper, AttributeMap};
use blaze_packages::package::RuleVisibility;
use blaze_packages::target::{Target, target_utils};
use blaze_query_engine::query_visibility;
use blaze_query_engine::{QueryError, QueryExpression, QueryVisibility, TargetAccessor};

use crate::common::abstract_blaze_query_environment::{
    AbstractBlazeQueryEnvironment, GetTargetForQueryError, get_targets,
};
use crate::query::blaze_query_visibility::BlazeQueryVisibility;

// Java: com/google/devtools/build/lib/query2/query/BlazeTargetAccessor.java:51 BlazeTargetAccessor.getTargetKind
pub fn get_target_kind(target: &Target) -> String {
    target.get_target_kind()
}

// Java: com/google/devtools/build/lib/query2/query/BlazeTargetAccessor.java:56 BlazeTargetAccessor.getLabel
pub fn get_label(target: &Target) -> String {
    target.get_label().to_string()
}

// Java: com/google/devtools/build/lib/query2/query/BlazeTargetAccessor.java:61 BlazeTargetAccessor.getPackage
pub fn get_package(target: &Target) -> String {
    target.get_package_metadata().get_name().to_owned()
}

// Java: com/google/devtools/build/lib/query2/query/BlazeTargetAccessor.java:66 BlazeTargetAccessor.getPrerequisites
pub fn get_prerequisites(
    query_environment: &dyn AbstractBlazeQueryEnvironment,
    caller: &QueryExpression,
    target: &Target,
    attr_name: &str,
    error_msg_prefix: &str,
) -> Result<Vec<Target>, QueryError> {
    let rule = match target {
        Target::Rule(rule) => rule,
        _ => panic!("IllegalArgumentException"),
    };
    let attr_map = AggregatingAttributeMapper::of(&**rule);
    let attr_type = attr_map.base().get_attribute_type(attr_name);
    if attr_type.is_none() {
        return Ok(vec![]);
    }

    let labels: Vec<Label> = attr_map
        .get_reachable_labels(attr_name, false)
        .into_iter()
        .collect();
    let label_target_map = get_targets(query_environment, &labels)?;
    if label_target_map.len() != labels.len() {
        for label in &labels {
            if !label_target_map.iter().any(|(l, _)| l == label) {
                match query_environment.get_target(label) {
                    Ok(_) => {}
                    Err(GetTargetForQueryError::TargetNotFound(e)) => {
                        query_environment.base().handle_error(
                            caller,
                            &format!("{}{}", error_msg_prefix, e.get_message()),
                            Some(e.get_detailed_exit_code()),
                        )?;
                    }
                    Err(GetTargetForQueryError::Query(e)) => return Err(e.into()),
                    Err(GetTargetForQueryError::Interrupted(e)) => return Err(e.into()),
                }
            }
        }
    }
    Ok(label_target_map.into_iter().map(|(_, t)| t).collect())
}

// Java: com/google/devtools/build/lib/query2/query/BlazeTargetAccessor.java:91 BlazeTargetAccessor.getStringListAttr
pub fn get_string_list_attr(target: &Target, attr_name: &str) -> Vec<String> {
    target_utils::get_string_list_attr(target, attr_name)
}

// Java: com/google/devtools/build/lib/query2/query/BlazeTargetAccessor.java:96 BlazeTargetAccessor.getStringAttr
pub fn get_string_attr(target: &Target, attr_name: &str) -> Option<String> {
    target_utils::get_string_attr(target, attr_name)
}

/// PORT: Java's `Iterable<String>` may hold nulls, which `RegexFilterExpression` skips; they are
/// dropped here.
// Java: com/google/devtools/build/lib/query2/query/BlazeTargetAccessor.java:101 BlazeTargetAccessor.getAttrAsString
pub fn get_attr_as_string(target: &Target, attr_name: &str) -> Vec<String> {
    target_utils::get_attr_as_string(target, attr_name)
        .into_iter()
        .flatten()
        .collect()
}

// Java: com/google/devtools/build/lib/query2/query/BlazeTargetAccessor.java:106 BlazeTargetAccessor.isRule
pub fn is_rule(target: &Target) -> bool {
    matches!(target, Target::Rule(_))
}

// Java: com/google/devtools/build/lib/query2/query/BlazeTargetAccessor.java:111 BlazeTargetAccessor.isExecutableNonTestRule
pub fn is_executable_non_test_rule(target: &Target) -> bool {
    target_utils::is_executable_non_test_rule(target)
}

// Java: com/google/devtools/build/lib/query2/query/BlazeTargetAccessor.java:116 BlazeTargetAccessor.isTestRule
pub fn is_test_rule(target: &Target) -> bool {
    target_utils::is_test_rule(target)
}

// Java: com/google/devtools/build/lib/query2/query/BlazeTargetAccessor.java:121 BlazeTargetAccessor.isTestSuite
pub fn is_test_suite(target: &Target) -> bool {
    target_utils::is_test_suite_rule(target)
}

/// `accessor` is the environment's `TargetAccessor` (Java `this`).
// Java: com/google/devtools/build/lib/query2/query/BlazeTargetAccessor.java:126 BlazeTargetAccessor.getVisibility
pub fn get_visibility<'a>(
    query_environment: &dyn AbstractBlazeQueryEnvironment,
    accessor: &'a dyn TargetAccessor<Target>,
    caller: &QueryExpression,
    target: &Target,
) -> Result<Vec<Arc<dyn QueryVisibility<Target> + 'a>>, QueryError> {
    // PORT: the visibilities borrow the single-threaded accessor; `Arc` is the element type of
    // QueryVisibility lists.
    #[allow(clippy::arc_with_non_send_sync, clippy::vec_init_then_push)]
    let mut result: Vec<Arc<dyn QueryVisibility<Target> + 'a>> = {
        let mut result: Vec<Arc<dyn QueryVisibility<Target> + 'a>> = Vec::new();
        result.push(Arc::new(query_visibility::same_package(target, accessor)));
        result
    };
    convert_visibility(query_environment, caller, &mut result, target)?;
    // ImmutableSet.Builder: the visibilities are distinct objects.
    Ok(result)
}

// Java: com/google/devtools/build/lib/query2/query/BlazeTargetAccessor.java:134 BlazeTargetAccessor.convertVisibility
fn convert_visibility<'a>(
    query_environment: &dyn AbstractBlazeQueryEnvironment,
    caller: &QueryExpression,
    package_specifications: &mut Vec<Arc<dyn QueryVisibility<Target> + 'a>>,
    target: &Target,
) -> Result<(), QueryError> {
    let rule_visibility = target.get_visibility();
    match &rule_visibility {
        RuleVisibility::Private => Ok(()),
        RuleVisibility::Public => {
            package_specifications.push(Arc::new(query_visibility::everything()));
            Ok(())
        }
        RuleVisibility::PackageGroups(package_groups_visibility) => {
            for group_label in package_groups_visibility.get_package_groups() {
                match add_all_package_groups(query_environment, group_label, package_specifications)
                {
                    Ok(()) => {}
                    Err(AddPackageGroupsError::TargetNotFound(e)) => {
                        query_environment.base().handle_error(
                            caller,
                            &format!(
                                "Invalid visibility label '{}': {}",
                                group_label.get_canonical_form(),
                                e.get_message()
                            ),
                            Some(e.get_detailed_exit_code()),
                        )?;
                    }
                    Err(AddPackageGroupsError::Other(e)) => return Err(e),
                }
            }
            package_specifications.push(Arc::new(BlazeQueryVisibility::new(
                package_groups_visibility.get_direct_packages().clone(),
            )));
            Ok(())
        }
    }
}

enum AddPackageGroupsError {
    TargetNotFound(blaze_query_engine::TargetNotFoundException),
    Other(QueryError),
}

impl From<GetTargetForQueryError> for AddPackageGroupsError {
    fn from(e: GetTargetForQueryError) -> Self {
        match e {
            GetTargetForQueryError::TargetNotFound(e) => AddPackageGroupsError::TargetNotFound(e),
            GetTargetForQueryError::Query(e) => AddPackageGroupsError::Other(e.into()),
            GetTargetForQueryError::Interrupted(e) => AddPackageGroupsError::Other(e.into()),
        }
    }
}

// Java: com/google/devtools/build/lib/query2/query/BlazeTargetAccessor.java:159 BlazeTargetAccessor.addAllPackageGroups
fn add_all_package_groups<'a>(
    query_environment: &dyn AbstractBlazeQueryEnvironment,
    group_label: &Label,
    package_specifications: &mut Vec<Arc<dyn QueryVisibility<Target> + 'a>>,
) -> Result<(), AddPackageGroupsError> {
    add_all_package_groups_recursive(
        query_environment,
        group_label,
        package_specifications,
        &mut HashSet::new(),
    )
}

// Java: com/google/devtools/build/lib/query2/query/BlazeTargetAccessor.java:166 BlazeTargetAccessor.addAllPackageGroupsRecursive
fn add_all_package_groups_recursive<'a>(
    query_environment: &dyn AbstractBlazeQueryEnvironment,
    group_label: &Label,
    package_specifications: &mut Vec<Arc<dyn QueryVisibility<Target> + 'a>>,
    seen: &mut HashSet<Label>,
) -> Result<(), AddPackageGroupsError> {
    if !seen.insert(group_label.clone()) {
        return Ok(());
    }
    let group_target = query_environment.get_target(group_label)?;
    if let Target::PackageGroup(package_group_target) = &group_target {
        for include in package_group_target.get_includes() {
            add_all_package_groups_recursive(
                query_environment,
                include,
                package_specifications,
                seen,
            )?;
        }
        package_specifications.push(Arc::new(BlazeQueryVisibility::new(
            package_group_target.get_package_specifications().clone(),
        )));
    }
    Ok(())
}
