//! Port of `packages/MacroInstance.java`: an instance of a symbolic macro in a package.

use std::fmt;
use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_cmdline::PackageIdentifier;
use blaze_events::EventHandler;
use blaze_starlark::Location;
use blaze_starlark::error::StackEntry;

use crate::attribute::AggregatingAttributeMapper;
use crate::attribute::Attribute;
use crate::attribute::AttributeProvider;
use crate::macros::MacroClass;
use crate::package::CallStackNode;
use crate::package::PackageDeclarations;
use crate::package::PackageMetadata;
use crate::package::RuleVisibility;
use crate::target::AttributeContainer;
use crate::target::RuleOrMacroInstance;
use crate::types::AttrValue;
use crate::types::LabelClass;
use crate::types::Type;

/// A symbolic macro instance (Java `MacroInstance`). Mutable while being populated, then
/// shared as `Arc<MacroInstance>`.
// Java: com/google/devtools/build/lib/packages/MacroInstance.java:46 MacroInstance
#[derive(Debug)]
pub struct MacroInstance {
    container: AttributeContainer,
    package_metadata: Arc<PackageMetadata>,
    package_declarations: Arc<PackageDeclarations>,
    parent: Option<Arc<MacroInstance>>,
    generator_name: Option<String>,
    build_file_location: Location,
    parent_call_stack: Option<Arc<CallStackNode>>,
    macro_class: Arc<MacroClass>,
    same_name_depth: i32,
}

impl MacroInstance {
    // Java: com/google/devtools/build/lib/packages/MacroInstance.java:85 MacroInstance.<init>
    pub fn new(
        package_metadata: Arc<PackageMetadata>,
        package_declarations: Arc<PackageDeclarations>,
        parent: Option<Arc<MacroInstance>>,
        generator_name: Option<String>,
        build_file_location: Location,
        parent_call_stack: Option<Arc<CallStackNode>>,
        macro_class: Arc<MacroClass>,
        label: Label,
        same_name_depth: i32,
    ) -> Self {
        blaze_util::check_argument!(same_name_depth > 0);
        let attr_count = macro_class.get_attribute_provider().get_attribute_count();
        MacroInstance {
            container: AttributeContainer::new(label, attr_count),
            package_metadata,
            package_declarations,
            parent,
            generator_name,
            build_file_location,
            parent_call_stack,
            macro_class,
            same_name_depth,
        }
    }

    pub fn get_label(&self) -> &Label {
        &self.container.label
    }

    pub fn get_name(&self) -> &str {
        self.container.label.get_name()
    }

    // Java: com/google/devtools/build/lib/packages/MacroInstance.java:140 MacroInstance.getPackageMetadata
    pub fn get_package_metadata(&self) -> &Arc<PackageMetadata> {
        &self.package_metadata
    }

    // Java: com/google/devtools/build/lib/packages/MacroInstance.java:158 MacroInstance.getParent
    pub fn get_parent(&self) -> Option<&Arc<MacroInstance>> {
        self.parent.as_ref()
    }

    // Java: com/google/devtools/build/lib/packages/MacroInstance.java:175 MacroInstance.getBuildFileLocation
    pub fn get_build_file_location(&self) -> &Location {
        &self.build_file_location
    }

    // Java: com/google/devtools/build/lib/packages/MacroInstance.java:191 MacroInstance.getGeneratorName
    pub fn get_generator_name(&self) -> Option<&str> {
        self.generator_name.as_deref()
    }

    // Java: com/google/devtools/build/lib/packages/MacroInstance.java:202 MacroInstance.getParentCallStack
    pub fn get_parent_call_stack(&self) -> Option<&Arc<CallStackNode>> {
        self.parent_call_stack.as_ref()
    }

    // Java: com/google/devtools/build/lib/packages/MacroInstance.java:213 MacroInstance.reconstructParentCallStack
    pub fn reconstruct_parent_call_stack(&self) -> Vec<StackEntry> {
        let mut stack = Vec::new();
        if self.parent.is_none() {
            stack.push(StackEntry {
                name: crate::package::call_stack::TOP_LEVEL.to_owned(),
                location: Some(self.build_file_location.clone()),
                source_line: None,
            });
        }
        let mut node = self.parent_call_stack.as_ref();
        while let Some(n) = node {
            stack.push(n.to_call_stack_entry());
            node = n.next();
        }
        stack
    }

    // Java: com/google/devtools/build/lib/packages/MacroInstance.java:225 MacroInstance.getMacroClass
    pub fn get_macro_class(&self) -> &Arc<MacroClass> {
        &self.macro_class
    }

    // Java: com/google/devtools/build/lib/packages/MacroInstance.java:236 MacroInstance.getSameNameDepth
    pub fn get_same_name_depth(&self) -> i32 {
        self.same_name_depth
    }

    /// The id of the macro within its package: `<name>:<same name depth>`.
    // Java: com/google/devtools/build/lib/packages/MacroInstance.java:259 MacroInstance.getId
    pub fn get_id(&self) -> String {
        format!("{}:{}", self.get_name(), self.same_name_depth)
    }

    // Java: com/google/devtools/build/lib/packages/MacroInstance.java:281 MacroInstance.getActualVisibility
    pub fn get_actual_visibility(&self) -> Vec<Label> {
        match self.get_attr("visibility") {
            Some(AttrValue::LabelList(visibility)) => visibility,
            None => panic!("NullPointerException: visibility"),
            Some(other) => panic!("ClassCastException: visibility value {other:?}"),
        }
    }

    // Java: com/google/devtools/build/lib/packages/MacroInstance.java:294 MacroInstance.getDefinitionPackage
    pub fn get_definition_package(&self) -> &PackageIdentifier {
        self.macro_class
            .get_defining_bzl_label()
            .get_package_identifier()
    }

    /// Visits the labels of explicitly set attributes (selects included).
    // Java: com/google/devtools/build/lib/packages/MacroInstance.java:306 MacroInstance.visitExplicitAttributeLabels
    pub fn visit_explicit_attribute_labels(&self, consumer: &mut dyn FnMut(&Label)) {
        for attribute in self.macro_class.get_attribute_provider().get_attributes() {
            let name = attribute.get_name();
            let type_ = attribute.get_type();
            if name.starts_with('_') {
                continue;
            }
            if type_.get_label_class() != LabelClass::Dependency {
                continue;
            }
            let value = self.get_attr_typed(name, type_);
            Self::visit_attribute_labels(value.as_ref(), type_, attribute, consumer);
        }
    }

    // `value` is either a T or SelectorList<T>.
    // Java: com/google/devtools/build/lib/packages/MacroInstance.java:325 MacroInstance.visitAttributeLabels
    fn visit_attribute_labels(
        value: Option<&AttrValue>,
        type_: Type,
        attribute: &Attribute,
        consumer: &mut dyn FnMut(&Label),
    ) {
        let Some(value) = value else {
            return;
        };
        let mut visitor = |label: &Label, _unused_attribute: Option<&Attribute>| {
            consumer(label);
        };
        if let AttrValue::SelectorList(selector_list) = value {
            AggregatingAttributeMapper::visit_labels_in_select(
                selector_list,
                attribute,
                type_,
                &mut visitor,
                /* rule= */
                None, // safe because late-bound defaults aren't a thing for macros
                /* include_keys= */ false,
                /* include_values= */ true,
            );
        } else {
            let cast_value = type_.cast(value);
            type_.visit_labels(&mut visitor, cast_value, Some(attribute));
        }
    }

    // Java: com/google/devtools/build/lib/packages/MacroInstance.java:374 MacroInstance.getShortDescription
    pub fn get_short_description(&self) -> String {
        format!(
            "{}macro {}:{} defined by {}%{}",
            if self.macro_class.is_finalizer() {
                "finalizer "
            } else {
                ""
            },
            self.package_metadata
                .package_identifier
                .get_canonical_form(),
            self.get_name(),
            self.macro_class
                .get_defining_bzl_label()
                .get_canonical_form(),
            self.macro_class.get_name()
        )
    }
}

impl fmt::Display for MacroInstance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.get_short_description())
    }
}

impl RuleOrMacroInstance for MacroInstance {
    fn attribute_container(&self) -> &AttributeContainer {
        &self.container
    }

    fn attribute_container_mut(&mut self) -> &mut AttributeContainer {
        &mut self.container
    }

    // Java: com/google/devtools/build/lib/packages/MacroInstance.java:354 MacroInstance.getAttributeProvider
    fn get_attribute_provider(&self) -> &AttributeProvider {
        self.macro_class.get_attribute_provider()
    }

    // Java: com/google/devtools/build/lib/packages/MacroInstance.java:364 MacroInstance.isRuleInstance
    fn is_rule_instance(&self) -> bool {
        false
    }

    // Java: com/google/devtools/build/lib/packages/MacroInstance.java:369 MacroInstance.isRuleCreatedInMacro
    fn is_rule_created_in_macro(&self) -> bool {
        false
    }

    // Java: com/google/devtools/build/lib/packages/MacroInstance.java:264 MacroInstance.getDefaultVisibility
    fn get_default_visibility(&self) -> RuleVisibility {
        RuleVisibility::parse_unchecked(&[Label::create_unvalidated(
            self.macro_class
                .get_defining_bzl_label()
                .get_package_identifier()
                .clone(),
            "__pkg__",
        )])
    }

    fn get_package_metadata(&self) -> &Arc<PackageMetadata> {
        &self.package_metadata
    }

    // Java: com/google/devtools/build/lib/packages/MacroInstance.java:145 MacroInstance.getPackageDeclarations
    fn get_package_declarations(&self) -> &Arc<PackageDeclarations> {
        &self.package_declarations
    }

    // Java: com/google/devtools/build/lib/packages/MacroInstance.java:167 MacroInstance.getDeclaringMacro
    fn get_declaring_macro(&self) -> Option<&Arc<MacroInstance>> {
        self.parent.as_ref()
    }

    // Java: com/google/devtools/build/lib/packages/MacroInstance.java:359 MacroInstance.reportError
    fn report_error(&self, message: &str, event_handler: &dyn EventHandler) {
        event_handler.handle(&blaze_events::Event::error(message));
    }
}
