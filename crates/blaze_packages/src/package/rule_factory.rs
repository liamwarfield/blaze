//! Port of `packages/RuleFactory.java`: creating rule instances from attribute values.

use std::sync::Arc;

use blaze_starlark::Location;
use blaze_starlark::error::StackEntry;
use indexmap::IndexMap;
use parking_lot::Mutex;
use starlark::values::FrozenHeap;
use starlark::values::FrozenHeapRef;
use starlark::values::Value;

use crate::package::NameConflictException;
use crate::package::TargetDefinitionContext;
use crate::rule_class::RuleClass;
use crate::rule_class::RuleClassType;
use crate::rule_class::RuleFunction;
use crate::target::Rule;

/// A rule could not be created (Java `RuleFactory.InvalidRuleException`).
// Java: com/google/devtools/build/lib/packages/RuleFactory.java:152 RuleFactory.InvalidRuleException
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{0}")]
pub struct InvalidRuleException(pub String);

/// Attribute values given in the build language (Starlark values) by name (Java
/// `RuleFactory.BuildLangTypedAttributeValuesMap`, the implementation of
/// `RuleFactory.AttributeValues` in use).
// Java: com/google/devtools/build/lib/packages/RuleFactory.java:181 RuleFactory.BuildLangTypedAttributeValuesMap
#[derive(Debug, Clone)]
pub struct BuildLangTypedAttributeValuesMap<'v> {
    pub attribute_values: IndexMap<String, Value<'v>>,
}

impl<'v> BuildLangTypedAttributeValuesMap<'v> {
    // Java: com/google/devtools/build/lib/packages/RuleFactory.java:186 RuleFactory.BuildLangTypedAttributeValuesMap.<init>
    pub fn new(attribute_values: IndexMap<String, Value<'v>>) -> Self {
        BuildLangTypedAttributeValuesMap { attribute_values }
    }

    // Java: com/google/devtools/build/lib/packages/RuleFactory.java:190 RuleFactory.BuildLangTypedAttributeValuesMap.containsAttributeNamed
    pub fn contains_attribute_named(&self, attribute_name: &str) -> bool {
        self.attribute_values.contains_key(attribute_name)
    }

    // Java: com/google/devtools/build/lib/packages/RuleFactory.java:194 RuleFactory.BuildLangTypedAttributeValuesMap.getAttributeValue
    pub fn get_attribute_value(&self, attribute_name: &str) -> Option<Value<'v>> {
        self.attribute_values.get(attribute_name).copied()
    }

    /// Java `AttributeValues.getAttributeAccessors()`: the entries in order.
    // Java: com/google/devtools/build/lib/packages/RuleFactory.java:204 RuleFactory.BuildLangTypedAttributeValuesMap.getAttributeAccessors
    pub fn get_attribute_accessors(&self) -> impl Iterator<Item = (&str, Value<'v>)> {
        self.attribute_values.iter().map(|(k, v)| (k.as_str(), *v))
    }

    /// Java `isExplicitlySpecified`: true for every entry.
    // Java: com/google/devtools/build/lib/packages/RuleFactory.java:219 RuleFactory.BuildLangTypedAttributeValuesMap.isExplicitlySpecified
    pub fn is_explicitly_specified(&self, attribute_name: &str) -> bool {
        let _ = attribute_name;
        true
    }
}

/// Errors of [`create_and_add_rule`] (Java: `InvalidRuleException`, `NameConflictException`).
#[derive(Debug, Clone, thiserror::Error)]
pub enum CreateAndAddRuleError {
    #[error("{0}")]
    InvalidRule(#[from] InvalidRuleException),
    #[error("{0}")]
    NameConflict(#[from] NameConflictException),
}

/// Creates a rule instance (the caller adds it to the package). `callstack` is the stack of
/// the creating thread, innermost frame (the rule function) last; for rules instantiated by a
/// symbolic macro, it is the inner macro's stack.
// Java: com/google/devtools/build/lib/packages/RuleFactory.java:58 RuleFactory.createRule
pub fn create_rule<'v>(
    target_definition_context: &TargetDefinitionContext,
    rule_class: &Arc<RuleClass>,
    attribute_values: &BuildLangTypedAttributeValuesMap<'v>,
    fail_on_unknown_attributes: bool,
    callstack: &[StackEntry],
) -> Result<Rule, InvalidRuleException> {
    let rule_class_name = rule_class.get_name();
    let name_object = attribute_values.get_attribute_value("name");
    let name = match name_object {
        None => {
            return Err(InvalidRuleException(format!(
                "{rule_class_name} rule has no 'name' attribute"
            )));
        }
        Some(v) => match v.unpack_str() {
            Some(s) => s.to_owned(),
            None => {
                return Err(InvalidRuleException(format!(
                    "{rule_class_name} 'name' attribute must be a string"
                )));
            }
        },
    };
    // Test that this would form a valid label name -- in particular, this
    // catches cases where Makefile variables $(foo) appear in "name".
    let label = match target_definition_context.create_label(&name) {
        Ok(label) => label,
        Err(e) => {
            return Err(InvalidRuleException(format!(
                "illegal rule name: {}: {}",
                name,
                e.get_message()
            )));
        }
    };

    // Add the generator_name attribute.
    let generator_name = get_generator_name(target_definition_context, attribute_values, callstack);
    // Don't bother copying anything if nothing changed.
    let processed_attributes_owned;
    let processed_attributes = match generator_name {
        Some(generator_name) => {
            // Java: ImmutableMap.Builder.buildKeepingLast (the key is new, see getGeneratorName).
            let mut values = attribute_values.attribute_values.clone();
            values.insert(
                "generator_name".to_owned(),
                alloc_generator_name(&generator_name),
            );
            processed_attributes_owned = BuildLangTypedAttributeValuesMap::new(values);
            &processed_attributes_owned
        }
        None => attribute_values,
    };

    // The raw stack is of the form [<toplevel>@BUILD:1, macro@lib.bzl:1, cc_library@<builtin>].
    // Pop the innermost frame for the rule, since it's obvious.
    let callstack = &callstack[..callstack.len() - 1]; // pop

    match rule_class.create_rule(
        target_definition_context,
        label,
        processed_attributes,
        fail_on_unknown_attributes,
        callstack,
    ) {
        Ok(rule) => Ok(rule),
        Err(e) => Err(InvalidRuleException(format!("{rule_class} {e}"))),
    }
}

/// Creates a rule and adds it to the package builder.
// Java: com/google/devtools/build/lib/packages/RuleFactory.java:122 RuleFactory.createAndAddRule
pub fn create_and_add_rule<'v>(
    target_definition_context: &mut TargetDefinitionContext,
    rule_class: &Arc<RuleClass>,
    attribute_values: &BuildLangTypedAttributeValuesMap<'v>,
    fail_on_unknown_attributes: bool,
    callstack: &[StackEntry],
) -> Result<Arc<Rule>, CreateAndAddRuleError> {
    let rule = Arc::new(create_rule(
        target_definition_context,
        rule_class,
        attribute_values,
        fail_on_unknown_attributes,
        callstack,
    )?);
    target_definition_context.add_rule(rule.clone())?;
    Ok(rule)
}

/// The `generator_name` to add for a rule being instantiated, or `None`.
// Java: com/google/devtools/build/lib/packages/RuleFactory.java:224 RuleFactory.getGeneratorName
fn get_generator_name<'v>(
    target_definition_context: &TargetDefinitionContext,
    args: &BuildLangTypedAttributeValuesMap<'v>,
    callstack: &[StackEntry],
) -> Option<String> {
    let macro_ = target_definition_context.current_macro();
    // The "generator" of a rule is the function outermost in the BUILD file thread's call stack
    // (regardless of whether or not it was passed a "name" parameter). For rules with
    // generators, the BUILD file thread's call stack must contain at least two entries:
    //   0: the outermost function (BUILD file top level),
    //   1: the function called by it (e.g. a macro in a .bzl file),
    // optionally followed by other Starlark or built-in functions, and finally - if the rule is
    // instantiated in the BUILD file thread - the rule instantiation function.
    if macro_.is_none() && (callstack.len() < 2 || !entry_file(&callstack[1]).ends_with(".bzl")) {
        // We are in a BUILD file thread, and the rule is being instantiated directly at the top
        // level of the BUILD file.
        return None;
    }

    if args.contains_attribute_named("generator_name") {
        // generator_name is explicitly set. Don't override it.
        return None;
    }

    let generator_name = match macro_ {
        Some(m) => m.get_generator_name().map(str::to_owned),
        None => target_definition_context
            .get_generator_name_by_location(&entry_location(&callstack[0]))
            .map(str::to_owned),
    };
    match generator_name {
        Some(g) => Some(g),
        // Fall back on target name (meh).
        None => args
            .get_attribute_value("name")
            .and_then(|v| v.unpack_str())
            .map(str::to_owned),
    }
}

/// Frozen heaps holding `generator_name` strings; kept for the life of the process.
// PORT: keep-alive. Java puts a java.lang.String into the attribute map; a Starlark value needs a
// heap, and the caller's heap is not reachable here.
static GENERATOR_NAME_HEAPS: Mutex<Vec<FrozenHeapRef>> = Mutex::new(Vec::new());

/// A Starlark string value for `generator_name`, valid for any heap lifetime.
fn alloc_generator_name<'v>(generator_name: &str) -> Value<'v> {
    let heap = FrozenHeap::new();
    let value = heap.alloc_str(generator_name).to_frozen_value();
    GENERATOR_NAME_HEAPS.lock().push(heap.into_ref());
    value.to_value()
}

/// Java `entry.location` (a builtin frame has `Location.BUILTIN`).
fn entry_location(entry: &StackEntry) -> Location {
    entry.location.clone().unwrap_or_else(Location::builtin)
}

/// Java `entry.location.file()`.
fn entry_file(entry: &StackEntry) -> String {
    entry_location(entry).file
}

/// The native rule function of a rule class (Java `RuleFactory.BuiltinRuleFunction`). The
/// Starlark value wrapping it is defined with the native rules.
// Java: com/google/devtools/build/lib/packages/RuleFactory.java:296 RuleFactory.BuiltinRuleFunction
#[derive(Debug, Clone)]
pub struct BuiltinRuleFunction {
    rule_class: Arc<RuleClass>,
}

impl BuiltinRuleFunction {
    // Java: com/google/devtools/build/lib/packages/RuleFactory.java:299 RuleFactory.BuiltinRuleFunction.<init>
    pub fn new(rule_class: Arc<RuleClass>) -> Self {
        BuiltinRuleFunction { rule_class }
    }

    // Java: com/google/devtools/build/lib/packages/RuleFactory.java:332 RuleFactory.BuiltinRuleFunction.getName
    pub fn get_name(&self) -> &str {
        self.rule_class.get_name()
    }
}

impl RuleFunction for BuiltinRuleFunction {
    // Java: com/google/devtools/build/lib/packages/RuleFactory.java:327 RuleFactory.BuiltinRuleFunction.getRuleClass
    fn get_rule_class(&self) -> Arc<RuleClass> {
        self.rule_class.clone()
    }
}

/// The rule functions of `rule_classes` by name.
// Java: com/google/devtools/build/lib/packages/RuleFactory.java:282 RuleFactory.buildRuleFunctions
pub fn build_rule_functions(
    rule_classes: &IndexMap<String, Arc<RuleClass>>,
) -> IndexMap<String, BuiltinRuleFunction> {
    let mut result = IndexMap::new();
    for (rule_class_name, cl) in rule_classes {
        if matches!(
            cl.get_rule_class_type(),
            RuleClassType::Normal | RuleClassType::Test | RuleClassType::BuildOnly
        ) {
            result.insert(
                rule_class_name.clone(),
                BuiltinRuleFunction::new(cl.clone()),
            );
        }
    }
    result
}
