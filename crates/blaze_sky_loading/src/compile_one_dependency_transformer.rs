//! Port of `pkgcache/CompileOneDependencyTransformer.java`: the implementation of
//! `--compile_one_dependency`.

use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_cmdline::{TargetParsingException, TargetPatternsCode};
use blaze_events::ExtendedEventHandler;
use blaze_packages::attribute::AttributeMap;
use blaze_packages::attribute::raw_attribute_mapper::RawAttributeMapper;
use blaze_packages::package::Package;
use blaze_packages::package::package_args::FeatureSet;
use blaze_packages::target::{Rule, RuleOrMacroInstance, Target};
use blaze_packages::types::file_type::FileType;
use blaze_packages::types::{AttrValue, Type};

use crate::target_pattern_eval::TargetPatternEvalError;
use crate::target_pattern_function::{
    ProviderError, RecursivePackageProvider, target_parsing_exception_of,
};

// Java: com/google/devtools/build/lib/pkgcache/CompileOneDependencyTransformer.java:51 CompileOneDependencyTransformer.CC_FILE_TYPE
fn cc_file_type() -> FileType {
    FileType::of(&[".cc", ".h", ".c"])
}

// Java: com/google/devtools/build/lib/pkgcache/CompileOneDependencyTransformer.java:52 CompileOneDependencyTransformer.JAVA_FILE_TYPE
fn java_file_type() -> FileType {
    FileType::of(&[".java"])
}

// Java: com/google/devtools/build/lib/pkgcache/CompileOneDependencyTransformer.java:53 CompileOneDependencyTransformer.PYTHON_FILE_TYPE
fn python_file_type() -> FileType {
    FileType::of(&[".py"])
}

/// `PREFERRED_RULES.getOrDefault(ruleClass, Predicates.alwaysFalse()).apply(name)`.
// Java: com/google/devtools/build/lib/pkgcache/CompileOneDependencyTransformer.java:55 CompileOneDependencyTransformer.PREFERRED_RULES
fn preferred_rule_applies(rule_class: &str, name: &str) -> bool {
    match rule_class {
        "cc_library" | "cc_binary" | "cc_test" => cc_file_type().apply(name),
        "java_library" => java_file_type().apply(name),
        "py_library" => python_file_type().apply(name),
        _ => false,
    }
}

/// Implementation of `--compile_one_dependency`.
// Java: com/google/devtools/build/lib/pkgcache/CompileOneDependencyTransformer.java:48 CompileOneDependencyTransformer
pub struct CompileOneDependencyTransformer<'p> {
    package_provider: &'p dyn RecursivePackageProvider,
}

impl<'p> CompileOneDependencyTransformer<'p> {
    // Java: com/google/devtools/build/lib/pkgcache/CompileOneDependencyTransformer.java:68 CompileOneDependencyTransformer.<init>
    pub fn new(package_provider: &'p dyn RecursivePackageProvider) -> Self {
        CompileOneDependencyTransformer { package_provider }
    }

    /// For each input file in the original result, returns a rule in the same package which has
    /// the input file as a source.
    ///
    /// PORT: Java takes and returns `ResolvedTargets<Target>`; when the original has an error it
    /// is returned unchanged, which the caller (`TargetPatternPhaseFunction.mergeAll`) checks
    /// before calling this. The result is the transformed targets in order (Java builds a new
    /// `ResolvedTargets` from them: no filtered targets, no error).
    // Java: com/google/devtools/build/lib/pkgcache/CompileOneDependencyTransformer.java:76 CompileOneDependencyTransformer.transformCompileOneDependency
    pub fn transform_compile_one_dependency<'t>(
        &self,
        event_handler: &dyn ExtendedEventHandler,
        original: impl IntoIterator<Item = &'t Target>,
    ) -> Result<Vec<Target>, TargetPatternEvalError> {
        let mut result = Vec::new();
        for target in original {
            result.push(self.transform_target(event_handler, target)?);
        }
        Ok(result)
    }

    // Java: com/google/devtools/build/lib/pkgcache/CompileOneDependencyTransformer.java:89 CompileOneDependencyTransformer.transformCompileOneDependency
    fn transform_target(
        &self,
        event_handler: &dyn ExtendedEventHandler,
        target: &Target,
    ) -> Result<Target, TargetPatternEvalError> {
        if !target.is_file() {
            return Err(TargetParsingException::new(
                format!(
                    "--compile_one_dependency target '{}' must be a file",
                    target.get_label()
                ),
                TargetPatternsCode::TargetMustBeAFile,
            )
            .into());
        }

        let pkg = match self
            .package_provider
            .get_package(event_handler, target.get_label().get_package_identifier())
        {
            Ok(pkg) => pkg,
            Err(ProviderError::Eval(e)) => return Err(e),
            // Only possible if lazy macro expansion is enabled, and an error was encountered
            // when loading a different package piece of this package.
            Err(ProviderError::NoSuchThing(e)) => {
                return Err(target_parsing_exception_of(
                    format!(
                        "Package of --compile_one_dependency target '{}' could not be loaded",
                        target.get_label()
                    ),
                    &e.get_detailed_exit_code(),
                )
                .into());
            }
        };
        let ordered_rule_list = get_ordered_rule_list(&pkg);

        let mut result: Option<&Arc<Rule>> = None;
        for rule in &ordered_rule_list {
            let labels: Vec<Label> = get_input_labels(rule).into_iter().collect();
            if self.list_contains_file(
                event_handler,
                &labels,
                target.get_label(),
                &mut HashSet::new(),
            )? {
                if preferred_rule_applies(rule.get_rule_class(), target.get_name()) {
                    result = Some(rule);
                    break;
                }
                if result.is_none() {
                    result = Some(rule);
                }
            }
        }

        let Some(result) = result else {
            return Err(TargetParsingException::new(
                format!(
                    "Couldn't find dependency on target '{}'",
                    target.get_label()
                ),
                TargetPatternsCode::DependencyNotFound,
            )
            .into());
        };

        // We want a rule where some action processes the input.
        // We should avoid cc_library rules that describe a set of headers but don't compile them.

        // If parse_headers is (probably) enabled, then this rule has a CppCompileHeader action.
        if has_parse_headers_heuristic(result) {
            return Ok(Target::Rule(result.clone()));
        }
        // If the rule has source targets, return it: one of those sources will parse the header.
        if result
            .get_rule_class_object()
            .get_attribute_provider()
            .has_attr("srcs", Type::LabelList)
            && !label_list(
                RawAttributeMapper::of(&**result).get_merged_values("srcs", Type::LabelList),
            )
            .is_empty()
        {
            return Ok(Target::Rule(result.clone()));
        }

        // Else, find a rule in the same package that has 'result' as a dependency.
        for rule in &ordered_rule_list {
            let attributes = RawAttributeMapper::of(&***rule);
            // We don't know which path to follow for configurable attributes, so skip them.
            if attributes.is_configurable("deps") || attributes.is_configurable("srcs") {
                continue;
            }
            let rule_class = rule.get_rule_class_object();
            if rule_class
                .get_attribute_provider()
                .has_attr("deps", Type::LabelList)
                && rule_class
                    .get_attribute_provider()
                    .has_attr("srcs", Type::LabelList)
            {
                for dep in label_list(attributes.get("deps", Type::LabelList)) {
                    if dep == *result.get_label()
                        && !label_list(attributes.get("srcs", Type::LabelList)).is_empty()
                    {
                        return Ok(Target::Rule((*rule).clone()));
                    }
                }
            }
        }

        Ok(Target::Rule(result.clone()))
    }

    /// Returns true if a specific rule compiles a specific source. Looks through genrules and
    /// filegroups.
    // Java: com/google/devtools/build/lib/pkgcache/CompileOneDependencyTransformer.java:214 CompileOneDependencyTransformer.listContainsFile
    fn list_contains_file(
        &self,
        event_handler: &dyn ExtendedEventHandler,
        src_labels: &[Label],
        source: &Label,
        visited_rule_labels: &mut HashSet<Label>,
    ) -> Result<bool, TargetPatternEvalError> {
        if src_labels.contains(source) {
            return Ok(true);
        }
        for label in src_labels {
            if !visited_rule_labels.insert(label.clone()) {
                continue;
            }

            let target = match self.package_provider.get_target(event_handler, label) {
                Ok(t) => Some(t),
                // Just ignore failing sources/packages. We could report them here, but as long
                // as we do early return, the presence of this error would then be determined by
                // the order of items in the srcs attribute. A proper error will be created by
                // the subsequent loading.
                Err(ProviderError::NoSuchThing(_)) => None,
                Err(ProviderError::Eval(e)) => return Err(e),
            };

            let Some(target) = target else {
                continue;
            };
            if target.is_file() {
                continue;
            }
            let Some(target_rule) = target.get_associated_rule() else {
                continue;
            };

            if target_rule.get_rule_class() == "filegroup" {
                let srcs = label_list(
                    RawAttributeMapper::of(&**target_rule)
                        .get_merged_values("srcs", Type::LabelList),
                );
                if self.list_contains_file(event_handler, &srcs, source, visited_rule_labels)? {
                    return Ok(true);
                }
            } else if target_rule.get_rule_class() == "genrule" {
                // TODO(djasper): Likely, it makes much more sense to look at the inputs of a
                // genrule.
                for file in target_rule.get_output_files() {
                    if file.get_label() == source {
                        return Ok(true);
                    }
                }
            }
        }
        Ok(false)
    }
}

// Java: com/google/devtools/build/lib/pkgcache/CompileOneDependencyTransformer.java:168 CompileOneDependencyTransformer.hasParseHeadersHeuristic
pub fn has_parse_headers_heuristic(rule: &Rule) -> bool {
    // We want to know whether the "parse_headers" toolchain feature is enabled or disabled.
    // At load time we can't really know, so check for the common static configuration sources.
    // (We ignore parse_headers being disabled through the toolchain & by rule implementations).

    let mut merged_features = rule
        .get_package_declarations()
        .get_package_args()
        .features()
        .clone();
    let rule_attrs = RawAttributeMapper::of(rule);
    if rule_attrs.has_typed("features", Type::StringList) && !rule_attrs.is_configurable("features")
    {
        let features: Vec<String> = match rule_attrs.get("features", Type::StringList) {
            Some(AttrValue::StringList(l)) => l,
            None => Vec::new(),
            Some(other) => panic!("ClassCastException: {other:?} is not a list of strings"),
        };
        let rule_features = FeatureSet::parse(&features);
        merged_features = FeatureSet::merge(&merged_features, &rule_features);
    }

    if merged_features.on.contains("parse_headers") {
        return true;
    }
    if merged_features.off.contains("parse_headers") {
        return false;
    }

    // We assume parse_headers is on globally, unless disabled locally.
    true
}

/// Returns a list of rules in the given package sorted by BUILD file order. When multiple rules
/// depend on a target, we choose the first match in this list (after filtering for preferred
/// dependencies - see below).
// Java: com/google/devtools/build/lib/pkgcache/CompileOneDependencyTransformer.java:198 CompileOneDependencyTransformer.getOrderedRuleList
fn get_ordered_rule_list(pkg: &Package) -> Vec<&Arc<Rule>> {
    let mut ordered_list: Vec<&Arc<Rule>> = pkg.get_rules().collect();
    // Collections.sort is stable; Location.compareTo orders by file, then line, then column.
    ordered_list.sort_by(|a, b| a.get_location().cmp(b.get_location()));
    ordered_list
}

/// Returns all labels that are contained in direct compile time inputs of `rule`.
// Java: com/google/devtools/build/lib/pkgcache/CompileOneDependencyTransformer.java:260 CompileOneDependencyTransformer.getInputLabels
fn get_input_labels(rule: &Rule) -> BTreeSet<Label> {
    let attribute_mapper = RawAttributeMapper::of(rule);
    let mut labels = BTreeSet::new();
    for attr_name in attribute_mapper.get_attribute_names() {
        if !attribute_mapper
            .get_attribute_definition(&attr_name)
            .expect("an attribute of the rule")
            .is_direct_compile_time_input()
        {
            continue;
        }
        // TODO(djasper): We might also want to look at LABEL types, but there currently is the
        // attribute xcode_config, which leads to test errors in Bazel tests.
        if rule.is_attr_defined(&attr_name, Type::LabelList) {
            labels.extend(label_list(
                attribute_mapper.get_merged_values(&attr_name, Type::LabelList),
            ));
        }
    }
    labels
}

/// The labels of a `LABEL_LIST` value (Java null is an empty list here: every caller iterates or
/// tests emptiness).
fn label_list(value: Option<AttrValue>) -> Vec<Label> {
    match value {
        Some(AttrValue::LabelList(l)) => l,
        None => Vec::new(),
        Some(other) => panic!("ClassCastException: {other:?} is not a list of labels"),
    }
}
