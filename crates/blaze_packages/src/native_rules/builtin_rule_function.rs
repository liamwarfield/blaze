//! The Starlark value of a native rule function (Java `RuleFactory.BuiltinRuleFunction`, a
//! `StarlarkCallable`; the Rust [`BuiltinRuleFunction`] holds its rule class).

use std::fmt;
use std::sync::Arc;

use allocative::Allocative;
use indexmap::IndexMap;
use starlark::any::ProvidesStaticType;
use starlark::environment::GlobalsBuilder;
use starlark::eval::Arguments;
use starlark::eval::Evaluator;
use starlark::starlark_simple_value;
use starlark::values::NoSerialize;
use starlark::values::StarlarkValue;
use starlark::values::Value;
use starlark::values::starlark_value;

use crate::build::eval_err;
use crate::build::thread_call_stack;
use crate::package::BuildLangTypedAttributeValuesMap;
use crate::package::BuiltinRuleFunction;
use crate::package::TargetDefinitionContext;
use crate::package::rule_factory::create_and_add_rule;
use crate::rule_class::RuleClass;
use crate::rule_class::RuleClassType;
use crate::rule_class::RuleFunction;

/// A callable Starlark value that creates rules of a native rule class (Starlark type
/// `rule`).
// Java: com/google/devtools/build/lib/packages/RuleFactory.java:296 RuleFactory.BuiltinRuleFunction
#[derive(Debug, Clone, ProvidesStaticType, NoSerialize, Allocative)]
pub struct BuiltinRuleFunctionValue(#[allocative(skip)] pub BuiltinRuleFunction);

starlark_simple_value!(BuiltinRuleFunctionValue);

impl BuiltinRuleFunctionValue {
    pub fn new(function: BuiltinRuleFunction) -> Self {
        BuiltinRuleFunctionValue(function)
    }

    // Java: com/google/devtools/build/lib/packages/RuleFactory.java:332 RuleFactory.BuiltinRuleFunction.getRuleClass
    pub fn get_rule_class(&self) -> Arc<RuleClass> {
        self.0.get_rule_class()
    }

    // Java: com/google/devtools/build/lib/packages/RuleFactory.java:337 RuleFactory.BuiltinRuleFunction.getName
    pub fn get_name(&self) -> &str {
        self.0.get_name()
    }
}

impl fmt::Display for BuiltinRuleFunctionValue {
    // Java: com/google/devtools/build/lib/packages/RuleFactory.java:342 RuleFactory.BuiltinRuleFunction.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<built-in rule {}>", self.get_name())
    }
}

#[starlark_value(type = "rule")]
impl<'v> StarlarkValue<'v> for BuiltinRuleFunctionValue {
    // Java: com/google/devtools/build/lib/packages/RuleFactory.java:304 RuleFactory.BuiltinRuleFunction.call
    fn invoke(
        &self,
        _me: Value<'v>,
        args: &Arguments<'v, '_>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        if args.positions(eval.heap())?.next().is_some() {
            return Err(eval_err("unexpected positional arguments"));
        }
        let kwargs: IndexMap<String, Value<'v>> = args
            .names_map()?
            .into_iter()
            .map(|(k, v)| (k.as_str().to_owned(), v))
            .collect();
        let rule_class = self.get_rule_class();
        let fail_on_unknown_attributes = blaze_starlark::ThreadContext::get(eval)
            .map_err(starlark::Error::new_other)?
            .semantics
            .incompatible_fail_on_unknown_attributes;
        let callstack = thread_call_stack(eval);
        let mut target_definition_context = match rule_class.get_rule_class_type() {
            RuleClassType::BuildOnly => {
                TargetDefinitionContext::from_or_fail_allow_build_only_participle(
                    eval,
                    &format!("{} rule", rule_class.get_name()),
                    "instantiated",
                )
            }
            _ => TargetDefinitionContext::from_or_fail_participle(eval, "a rule", "instantiated"),
        }
        .map_err(eval_err)?;
        create_and_add_rule(
            &mut target_definition_context,
            &rule_class,
            &BuildLangTypedAttributeValuesMap::new(kwargs),
            fail_on_unknown_attributes,
            &callstack,
        )
        .map_err(eval_err)?;
        Ok(Value::new_none())
    }
}

/// Adds the rule functions as globals of `builder` (Java: the `ruleFunctions` entries of the
/// BUILD environment and of `native`, in `BazelStarlarkEnvironment`).
pub fn add_rule_functions(
    builder: &mut GlobalsBuilder,
    rule_functions: &IndexMap<String, BuiltinRuleFunction>,
) {
    for (name, function) in rule_functions {
        builder.set(name, BuiltinRuleFunctionValue::new(function.clone()));
    }
}
