//! Port of `packages/MacroClass.java`: a symbolic macro defined by `macro()`.

use std::sync::Arc;
use std::sync::LazyLock;

use blaze_cmdline::EvalException;
use blaze_cmdline::Label;
use blaze_events::EventHandler;
use blaze_starlark::StarlarkSemantics;
use blaze_starlark::error::StackEntry;
use starlark::values::Value;

use crate::attribute::Attribute;
use crate::attribute::AttributeProvider;
use crate::attribute::attr;
use crate::exceptions::PackageLoadingCode;
use crate::macros::MacroInstance;
use crate::package::Package;
use crate::package::TargetDefinitionContext;
use crate::rule_class::StarlarkCallable;
use crate::types::Type;

/// Names that users may not pass as keys of the `attrs` dict when calling `macro()`.
// Java: com/google/devtools/build/lib/packages/MacroClass.java:68 MacroClass.RESERVED_MACRO_ATTR_NAMES
pub const RESERVED_MACRO_ATTR_NAMES: [&str; 2] = ["name", "visibility"];

/// "visibility" attribute present on all symbolic macros (like the rule one, without the exec
/// transitions).
// Java: com/google/devtools/build/lib/packages/MacroClass.java:76 MacroClass.VISIBILITY_ATTRIBUTE
pub static VISIBILITY_ATTRIBUTE: LazyLock<Attribute> = LazyLock::new(|| {
    attr("visibility", Type::NodepLabelList)
        .order_independent()
        .nonconfigurable("special attribute integrated more deeply into Bazel's core logic")
        .build()
});

/// A symbolic macro class (Java `MacroClass`). Immutable; shared as `Arc<MacroClass>`.
// Java: com/google/devtools/build/lib/packages/MacroClass.java:61 MacroClass
#[derive(Debug)]
pub struct MacroClass {
    name: String,
    defining_bzl_label: Label,
    implementation: StarlarkCallable,
    /// Implicit attributes are stored under their given name (`_foo`), not mangled (`$foo`).
    is_finalizer: bool,
    attribute_provider: AttributeProvider,
}

impl MacroClass {
    // Java: com/google/devtools/build/lib/packages/MacroClass.java:91 MacroClass.<init>
    fn new(
        name: String,
        defining_bzl_label: Label,
        implementation: StarlarkCallable,
        attributes: Vec<Attribute>,
        is_finalizer: bool,
    ) -> Self {
        let attribute_index = attributes
            .iter()
            .enumerate()
            .map(|(i, a)| (a.get_name().to_owned(), i))
            .collect();
        let attribute_provider =
            AttributeProvider::new(attributes, attribute_index, None, name.clone(), false);
        MacroClass {
            name,
            defining_bzl_label,
            implementation,
            is_finalizer,
            attribute_provider,
        }
    }

    // Java: com/google/devtools/build/lib/packages/MacroClass.java:116 MacroClass.getName
    pub fn get_name(&self) -> &str {
        &self.name
    }

    // Java: com/google/devtools/build/lib/packages/MacroClass.java:121 MacroClass.getDefiningBzlLabel
    pub fn get_defining_bzl_label(&self) -> &Label {
        &self.defining_bzl_label
    }

    // Java: com/google/devtools/build/lib/packages/MacroClass.java:125 MacroClass.getImplementation
    pub fn get_implementation(&self) -> StarlarkCallable {
        self.implementation
    }

    // Java: com/google/devtools/build/lib/packages/MacroClass.java:129 MacroClass.getAttributeProvider
    pub fn get_attribute_provider(&self) -> &AttributeProvider {
        &self.attribute_provider
    }

    // Java: com/google/devtools/build/lib/packages/MacroClass.java:137 MacroClass.isFinalizer
    pub fn is_finalizer(&self) -> bool {
        self.is_finalizer
    }

    /// Instantiates the macro from the call's keyword arguments and adds it to the package.
    // Java: com/google/devtools/build/lib/packages/MacroClass.java:371 MacroClass.instantiateAndAddMacro
    pub fn instantiate_and_add_macro<'v>(
        self: &Arc<Self>,
        target_definition_context: &mut TargetDefinitionContext,
        kwargs: &[(String, Value<'v>)],
        parent_thread_call_stack: &[StackEntry],
    ) -> Result<Arc<MacroInstance>, EvalException> {
        let _ = (target_definition_context, kwargs, parent_thread_call_stack);
        blaze_util::unported!("MacroClass.instantiateAndAddMacro")
    }

    /// Runs a symbolic macro's implementation function in a new Starlark thread, adding its
    /// targets to the package under construction.
    // PORT: Java's StarlarkSemantics is blaze_starlark's.
    // Java: com/google/devtools/build/lib/packages/MacroClass.java:390 MacroClass.executeMacroImplementation
    pub fn execute_macro_implementation(
        macro_: &Arc<MacroInstance>,
        target_definition_context: &mut TargetDefinitionContext,
        semantics: &StarlarkSemantics,
    ) -> Result<(), EvalException> {
        // Ensure we're not expanding a (possibly indirect) recursive macro. This is morally
        // analogous to StarlarkThread#isRecursiveCall, except in this context, recursion is
        // through the chain of macro instantiations, which may or may not actually be
        // concurrently executing on the stack depending on whether the evaluation is eager or
        // deferred.
        if let Some(recursion_msg) = MacroClass::get_recursion_error_message(macro_) {
            target_definition_context
                .get_local_event_handler()
                .handle(&Package::error(
                    /* location= */ None,
                    &recursion_msg,
                    PackageLoadingCode::StarlarkEvalError,
                ));
            target_definition_context.set_contains_errors();
            // Don't try to evaluate this macro again.
            target_definition_context.mark_macro_complete(macro_);
            return Ok(());
        }
        let _ = semantics;
        // PORT: needs a Starlark thread for the macro (blaze_starlark::eval::call_frozen with
        // the builder moved into its ThreadContext payload) and Attribute.valueToStarlark /
        // SelectorList.wrapSingleValue for the kwargs.
        blaze_util::unported!("MacroClass.executeMacroImplementation")
    }

    /// Java `shouldForceDefaultToNone`: non-direct defaults and legacy license attributes.
    // Java: com/google/devtools/build/lib/packages/MacroClass.java:351 MacroClass.shouldForceDefaultToNone
    #[allow(dead_code)] // PORT: used by executeMacroImplementation once ported.
    fn should_force_default_to_none(attr: &Attribute) -> bool {
        attr.has_computed_default()
            || attr.is_late_bound()
            || attr.is_materializing()
            || attr.get_type() == Type::License
    }

    /// The error for a recursive macro instantiation, or `None`.
    // Java: com/google/devtools/build/lib/packages/MacroClass.java:500 MacroClass.getRecursionErrorMessage
    fn get_recursion_error_message(macro_: &Arc<MacroInstance>) -> Option<String> {
        let mut ancestor = macro_.get_parent();
        let mut found_recursion = false;
        let mut on_immediate_parent = true;
        while let Some(a) = ancestor {
            // TODO: #19922 - We're checking based on object identity here.
            if Arc::ptr_eq(a.get_macro_class(), macro_.get_macro_class()) {
                found_recursion = true;
                break;
            }
            ancestor = a.get_parent();
            on_immediate_parent = false;
        }
        if !found_recursion {
            return None;
        }
        let ancestor = ancestor.unwrap();
        let mut msg = format!(
            "macro '{}' is {} recursive call of '{}'. Macro instantiation traceback (most recent \
             call last):",
            macro_.get_name(),
            if on_immediate_parent {
                "a direct"
            } else {
                "an indirect"
            },
            ancestor.get_name()
        );
        // Materialize the stack, since we want to output it in reverse order (outermost first).
        let mut all_ancestors: Vec<&Arc<MacroInstance>> = Vec::new();
        let mut a = Some(macro_);
        while let Some(item) = a {
            all_ancestors.push(item);
            a = item.get_parent();
        }
        for item in all_ancestors.into_iter().rev() {
            let pkg = item
                .get_package_metadata()
                .package_identifier
                .get_canonical_form();
            let type_ = format!(
                "{}%{}",
                item.get_macro_class()
                    .get_defining_bzl_label()
                    .get_canonical_form(),
                item.get_macro_class().get_name()
            );
            msg.push_str(&format!(
                "\n\tPackage {}, macro '{}' of type {}",
                pkg,
                item.get_name(),
                type_
            ));
        }
        Some(msg)
    }
}

/// Java `MacroClass.Builder`.
// Java: com/google/devtools/build/lib/packages/MacroClass.java:142 MacroClass.Builder
#[derive(Debug)]
pub struct MacroClassBuilder {
    name: Option<String>,
    defining_bzl_label: Option<Label>,
    implementation: StarlarkCallable,
    attributes: Vec<Attribute>,
    is_finalizer: bool,
}

impl MacroClassBuilder {
    /// A builder with the `name` and `visibility` attributes.
    // Java: com/google/devtools/build/lib/packages/MacroClass.java:149 MacroClass.Builder.<init>
    pub fn new(implementation: StarlarkCallable) -> Self {
        let mut builder = MacroClassBuilder {
            name: None,
            defining_bzl_label: None,
            implementation,
            attributes: Vec::new(),
            is_finalizer: false,
        };
        builder.add_attribute(crate::rule_class::NAME_ATTRIBUTE.clone());
        builder.add_attribute(VISIBILITY_ATTRIBUTE.clone());
        builder
    }

    // Java: com/google/devtools/build/lib/packages/MacroClass.java:157 MacroClass.Builder.setName
    pub fn set_name(&mut self, name: String) -> &mut Self {
        self.name = Some(name);
        self
    }

    // Java: com/google/devtools/build/lib/packages/MacroClass.java:163 MacroClass.Builder.setDefiningBzlLabel
    pub fn set_defining_bzl_label(&mut self, label: Label) -> &mut Self {
        self.defining_bzl_label = Some(label);
        self
    }

    // Java: com/google/devtools/build/lib/packages/MacroClass.java:169 MacroClass.Builder.addAttribute
    pub fn add_attribute(&mut self, attribute: Attribute) -> &mut Self {
        self.attributes.push(attribute);
        self
    }

    // Java: com/google/devtools/build/lib/packages/MacroClass.java:175 MacroClass.Builder.setIsFinalizer
    pub fn set_is_finalizer(&mut self) -> &mut Self {
        self.is_finalizer = true;
        self
    }

    // Java: com/google/devtools/build/lib/packages/MacroClass.java:180 MacroClass.Builder.build
    pub fn build(&self) -> MacroClass {
        let name = self
            .name
            .clone()
            .expect("NullPointerException: MacroClass.Builder.name");
        let defining_bzl_label = self
            .defining_bzl_label
            .clone()
            .expect("NullPointerException: MacroClass.Builder.definingBzlLabel");
        MacroClass::new(
            name,
            defining_bzl_label,
            self.implementation,
            self.attributes.clone(),
            self.is_finalizer,
        )
    }
}
