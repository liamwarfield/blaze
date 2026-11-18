//! Attribute definitions and attribute value lookup: ports of `packages/Attribute.java`
//! (with its nested `Builder`, `ImmutableAttributeFactory`, `AllowedValueSet`,
//! `ComputedDefault`, `StarlarkComputedDefaultTemplate`, `LateBoundDefault`, ...),
//! `AttributeValueSource`, `AttributeTransitionData`, `ImplicitOutputsFunction`,
//! `AttributeProvider`, `AttributeMap` and its mappers, and `DependencyFilter`.

pub mod abstract_attribute_mapper;
pub mod aggregating_attribute_mapper;
#[allow(clippy::module_inception)]
pub mod attribute;
pub mod attribute_map;
pub mod attribute_provider;
pub mod attribute_value_source;
pub mod computed_default;
pub mod dependency_filter;
pub mod implicit_outputs_function;
pub mod nonconfigurable_attribute_mapper;
pub mod nonconfigured_attribute_mapper;
pub mod raw_attribute_mapper;
pub mod string_util;
pub mod transition_factory;

pub use abstract_attribute_mapper::AbstractAttributeMapper;
pub use aggregating_attribute_mapper::AggregatingAttributeMapper;
pub use attribute::AllowedValueSet;
pub use attribute::AllowedValues;
pub use attribute::Attribute;
pub use attribute::AttributeBuilder;
pub use attribute::ImmutableAttributeFactory;
pub use attribute::PredicateWithMessage;
pub use attribute::PropertyFlag;
pub use attribute::any_rule;
pub use attribute::attr;
pub use attribute_map::AttributeMap;
pub use attribute_provider::AttributeProvider;
pub use attribute_value_source::AttributeValueSource;
pub use computed_default::CannotPrecomputeDefaultsException;
pub use computed_default::ComputedDefault;
pub use computed_default::ComputedDefaultKind;
pub use computed_default::LateBoundDefault;
pub use computed_default::LateBoundDefaultEvaluator;
pub use computed_default::LateBoundDefaultKind;
pub use computed_default::MaterializingDefault;
pub use computed_default::NativeComputedDefaultFn;
pub use computed_default::StarlarkComputedDefault;
pub use computed_default::StarlarkComputedDefaultTemplate;
pub use dependency_filter::AttributeInfoProvider;
pub use dependency_filter::DependencyFilter;
pub use implicit_outputs_function::ImplicitOutputsFunction;
pub use implicit_outputs_function::NativeImplicitOutputsFn;
pub use nonconfigurable_attribute_mapper::NonconfigurableAttributeMapper;
pub use nonconfigured_attribute_mapper::NonconfiguredAttributeMapper;
pub use raw_attribute_mapper::RawAttributeMapper;
pub use transition_factory::AttributeTransitionData;
pub use transition_factory::TransitionFactory;
pub use transition_factory::TransitionType;

#[cfg(test)]
mod tests_attribute;
#[cfg(test)]
mod tests_implicit_outputs;
#[cfg(test)]
mod tests_mappers;
