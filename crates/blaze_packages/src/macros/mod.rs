//! Symbolic macros: ports of `packages/MacroClass.java` and `packages/MacroInstance.java`.
//! The macro frame of the package builder is `crate::package::MacroFrame`.

pub mod macro_class;
pub mod macro_instance;

pub use macro_class::MacroClass;
pub use macro_class::MacroClassBuilder;
pub use macro_instance::MacroInstance;
