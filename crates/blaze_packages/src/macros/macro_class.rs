//! Port of `packages/MacroClass.java`: a symbolic macro defined by `macro()`.

use std::sync::Arc;
use std::sync::LazyLock;

use blaze_cmdline::EvalException;
use blaze_cmdline::Label;
use blaze_cmdline::LabelRepoContextImpl;
use blaze_events::EventHandler;
use blaze_starlark::EvalError;
use blaze_starlark::Phase;
use blaze_starlark::StarlarkSemantics;
use blaze_starlark::ThreadContext;
use blaze_starlark::error::EvalErrorKind;
use blaze_starlark::error::StackEntry;
use blaze_starlark::eval::EvalOptions;
use blaze_starlark::label::LabelValue;
use indexmap::IndexMap;
use starlark::values::Heap;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::list::ListRef;

use crate::attribute::Attribute;
use crate::attribute::AttributeProvider;
use crate::attribute::attr;
use crate::attribute::string_util::did_you_mean;
use crate::build::package_factory::DebugPrintHandler;
use crate::exceptions::PackageLoadingCode;
use crate::macros::MacroInstance;
use crate::package::MacroFrame;
use crate::package::Package;
use crate::package::RuleVisibility;
use crate::package::TargetDefinitionContext;
use crate::package::rule_factory::BuildLangTypedAttributeValuesMap;
use crate::rule_class::StarlarkCallable;
use crate::target::RuleOrMacroInstance;
use crate::types::AttrValue;
use crate::types::Type;
use crate::types::build_type::copy_and_lift_starlark_value;
use crate::types::select::SelectorList;

/// The labels of a Starlark list of `Label`s (Java's unchecked cast to `List<Label>` of a value
/// `copyAndLiftStarlarkValue` returned for a label-list attribute).
fn starlark_label_list(v: Value) -> Vec<Label> {
    match ListRef::from_value(v) {
        Some(list) => list
            .iter()
            .map(|e| match e.downcast_ref::<LabelValue>() {
                Some(l) => l.0.clone(),
                None => panic!("ClassCastException: {} is not a Label", e.get_type()),
            })
            .collect(),
        None => panic!("ClassCastException: {} is not a List", v.get_type()),
    }
}

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

    /// Constructs and returns a new [`MacroInstance`] associated with this `MacroClass` (see
    /// [`MacroClass::instantiate_and_add_macro`]).
    // TODO(#19922): Consider reporting multiple events instead of failing on the first one. See
    // analogous implementation in RuleClass#populateDefinedRuleAttributeValues.
    // Java: com/google/devtools/build/lib/packages/MacroClass.java:199 MacroClass.instantiateMacro
    fn instantiate_macro<'v>(
        self: &Arc<Self>,
        target_definition_context: &TargetDefinitionContext,
        kwargs: &IndexMap<String, Value<'v>>,
        parent_thread_call_stack: &[StackEntry],
        heap: Heap<'v>,
    ) -> Result<MacroInstance, EvalException> {
        // A word on edge cases:
        //   - If an attr is implicit but does not have a default specified, its value is just the
        //     default value for its attr type (e.g. `[]` for `attr.label_list()`).
        //   - If an attr is implicit but also mandatory, it's impossible to instantiate it without
        //     error.
        //   - If an attr is mandatory but also has a default, the default is meaningless.
        // These behaviors align with rule attributes.

        // Java's Dict.Builder.buildImmutable keeps the last value of a repeated key at its first
        // position: IndexMap::insert.
        let mut attr_values: IndexMap<String, Value<'v>> = IndexMap::new();

        // For each given attr value, validate that the attr exists and can be set.
        for (attr_name, value) in kwargs {
            let value = *value;

            // Check for unknown attr.
            if self
                .attribute_provider
                .get_attribute_index(attr_name)
                .is_none()
            {
                let documented: Vec<&str> = self
                    .attribute_provider
                    .get_attributes()
                    .iter()
                    .filter(|a| a.is_documented())
                    .map(|a| a.get_name())
                    .collect();
                return Err(EvalException::new(format!(
                    "no such attribute '{}' in '{}' macro{}",
                    attr_name,
                    self.name,
                    did_you_mean(attr_name, &documented)
                )));
            }

            // Setting an attr to None is the same as omitting it (except that it's still an error
            // to set an unknown attr to None). If the attr is optional, skip adding it to the map
            // now but put it in below when we realize it's missing.
            if value.is_none() {
                continue;
            }

            // Can't set implicit default.
            // (We don't check Attribute#isImplicit() because that assumes "_" -> "$" prefix
            // mangling.)
            // TODO: #19922 - The lack of "_" -> "$" mangling may impact the future feature of
            // inheriting attributes from rules. We could consider just doing the mangling for
            // macros too so they're consistent.
            if attr_name.starts_with('_') {
                return Err(EvalException::new(format!(
                    "cannot set value of implicit attribute '{attr_name}'"
                )));
            }

            attr_values.insert(attr_name.clone(), value);
        }

        // Special processing of the "visibility" attribute.
        // TODO(brandjon): When we add introspection of attributes of symbolic macros, we'll want
        // to distinguish between the different types of visibility a la Target#getRawVisibility /
        // #getVisibility / #getActualVisibility.
        let parent_macro_frame = target_definition_context.get_current_macro_frame();
        let raw_visibility = kwargs.get("visibility").copied();
        let parsed_visibility = match raw_visibility {
            None => None,
            Some(v) if v.is_none() => None,
            Some(raw_visibility) => {
                let lifted_visibility = copy_and_lift_starlark_value(
                    &self.name,
                    &VISIBILITY_ATTRIBUTE,
                    raw_visibility,
                    Some(target_definition_context.get_label_converter()),
                    heap,
                )
                .map_err(|e| EvalException::new(e.get_message()))?;
                Some(RuleVisibility::parse(&starlark_label_list(
                    lifted_visibility,
                ))?)
            }
        };
        let parsed_visibility = match parsed_visibility {
            Some(v) => v,
            // Visibility wasn't explicitly supplied. If we're not in another symbolic macro, use
            // the package's default visibility, otherwise use private visibility.
            None => match parent_macro_frame {
                None => target_definition_context
                    .get_partial_package_args()
                    .default_visibility()
                    .cloned()
                    .unwrap_or_else(|| {
                        panic!("NullPointerException: PackageArgs.defaultVisibility()")
                    }),
                Some(_) => RuleVisibility::Private,
            },
        };
        // Concatenate the visibility (as previously populated) with the instantiation site's
        // location.
        let instantiating_loc = match parent_macro_frame {
            None => target_definition_context.get_package_identifier().clone(),
            Some(frame) => frame.macro_instance.get_definition_package().clone(),
        };
        let actual_visibility = parsed_visibility.concat_with_package(&instantiating_loc);
        let declared: Vec<Value<'v>> = actual_visibility
            .get_declared_labels()
            .into_iter()
            .map(|l| heap.alloc(LabelValue(l)))
            .collect();
        // Java: Starlark.fromJava(list, Mutability.IMMUTABLE).
        attr_values.insert(
            "visibility".to_owned(),
            crate::types::type_::alloc_immutable_list(heap, declared),
        );

        // Normalize and validate all attr values. (E.g., convert strings to labels, promote
        // configurable attribute values to select()s, fail if bool was passed instead of label,
        // ensure values are immutable.)
        for attribute in self.attribute_provider.get_attributes() {
            if (attribute.is_public() && attribute.starlark_defined())
                || attribute.get_name() == "name"
            {
                if let Some(value) = kwargs.get(attribute.get_name()).copied() {
                    if value.is_none() {
                        // Don't promote None to select({"//conditions:default": None}).
                        continue;
                    }
                    let normalized_value =
                        // copyAndLiftStarlarkValue ensures immutability.
                        copy_and_lift_starlark_value(
                            &self.name,
                            attribute,
                            value,
                            Some(target_definition_context.get_label_converter()),
                            heap,
                        )
                        .map_err(|e| EvalException::new(e.get_message()))?;
                    // TODO(#19922): Validate that LABEL_LIST type attributes don't contain
                    // duplicates, to match the behavior of rules. This probably requires factoring
                    // out logic from AggregatingAttributeMapper.
                    attr_values.insert(attribute.get_name().to_owned(), normalized_value);
                }
            }
        }

        // Type and existence enforced by RuleClass.NAME_ATTRIBUTE.
        // Other mandatory attributes are enforced after the macro is created, but we need to
        // check for name now in order to find out the depth.
        let Some(name_value) = kwargs.get("name") else {
            return Err(EvalException::new(format!(
                "missing value for mandatory attribute 'name' in '{}' macro",
                self.name
            )));
        };
        let name = match name_value.unpack_str() {
            Some(name) => name.to_owned(),
            None => panic!(
                "ClassCastException: {} cannot be cast to class java.lang.String",
                name_value.get_type()
            ),
        };
        // Determine the id for this macro. If we're in another macro by the same name, increment
        // the number, otherwise use 1 for the number.
        let same_name_depth = match parent_macro_frame {
            Some(frame) if name == frame.macro_instance.get_name() => {
                frame.macro_instance.get_same_name_depth() + 1
            }
            _ => 1,
        };

        let attribute_values = BuildLangTypedAttributeValuesMap::new(attr_values);

        let mut macro_instance = target_definition_context
            .create_macro(self, &name, same_name_depth, parent_thread_call_stack)
            .map_err(|e| EvalException::new(e.get_message()))?;
        self.attribute_provider
            .populate_rule_attribute_values(
                &mut macro_instance,
                target_definition_context,
                &attribute_values,
                /* fail_on_unknown_attributes= */ true,
                /* is_starlark= */ true,
            )
            .map_err(|e| EvalException::new(e.0))?;
        Ok(macro_instance)
    }

    /// Constructs a new [`MacroInstance`] associated with this `MacroClass`, adds it to the
    /// package, and returns it.
    ///
    /// `kwargs` maps attribute names to their given Starlark values, as passed in a BUILD file
    /// (prior to type conversion, `select()` promotion, default value substitution, or even
    /// validation that the attribute exists). `parent_thread_call_stack` is the call stack of the
    /// Starlark thread in whose context the macro instance is being constructed; it is *not* the
    /// thread that will execute the macro's implementation function.
    // Java: com/google/devtools/build/lib/packages/MacroClass.java:371 MacroClass.instantiateAndAddMacro
    pub fn instantiate_and_add_macro<'v>(
        self: &Arc<Self>,
        target_definition_context: &mut TargetDefinitionContext,
        kwargs: &IndexMap<String, Value<'v>>,
        parent_thread_call_stack: &[StackEntry],
        heap: Heap<'v>,
    ) -> Result<Arc<MacroInstance>, EvalException> {
        // Java's catch of LabelSyntaxException | NameConflictException |
        // CannotPrecomputeDefaultsException wraps them in an EvalException with their message.
        let macro_instance = Arc::new(self.instantiate_macro(
            target_definition_context,
            kwargs,
            parent_thread_call_stack,
            heap,
        )?);
        target_definition_context
            .add_macro(macro_instance.clone())
            .map_err(|e| EvalException::new(e.get_message()))?;
        Ok(macro_instance)
    }

    /// Executes a symbolic macro's implementation function, in a new Starlark thread, mutating
    /// the given packageoid under construction.
    ///
    /// PORT: Java mutates the builder it is given; the builder is the payload of the new
    /// thread's context while the implementation runs, so it is passed by value and returned.
    /// The error is Java's `EvalException` with its call stack.
    // Java: com/google/devtools/build/lib/packages/MacroClass.java:390 MacroClass.executeMacroImplementation
    pub fn execute_macro_implementation(
        macro_: &Arc<MacroInstance>,
        mut target_definition_context: TargetDefinitionContext,
        semantics: &Arc<StarlarkSemantics>,
    ) -> (TargetDefinitionContext, Result<(), EvalError>) {
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
            if target_definition_context.is_package_builder() {
                target_definition_context.mark_macro_complete(macro_);
            }
            return (target_definition_context, Ok(()));
        }

        // Java: StarlarkThread.create(mu, semantics, macro.getShortDescription(),
        // SymbolGenerator.create(MacroInstance.UniqueId...)) with
        // thread.setPrintHandler(Event.makeDebugPrintHandler(localEventHandler)).
        // PORT: the thread context mirrors the BUILD thread's (file label and repository
        // mapping of the package), as `Label()` and identity tokens need them; Java's symbol
        // generator owner is the macro's unique id.
        let metadata = target_definition_context.get_metadata().clone();
        let events = Arc::new(DebugPrintHandler::new(
            target_definition_context
                .get_local_event_handler_arc()
                .clone(),
        ));
        let thread = ThreadContext::new(Phase::Loading, semantics.clone(), events)
            .with_file_label(metadata.build_file_label.clone())
            .with_label_repo_context(LabelRepoContextImpl::of(
                metadata.build_file_label.get_repository().clone(),
                metadata.repository_mapping.clone(),
            ))
            .with_main_repo_mapping(
                target_definition_context
                    .get_main_repository_mapping()
                    .clone(),
            );

        // TODO: #19922 - If we want to support creating analysis_test rules inside symbolic
        // macros, we'd need to call `thread.setThreadLocal(RuleDefinitionEnvironment.class,
        // ruleClassProvider)`. In that case we'll need to consider how to get access to the
        // ConfiguredRuleClassProvider. For instance, we could put it in the builder.

        let child_macro_frame = MacroFrame::new(macro_.clone());
        let parent_macro_frame =
            target_definition_context.set_current_macro_frame(Some(child_macro_frame));
        // Retrieve the values of the macro's attributes (converted to Starlark values below,
        // on the new thread's heap).
        let mut attr_values: Vec<(String, Option<AttrValue>, bool)> = Vec::new();
        for attr in macro_
            .get_macro_class()
            .get_attribute_provider()
            .get_attributes()
        {
            let mut attr_value = macro_.get_attr_typed(attr.get_name(), attr.get_type());
            let mut force_none = false;
            if attr_value.is_none() {
                attr_value = attr.get_default_value_unchecked().cloned();
                if attr_value.is_none() || MacroClass::should_force_default_to_none(attr) {
                    force_none = true;
                }
            }
            attr_values.push((
                attr.get_name().to_owned(),
                if force_none { None } else { attr_value },
                attr.is_configurable(),
            ));
        }

        // TODO: #19922 - Technically the embedded SymbolGenerator field should use a different
        // key than the one in the main BUILD thread, but that'll be fixed when we change the type
        // to PackagePiece.Builder.
        let thread = thread.with_payload(target_definition_context);
        let implementation = macro_
            .get_macro_class()
            .get_implementation()
            .to_owned_frozen_value();
        let macro_name = macro_.get_name().to_owned();
        // PORT: Java's updateStartedThreadComputationSteps accounting is skipped (starlark-rust
        // does not count Java's steps; see PackageFactory.executeBuildFileImpl).
        let result = blaze_starlark::eval::call_frozen(
            &implementation,
            &thread,
            &EvalOptions::default(),
            |module| {
                let heap = module.heap();
                let mut kwargs: Vec<(String, Value)> = Vec::new();
                for (name, value, configurable) in &attr_values {
                    let mut v = Attribute::value_to_starlark(value.as_ref(), heap);
                    if *configurable && SelectorList::from_value(v).is_none() && !v.is_none() {
                        v = heap.alloc(SelectorList::wrap_single_value(v, heap));
                    }
                    kwargs.push((name.clone(), v));
                }
                (Vec::new(), kwargs)
            },
            |return_value, _module, _eval| {
                if !return_value.is_none() {
                    return Err(EvalError {
                        kind: EvalErrorKind::Eval,
                        message: format!(
                            "macro '{}' may not return a non-None value (got {})",
                            macro_name,
                            return_value.to_repr()
                        ),
                        callstack: Vec::new(),
                        include_stack: true,
                    });
                }
                Ok(())
            },
        );
        let mut target_definition_context = thread
            .take_payload::<TargetDefinitionContext>()
            .expect("the macro thread context holds the package builder");
        let result = match result {
            Ok(()) => Ok(()),
            // from either call() or non-None return
            Err(mut ex) => {
                if ex.kind != EvalErrorKind::Cancelled
                    && (ex.callstack.is_empty()
                        || ex.callstack[0]
                            .location
                            .as_ref()
                            .is_some_and(|l| l.file.ends_with(".bzl")))
                {
                    // If the call stack starts at a .bzl file (i.e. at the macro definition),
                    // prepend the call stacks of all outer threads to it, so that the user
                    // understands how the failing macro was instantiated.
                    let mut callstack = macro_.reconstruct_parent_call_stack();
                    callstack.append(&mut ex.callstack);
                    ex.callstack = callstack;
                    ex.read_missing_source_lines();
                }
                Err(ex)
            }
        };
        // Restore the previously running symbolic macro's state (if any).
        let top = target_definition_context.set_current_macro_frame(parent_macro_frame);
        blaze_util::check_state!(
            top.as_ref()
                .is_some_and(|f| Arc::ptr_eq(&f.macro_instance, macro_)),
            "inconsistent macro stack state"
        );
        // Mark the macro as having completed, even if it was in error (or interrupted?).
        if target_definition_context.is_package_builder() {
            target_definition_context.mark_macro_complete(macro_);
        }
        (target_definition_context, result)
    }

    /// Java `shouldForceDefaultToNone`: non-direct defaults and legacy license attributes.
    // Java: com/google/devtools/build/lib/packages/MacroClass.java:351 MacroClass.shouldForceDefaultToNone
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
