//! Port of `packages/Rule.java`: an instance of a rule class in a package.

use std::collections::BTreeSet;
use std::collections::HashSet;
use std::fmt;
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_cmdline::LabelSyntaxException;
use blaze_cmdline::PackageIdentifier;
use blaze_cmdline::RepositoryName;
use blaze_events::Event;
use blaze_events::EventHandler;
use blaze_optdefs::common::test_timeout::TestTimeout;
use blaze_starlark::Location;
use blaze_starlark::error::StackEntry;
use indexmap::IndexMap;
use indexmap::IndexSet;

use crate::aspect::AspectDefinition;
use crate::attribute::AggregatingAttributeMapper;
use crate::attribute::Attribute;
use crate::attribute::AttributeMap;
use crate::attribute::AttributeProvider;
use crate::attribute::DependencyFilter;
use crate::attribute::ImplicitOutputsFunction;
use crate::attribute::NonconfigurableAttributeMapper;
use crate::attribute::RawAttributeMapper;
use crate::exceptions::PackageLoadingCode;
use crate::macros::MacroInstance;
use crate::package::CallStackNode;
use crate::package::ConfigSettingVisibilityPolicy;
use crate::package::Package;
use crate::package::PackageDeclarations;
use crate::package::PackageMetadata;
use crate::package::RuleVisibility;
use crate::package::TargetDefinitionContext;
use crate::rule_class::AdvertisedProviderSet;
use crate::rule_class::RequiredProviders;
use crate::rule_class::RuleClass;
use crate::target::AttributeContainer;
use crate::target::OutputFile;
use crate::target::PackageSpecification;
use crate::target::RuleOrMacroInstance;
use crate::target::TargetData;
use crate::target::rule_or_macro_instance::GENERATOR_FUNCTION;
use crate::target::rule_or_macro_instance::GENERATOR_LOCATION;
use crate::target::rule_or_macro_instance::GENERATOR_NAME;
use crate::types::AttrValue;
use crate::types::License;
use crate::types::Type;

/// Java `Rule.IS_EXECUTABLE_ATTRIBUTE_NAME`.
// Java: com/google/devtools/build/lib/packages/Rule.java:82 Rule.IS_EXECUTABLE_ATTRIBUTE_NAME
pub const IS_EXECUTABLE_ATTRIBUTE_NAME: &str = "$is_executable";

/// One output file of a rule, as the rule records it (Java: the `OutputFile` objects in
/// `Rule.outputFiles`). [`Rule::get_output_files`] turns them into [`OutputFile`] targets.
// PORT: Java's Rule and OutputFile point at each other; here the rule keeps the data and each
// OutputFile holds an Arc of its rule (no Arc cycle).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleOutput {
    pub label: Label,
    /// The implicit output key, or the attribute name for explicit outputs.
    pub output_key: String,
    pub implicit: bool,
}

/// A rule instance (Java `Rule`). Mutable while its package is built, then shared as
/// `Arc<Rule>`.
// Java: com/google/devtools/build/lib/packages/Rule.java:78 Rule
#[derive(Debug)]
pub struct Rule {
    container: AttributeContainer,
    /// Java `pkg.getMetadata()`.
    package_metadata: Arc<PackageMetadata>,
    /// Java `pkg.getDeclarations()`.
    package_declarations: Arc<PackageDeclarations>,
    /// Java `Target.getDeclaringMacro()` (looked up in the package in Java).
    // PORT: stored on the target (see Java's TODO in Target.getDeclaringMacro).
    declaring_macro: Option<Arc<MacroInstance>>,
    rule_class: Arc<RuleClass>,
    location: Location,
    interior_call_stack: Option<Arc<CallStackNode>>,
    /// Implicit outputs first, then explicit ones (Java `outputFiles`).
    output_files: Vec<RuleOutput>,
    /// Java `outputFiles != null`.
    output_files_populated: bool,
}

impl Rule {
    // Java: com/google/devtools/build/lib/packages/Rule.java:108 Rule.<init>
    pub fn new(
        package_metadata: Arc<PackageMetadata>,
        package_declarations: Arc<PackageDeclarations>,
        declaring_macro: Option<Arc<MacroInstance>>,
        label: Label,
        rule_class: Arc<RuleClass>,
        location: Location,
        interior_call_stack: Option<Arc<CallStackNode>>,
    ) -> Self {
        let attr_count = rule_class.get_attribute_provider().get_attribute_count();
        Rule {
            container: AttributeContainer::new(label, attr_count),
            package_metadata,
            package_declarations,
            declaring_macro,
            rule_class,
            location,
            interior_call_stack,
            output_files: Vec::new(),
            output_files_populated: false,
        }
    }

    // Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:140 RuleOrMacroInstance.getLabel
    pub fn get_label(&self) -> &Label {
        &self.container.label
    }

    // Java: com/google/devtools/build/lib/packages/RuleOrMacroInstance.java:152 RuleOrMacroInstance.getName
    pub fn get_name(&self) -> &str {
        self.container.label.get_name()
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:127 Rule.getPackageMetadata
    pub fn get_package_metadata(&self) -> &Arc<PackageMetadata> {
        &self.package_metadata
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:132 Rule.getPackageDeclarations
    pub fn get_package_declarations(&self) -> &Arc<PackageDeclarations> {
        &self.package_declarations
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:143 Rule.getDeclaringMacro
    pub fn get_declaring_macro(&self) -> Option<&Arc<MacroInstance>> {
        self.declaring_macro.as_ref()
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:147 Rule.getRuleClassObject
    pub fn get_rule_class_object(&self) -> &Arc<RuleClass> {
        &self.rule_class
    }

    /// The rule class name (Java `getRuleClass()`).
    // Java: com/google/devtools/build/lib/packages/Rule.java:158 Rule.getRuleClass
    pub fn get_rule_class(&self) -> &str {
        self.rule_class.get_name()
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:167 Rule.outputsToBindir
    pub fn outputs_to_bindir(&self) -> bool {
        if self.rule_class.get_name() == "genrule" {
            // this is unfortunate...
            NonconfigurableAttributeMapper::of(self)
                .get("output_to_bindir", Type::Boolean)
                .and_then(|v| v.as_bool())
                .expect("NullPointerException: output_to_bindir")
        } else {
            self.rule_class.outputs_to_bindir()
        }
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:174 Rule.isAnalysisTest
    pub fn is_analysis_test(&self) -> bool {
        self.rule_class.is_analysis_test()
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:182 Rule.hasAnalysisTestTransition
    pub fn has_analysis_test_transition(&self) -> bool {
        self.rule_class.has_analysis_test_transition()
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:186 Rule.isBuildSetting
    pub fn is_build_setting(&self) -> bool {
        self.rule_class.get_build_setting().is_some()
    }

    /// Java `pkg.containsErrors()`.
    // Java: com/google/devtools/build/lib/packages/Rule.java:199 Rule.containsErrors
    pub fn contains_errors(&self) -> bool {
        self.package_declarations.contains_errors()
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:203 Rule.hasAspects
    pub fn has_aspects(&self) -> bool {
        self.rule_class.has_aspects()
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:210 Rule.isConfigurableAttribute
    pub fn is_configurable_attribute(&self, attribute_name: &str) -> bool {
        crate::attribute::AbstractAttributeMapper::is_configurable_in(self, attribute_name)
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:224 Rule.getAttributeDefinition
    pub fn get_attribute_definition(&self, attr_name: &str) -> Option<&Attribute> {
        self.rule_class
            .get_attribute_provider()
            .get_attribute_by_name_maybe(attr_name)
    }

    /// The output file targets of this rule, implicit ones first.
    // Java: com/google/devtools/build/lib/packages/Rule.java:240 Rule.getOutputFiles
    pub fn get_output_files(self: &Arc<Self>) -> Vec<Arc<OutputFile>> {
        self.output_files
            .iter()
            .map(|o| {
                Arc::new(OutputFile::new(
                    o.label.clone(),
                    self.clone(),
                    o.output_key.clone(),
                    o.implicit,
                ))
            })
            .collect()
    }

    /// The recorded output entries (implicit first).
    pub fn get_output_entries(&self) -> &[RuleOutput] {
        &self.output_files
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:248 Rule.getImplicitOutputFiles
    pub fn get_implicit_output_files(self: &Arc<Self>) -> Vec<Arc<OutputFile>> {
        let mut result = Vec::new();
        for output in self.get_output_files() {
            if !output.is_implicit() {
                break;
            }
            result.push(output);
        }
        result
    }

    /// Explicit outputs by attribute name.
    // Java: com/google/devtools/build/lib/packages/Rule.java:269 Rule.getExplicitOutputFileMap
    pub fn get_explicit_output_file_map(
        self: &Arc<Self>,
    ) -> IndexMap<String, Vec<Arc<OutputFile>>> {
        let mut result: IndexMap<String, Vec<Arc<OutputFile>>> = IndexMap::new();
        for output in self.get_output_files() {
            if !output.is_implicit() {
                result
                    .entry(output.get_output_key().to_owned())
                    .or_default()
                    .push(output);
            }
        }
        result
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:287 Rule.getStarlarkImplicitOutputFileMap
    pub fn get_starlark_implicit_output_file_map(
        self: &Arc<Self>,
    ) -> IndexMap<String, Arc<OutputFile>> {
        if !self
            .rule_class
            .get_default_implicit_outputs_function()
            .is_starlark()
        {
            return IndexMap::new();
        }
        let mut result = IndexMap::new();
        for output in self.get_output_files() {
            if !output.is_implicit() {
                break;
            }
            let key = output.get_output_key().to_owned();
            if result.insert(key.clone(), output).is_some() {
                // Java: ImmutableMap.Builder.buildOrThrow.
                panic!("IllegalArgumentException: Multiple entries with same key: {key}");
            }
        }
        result
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:309 Rule.getLocation
    pub fn get_location(&self) -> &Location {
        &self.location
    }

    /// The full call stack of the rule's instantiation, outermost first.
    // Java: com/google/devtools/build/lib/packages/Rule.java:322 Rule.reconstructCallStack
    pub fn reconstruct_call_stack(&self) -> Vec<StackEntry> {
        let mut stack = vec![StackEntry {
            name: crate::package::call_stack::TOP_LEVEL.to_owned(),
            location: Some(self.location.clone()),
            source_line: None,
        }];
        let mut node = self.interior_call_stack.as_ref();
        while let Some(n) = node {
            stack.push(n.to_call_stack_entry());
            node = n.next();
        }
        stack
    }

    /// The BUILD-file location from the package name on (Java `getRelativeLocation`).
    // Java: com/google/devtools/build/lib/packages/Rule.java:349 Rule.getRelativeLocation
    fn get_relative_location(&self) -> Option<String> {
        // Determining the workspace root only works reliably if both location and label point to
        // files in the same package.
        //
        // TODO(b/151165647): this logic has always been wrong:
        // it spuriously matches occurrences of the package name earlier in the path.
        let absolute_path = self.location.to_string();
        let pos = absolute_path.find(self.get_label().get_package_name());
        pos.map(|pos| absolute_path[pos..].to_owned())
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:332 Rule.getInteriorCallStack
    pub fn get_interior_call_stack(&self) -> Option<&Arc<CallStackNode>> {
        self.interior_call_stack.as_ref()
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:419 Rule.getGeneratorFunction
    pub fn get_generator_function(&self) -> String {
        match self.get_attr(GENERATOR_FUNCTION) {
            Some(AttrValue::String(val_string)) => val_string,
            _ => String::new(),
        }
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:427 Rule.hasStringAttribute
    fn has_string_attribute(&self, attr_name: &str) -> bool {
        match self.get_attr(attr_name) {
            Some(AttrValue::String(val_string)) => !val_string.is_empty(),
            _ => false,
        }
    }

    /// All labels of all attributes (possible values of selects included).
    // Java: com/google/devtools/build/lib/packages/Rule.java:438 Rule.getLabels
    pub fn get_labels(&self) -> Vec<Label> {
        let mut labels = Vec::new();
        AggregatingAttributeMapper::of(self)
            .visit_all_labels(&mut |_attribute, label| labels.push(label.clone()));
        labels
    }

    /// The labels of the attributes `filter` accepts, sorted and deduplicated.
    // Java: com/google/devtools/build/lib/packages/Rule.java:453 Rule.getSortedLabels
    pub fn get_sorted_labels(&self, filter: &DependencyFilter) -> Vec<Label> {
        let mut labels = BTreeSet::new();
        AggregatingAttributeMapper::of(self).visit_labels_with_filter(
            filter,
            &mut |_attribute, label| {
                labels.insert(label.clone());
            },
        );
        labels.into_iter().collect()
    }

    /// Labels by attribute, for the attributes `filter` accepts (Java `SetMultimap`).
    // Java: com/google/devtools/build/lib/packages/Rule.java:472 Rule.getTransitions
    // PORT: java-hash-order. Java returns a HashMultimap; the pairs are kept here in visitation
    // order, without duplicates.
    pub fn get_transitions(&self, filter: &DependencyFilter) -> Vec<(Attribute, Label)> {
        let mut transitions: Vec<(Attribute, Label)> = Vec::new();
        AggregatingAttributeMapper::of(self).visit_labels_with_filter(
            filter,
            &mut |attribute, label| {
                if !transitions
                    .iter()
                    .any(|(a, l)| a.get_name() == attribute.get_name() && l == label)
                {
                    transitions.push((attribute.clone(), label.clone()));
                }
            },
        );
        transitions
    }

    /// Computes and records the rule's output files, reporting errors.
    // Java: com/google/devtools/build/lib/packages/Rule.java:482 Rule.populateOutputFiles
    pub fn populate_output_files(
        &mut self,
        event_handler: &dyn EventHandler,
        pkg_id: &PackageIdentifier,
    ) -> Result<(), LabelSyntaxException> {
        let implicit_outputs_function = self
            .rule_class
            .get_default_implicit_outputs_function()
            .clone();
        self.populate_output_files_internal(
            event_handler,
            pkg_id,
            &implicit_outputs_function,
            /* check_labels= */ true,
        )
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:491 Rule.populateOutputFilesUnchecked
    pub fn populate_output_files_unchecked(
        &mut self,
        target_definition_context: &TargetDefinitionContext,
        implicit_outputs_function: &ImplicitOutputsFunction,
    ) {
        let pkg_id = target_definition_context.get_package_identifier().clone();
        if let Err(e) = self.populate_output_files_internal(
            blaze_events::NullEventHandler::INSTANCE,
            &pkg_id,
            implicit_outputs_function,
            /* check_labels= */ false,
        ) {
            panic!("IllegalStateException: {e}");
        }
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:516 Rule.populateOutputFilesInternal
    fn populate_output_files_internal(
        &mut self,
        event_handler: &dyn EventHandler,
        pkg_id: &PackageIdentifier,
        implicit_outputs_function: &ImplicitOutputsFunction,
        check_labels: bool,
    ) -> Result<(), LabelSyntaxException> {
        blaze_util::check_state!(!self.output_files_populated);

        let mut outputs: Vec<RuleOutput> = Vec::new();
        // Detects collisions where the same output key is used for both an explicit and implicit
        // entry.
        let mut implicit_output_keys: HashSet<String> = HashSet::new();

        // We need the implicits to appear before the explicits in the final data structure, so we
        // process them first. We check for duplicates while handling the explicits.

        // outputKey: associated dict key if Starlark-defined, empty string otherwise
        // outputName: package-relative path fragment
        let mut implicit_output_handler = |this: &Rule, output_key: String, output_name: String| {
            let label = if check_labels {
                // controls label syntax validation only
                match Label::create_in_package(pkg_id, &output_name) {
                    Ok(label) => label,
                    Err(e) => {
                        this.report_error(
                            &format!(
                                "illegal output file name '{}' in rule {} due to: {}",
                                output_name,
                                this.get_label(),
                                e.get_message()
                            ),
                            event_handler,
                        );
                        return;
                    }
                }
            } else {
                Label::create_unvalidated(pkg_id.clone(), &output_name)
            };
            this.validate_output_label(&label, event_handler);

            outputs.push(RuleOutput {
                label,
                output_key: output_key.clone(),
                implicit: true,
            });
            implicit_output_keys.insert(output_key);
        };

        // Populate the implicit outputs.
        {
            let attribute_map = RawAttributeMapper::of(&*self);
            // TODO(bazel-team): Reconsider the ImplicitOutputsFunction abstraction. It doesn't
            // seem to be a good fit if it forces us to downcast in situations like this.
            let result = if implicit_outputs_function.is_starlark() {
                implicit_outputs_function
                    .calculate_outputs(event_handler, &attribute_map)
                    .map(|entries| {
                        for (key, value) in entries {
                            implicit_output_handler(&*self, key, value);
                        }
                    })
            } else {
                implicit_outputs_function
                    .get_implicit_outputs(event_handler, &attribute_map)
                    .map(|outs| {
                        for out in outs {
                            implicit_output_handler(&*self, String::new(), out);
                        }
                    })
            };
            if let Err(e) = result {
                self.report_error(
                    &format!("In rule {}: {}", self.get_label(), e.get_message()),
                    event_handler,
                );
            }
        }

        // Populate the explicit outputs.
        let mut explicit: Vec<(Attribute, Label)> = Vec::new();
        {
            let non_configurable_attributes = NonconfigurableAttributeMapper::of(&*self);
            for attribute in self.rule_class.get_attribute_provider().get_attributes() {
                let name = attribute.get_name();
                let type_ = attribute.get_type();
                if type_ == Type::Output {
                    if let Some(AttrValue::Label(label)) =
                        non_configurable_attributes.get(name, Type::Output)
                    {
                        explicit.push((attribute.clone(), label));
                    }
                } else if type_ == Type::OutputList
                    && let Some(AttrValue::LabelList(labels)) =
                        non_configurable_attributes.get(name, Type::OutputList)
                {
                    for label in labels {
                        explicit.push((attribute.clone(), label));
                    }
                }
            }
        }
        for (attribute, output_label) in explicit {
            // Java: the ExplicitOutputHandler lambda.
            let attr_name = attribute.get_name();
            if implicit_output_keys.contains(attr_name) {
                self.report_error(
                    &format!(
                        "Implicit output key '{attr_name}' collides with output attribute name"
                    ),
                    event_handler,
                );
            }
            if check_labels {
                if output_label.get_package_identifier()
                    != &self.package_metadata.package_identifier
                {
                    panic!(
                        "IllegalStateException: Label for attribute {} should refer to '{}' but \
                         instead refers to '{}' (label '{}')",
                        attribute,
                        self.package_metadata.get_name(),
                        output_label.get_package_fragment(),
                        output_label.get_name()
                    );
                }
                if output_label.get_name() == "." {
                    return Err(LabelSyntaxException::new(
                        "output file name can't be equal '.'",
                    ));
                }
            }
            self.validate_output_label(&output_label, event_handler);

            outputs.push(RuleOutput {
                label: output_label,
                output_key: attr_name.to_owned(),
                implicit: false,
            });
        }

        self.output_files = outputs;
        self.output_files_populated = true;
        Ok(())
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:632 Rule.validateOutputLabel
    fn validate_output_label(&self, label: &Label, event_handler: &dyn EventHandler) {
        if label.get_name() == self.get_name() {
            // TODO(bazel-team): for now (23 Apr 2008) this is just a warning.  After
            // June 1st we should make it an error.
            self.report_warning(
                &format!(
                    "target '{}' is both a rule and a file; please choose another name for the rule",
                    self.get_name()
                ),
                event_handler,
            );
        }
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:659 Rule.reportWarning
    pub(crate) fn report_warning(&self, message: &str, event_handler: &dyn EventHandler) {
        event_handler.handle(&Event::warn_at(
            Some(crate::package::package::to_event_location(&self.location)),
            message,
        ));
    }

    /// The `visibility` attribute value parsed, or `None` if unset.
    // Java: com/google/devtools/build/lib/packages/Rule.java:684 Rule.getRawVisibility
    pub fn get_raw_visibility(&self) -> Option<RuleVisibility> {
        // The attribute value was already validated when it was set, so call the unchecked
        // method.
        self.get_raw_visibility_labels()
            .map(|raw_labels| RuleVisibility::parse_unchecked(&raw_labels))
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:695 Rule.getDefaultVisibility
    pub fn get_default_visibility(&self) -> RuleVisibility {
        if self.rule_class.get_name() == "bind" {
            return RuleVisibility::Public; // bind rules are always public.
        }
        // Temporary logic to relax config_setting's visibility enforcement while depot migrations
        // set visibility settings properly (legacy code may have visibility settings that would
        // break if enforced). See https://github.com/bazelbuild/bazel/issues/12669. Ultimately
        // this entire conditional should be removed.
        if self.rule_class.get_name() == "config_setting"
            && self.package_metadata.config_setting_visibility_policy
                == Some(ConfigSettingVisibilityPolicy::DefaultPublic)
        {
            return RuleVisibility::Public; // Default: //visibility:public.
        }

        // Java: com/google/devtools/build/lib/packages/Target.java:166 Target.getDefaultVisibility
        if self.declaring_macro.is_some() {
            RuleVisibility::Private
        } else {
            self.package_declarations
                .get_package_args()
                .default_visibility()
                .cloned()
                .expect("NullPointerException: default visibility")
        }
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:713 Rule.getVisibilityDependencyLabels
    pub fn get_visibility_dependency_labels(&self) -> Vec<Label> {
        let Some(raw_labels) = self.get_raw_visibility_labels() else {
            return self.get_default_visibility().get_dependency_labels();
        };
        if let Some(constant_visibility) = RuleVisibility::parse_if_constant(&raw_labels) {
            return constant_visibility.get_dependency_labels();
        }
        // Filter out labels like :__pkg__ and :__subpackages__.
        raw_labels
            .into_iter()
            .filter(|label| PackageSpecification::from_label(label).is_none())
            .collect()
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:733 Rule.getLicense
    pub fn get_license(&self) -> License {
        // New style licenses defined by Starlark rules don't
        // have old-style licenses. This is hardcoding the representation
        // of new-style rules, but it's in the old-style licensing code path
        // and will ultimately be removed.
        if self.rule_class.is_package_metadata_rule() {
            License::no_license()
        } else if self.is_attr_defined("licenses", Type::License)
            && RuleOrMacroInstance::is_attribute_value_explicitly_specified(self, "licenses")
        {
            match NonconfigurableAttributeMapper::of(self).get("licenses", Type::License) {
                Some(AttrValue::License(license)) => license,
                other => panic!("ClassCastException: licenses value {other:?}"),
            }
        } else if self.rule_class.ignore_licenses() {
            License::no_license()
        } else {
            self.package_declarations
                .get_package_args()
                .license()
                .cloned()
                .expect("NullPointerException: package license")
        }
    }

    /// The tags of the rule: the `tags` attribute plus Boolean-attribute tags.
    // Java: com/google/devtools/build/lib/packages/Rule.java:752 Rule.getRuleTags
    pub fn get_rule_tags(&self) -> IndexSet<String> {
        let mut rule_tags = IndexSet::new();
        for attribute in self.rule_class.get_attribute_provider().get_attributes() {
            if attribute.is_taggable() {
                let attr_type = attribute.get_type();
                let name = attribute.get_name();
                // This enforces the expectation that taggable attributes are non-configurable.
                let value = NonconfigurableAttributeMapper::of(self).get(name, attr_type);
                let tags = attr_type.to_tag_set(value.as_ref(), name);
                rule_tags.extend(tags);
            }
        }
        rule_tags
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:768 Rule.getOnlyTagsAttribute
    pub fn get_only_tags_attribute(&self) -> Vec<String> {
        let tags_attribute = self
            .rule_class
            .get_attribute_provider()
            .get_attribute_by_name("tags");
        let attr_type = tags_attribute.get_type();
        let name = tags_attribute.get_name();
        // This enforces the expectation that taggable attributes are non-configurable.
        let value = NonconfigurableAttributeMapper::of(self).get(name, attr_type);
        attr_type
            .to_tag_set(value.as_ref(), name)
            .into_iter()
            .collect()
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:789 Rule.getDeprecationWarning
    pub fn get_deprecation_warning(&self) -> Option<String> {
        NonconfigurableAttributeMapper::attribute_or_null(self, "deprecation", Type::String)
            .and_then(|v| v.as_str().map(str::to_owned))
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:794 Rule.isTestOnly
    pub fn is_test_only(&self) -> bool {
        match NonconfigurableAttributeMapper::attribute_or_null(self, "testonly", Type::Boolean) {
            None => false,
            Some(value) => value.as_bool().expect("ClassCastException: testonly"),
        }
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:803 Rule.satisfies
    pub fn satisfies(&self, required: &RequiredProviders) -> bool {
        required.is_satisfied_by(self.rule_class.get_advertised_providers())
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:808 Rule.getTestTimeout
    pub fn get_test_timeout(&self) -> Option<TestTimeout> {
        // Java: com/google/devtools/build/lib/packages/TestTimeout.java:139 TestTimeout.getTestTimeout
        let attr =
            NonconfigurableAttributeMapper::attribute_or_null(self, "timeout", Type::String)?;
        // The target is not a test when the attribute is missing.
        let attr = attr.as_str()?;
        TestTimeout::get_test_timeout(attr)
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:813 Rule.isForDependencyResolution
    pub fn is_for_dependency_resolution(&self) -> bool {
        self.rule_class.is_dependency_resolution_rule()
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:818 Rule.getAdvertisedProviders
    pub fn get_advertised_providers(&self) -> &AdvertisedProviderSet {
        self.rule_class.get_advertised_providers()
    }

    /// The labels of aspect-implicit dependencies (a superset).
    // Java: com/google/devtools/build/lib/packages/Rule.java:826 Rule.getAspectLabelsSuperset
    pub fn get_aspect_labels_superset(&self, predicate: &DependencyFilter) -> Vec<Label> {
        if !self.has_aspects() {
            return Vec::new();
        }
        // Java: a LinkedHashMultimap<Attribute, Label>: (attribute, label) pairs deduplicated,
        // values() in insertion order.
        let mut labels: Vec<(Attribute, Label)> = Vec::new();
        for attribute in self.get_attributes() {
            for candidate_class in attribute.get_aspects(self) {
                let mut added: Vec<(Attribute, Label)> = Vec::new();
                AspectDefinition::add_all_attributes_of_aspect(
                    &mut added,
                    &candidate_class,
                    predicate,
                );
                for entry in added {
                    if !labels.contains(&entry) {
                        labels.push(entry);
                    }
                }
            }
        }
        labels.into_iter().map(|(_, label)| label).collect()
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:852 Rule.useToolchainResolution
    pub fn use_toolchain_resolution(&self) -> bool {
        self.rule_class.use_toolchain_resolution(self)
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:856 Rule.isExecutable
    pub fn is_executable(&self) -> bool {
        if self
            .rule_class
            .get_attribute_provider()
            .has_attr(IS_EXECUTABLE_ATTRIBUTE_NAME, Type::Boolean)
        {
            return NonconfigurableAttributeMapper::of(self)
                .get(IS_EXECUTABLE_ATTRIBUTE_NAME, Type::Boolean)
                .and_then(|v| v.as_bool())
                .expect("NullPointerException: $is_executable");
        }
        false
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:866 Rule.getRepository
    pub fn get_repository(&self) -> &RepositoryName {
        self.container.label.get_repository()
    }

    /// Java `Rule.targetKindSuffix()`: `" rule"`.
    // Java: com/google/devtools/build/lib/packages/Rule.java:871 Rule.targetKindSuffix
    pub fn target_kind_suffix() -> &'static str {
        " rule"
    }
}

impl fmt::Display for Rule {
    // Java: com/google/devtools/build/lib/packages/Rule.java:665 Rule.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} rule {}", self.get_rule_class(), self.get_label())
    }
}

impl RuleOrMacroInstance for Rule {
    fn attribute_container(&self) -> &AttributeContainer {
        &self.container
    }

    fn attribute_container_mut(&mut self) -> &mut AttributeContainer {
        &mut self.container
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:778 Rule.getAttributeProvider
    fn get_attribute_provider(&self) -> &AttributeProvider {
        self.rule_class.get_attribute_provider()
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:408 Rule.isRuleInstance
    fn is_rule_instance(&self) -> bool {
        true
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:413 Rule.isRuleCreatedInMacro
    fn is_rule_created_in_macro(&self) -> bool {
        // TODO(bazel-team): do we really need the `hasStringAttribute(GENERATOR_NAME)` check?
        self.interior_call_stack.is_some() || self.has_string_attribute(GENERATOR_NAME)
    }

    fn get_default_visibility(&self) -> RuleVisibility {
        Rule::get_default_visibility(self)
    }

    fn get_package_metadata(&self) -> &Arc<PackageMetadata> {
        &self.package_metadata
    }

    fn get_package_declarations(&self) -> &Arc<PackageDeclarations> {
        &self.package_declarations
    }

    fn get_declaring_macro(&self) -> Option<&Arc<MacroInstance>> {
        self.declaring_macro.as_ref()
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:654 Rule.reportError
    fn report_error(&self, message: &str, event_handler: &dyn EventHandler) {
        event_handler.handle(&Package::error(
            Some(&self.location),
            message,
            PackageLoadingCode::StarlarkEvalError,
        ));
        self.package_declarations.set_contains_errors(true);
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:374 Rule.getAttrWithIndex
    fn get_attr_with_index(&self, attr_index: usize) -> Option<AttrValue> {
        if let Some(value) = self.container.get_attr_if_stored(attr_index) {
            return Some(value.clone());
        }
        let attr = self
            .rule_class
            .get_attribute_provider()
            .get_attribute(attr_index);
        if attr.has_computed_default() {
            // Frozen rules don't store computed defaults, so get it from the attribute. Mutable
            // rules do store computed defaults if they've been populated. If no value is stored
            // for a mutable rule, return null here since resolving the default could trigger
            // reads of other attributes which have not yet been populated. Note that in this
            // situation returning null does not result in a correctness issue, since the value
            // for the attribute is actually a function to compute the value.
            return if self.is_frozen() {
                attr.get_default_value(Some(self as &dyn RuleOrMacroInstance))
            } else {
                None
            };
        }
        if attr.is_materializing() {
            blaze_util::check_state!(self.is_frozen(), "Mutable rule missing LateBoundDefault");
            return Some(AttrValue::Materializer(attr.get_materializer().clone()));
        }
        if attr.is_late_bound() {
            // Frozen rules don't store late bound defaults.
            blaze_util::check_state!(self.is_frozen(), "Mutable rule missing LateBoundDefault");
            return attr
                .get_late_bound_default()
                .cloned()
                .map(AttrValue::LateBound);
        }
        match attr.get_name() {
            GENERATOR_FUNCTION => Some(AttrValue::String(match &self.interior_call_stack {
                Some(stack) => stack.function_name().to_owned(),
                None => String::new(),
            })),
            GENERATOR_LOCATION => {
                if self.interior_call_stack.is_some() {
                    // Java returns null when the package name is not found in the location.
                    self.get_relative_location().map(AttrValue::String)
                } else {
                    Some(AttrValue::String(String::new()))
                }
            }
            GENERATOR_NAME => {
                let len = self.container.generator_name_prefix_length;
                Some(AttrValue::String(if len > 0 {
                    self.get_name()[..len as usize].to_owned()
                } else {
                    String::new()
                }))
            }
            _ => attr.get_default_value(Some(self as &dyn RuleOrMacroInstance)),
        }
    }
}

impl TargetData for Rule {
    // Java: com/google/devtools/build/lib/packages/Rule.java:152 Rule.getTargetKind
    fn get_target_kind(&self) -> String {
        self.rule_class.get_target_kind().to_owned()
    }

    fn get_location(&self) -> Location {
        self.location.clone()
    }

    fn get_rule_class(&self) -> String {
        Rule::get_rule_class(self).to_owned()
    }

    fn get_rule_tags(&self) -> IndexSet<String> {
        Rule::get_rule_tags(self)
    }

    fn get_label(&self) -> &Label {
        &self.container.label
    }

    // Java: com/google/devtools/build/lib/packages/Rule.java:783 Rule.isRule
    fn is_rule(&self) -> bool {
        true
    }

    fn get_deprecation_warning(&self) -> Option<String> {
        Rule::get_deprecation_warning(self)
    }

    fn is_test_only(&self) -> bool {
        Rule::is_test_only(self)
    }

    fn satisfies(&self, required: &RequiredProviders) -> bool {
        Rule::satisfies(self, required)
    }

    fn get_test_timeout(&self) -> Option<TestTimeout> {
        Rule::get_test_timeout(self)
    }

    fn is_for_dependency_resolution(&self) -> bool {
        Rule::is_for_dependency_resolution(self)
    }

    fn get_advertised_providers(&self) -> AdvertisedProviderSet {
        Rule::get_advertised_providers(self).clone()
    }
}
