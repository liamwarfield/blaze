//! The loading-phase aspect model: ports of `packages/{Aspect, AspectClass, NativeAspectClass,
//! StarlarkAspectClass, AspectDescriptor, AspectParameters, AspectDefinition,
//! AspectPropagationEdgesSupplier, AspectPropagationPredicate}`.
//!
//! The Starlark value made by `aspect()` stays in
//! [`crate::starlark_defs::aspect::StarlarkDefinedAspect`]; it builds [`AspectDefinition`]s
//! (`get_definition`) and knows its [`StarlarkAspectClass`]. `AspectsList` and `Attribute`
//! (in `rule_class` / `attribute`) instantiate [`Aspect`]s for rules.

#[allow(clippy::module_inception)]
pub mod aspect;
pub mod aspect_class;
pub mod aspect_definition;
pub mod aspect_descriptor;
pub mod aspect_parameters;
pub mod aspect_propagation_edges_supplier;
pub mod aspect_propagation_predicate;
pub mod native_aspect_class;
pub mod starlark_aspect_class;

pub use aspect::Aspect;
pub use aspect_class::AspectClass;
pub use aspect_definition::AspectDefinition;
pub use aspect_definition::AspectDefinitionBuilder;
pub use aspect_descriptor::AspectDescriptor;
pub use aspect_parameters::AspectParameters;
pub use aspect_parameters::AspectParametersBuilder;
pub use aspect_propagation_edges_supplier::ALL_TOOLCHAINS;
pub use aspect_propagation_edges_supplier::AspectPropagationEdgesSupplier;
pub use aspect_propagation_edges_supplier::AttrAspectsFunctionSupplier;
pub use aspect_propagation_edges_supplier::FixedListSupplier;
pub use aspect_propagation_edges_supplier::FunctionSupplier;
pub use aspect_propagation_edges_supplier::StarlarkAspectPropagationContext;
pub use aspect_propagation_edges_supplier::ToolchainsAspectsFunctionSupplier;
pub use aspect_propagation_predicate::AspectPropagationPredicate;
pub use native_aspect_class::NativeAspectClass;
pub use starlark_aspect_class::AspectClassCreationException;
pub use starlark_aspect_class::StarlarkAspectClass;

#[cfg(test)]
mod tests;
