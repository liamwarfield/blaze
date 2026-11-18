//! Attribute types and converted values: ports of `packages/Type.java`, `Types.java`,
//! `BuildType.java`, `TriState.java`, `License.java`, `LabelConverter.java`,
//! `SelectorValue.java`, `SelectorList.java` (crates/blaze_packages/DESIGN.md "Types").

pub mod attr_value;
pub mod build_type;
pub mod conversion_exception;
pub mod file_type;
pub mod label_converter;
pub mod license;
pub mod select;
pub mod tri_state;
pub mod type_;

pub use attr_value::AttrValue;
pub use build_type::AttributeConversionContext;
pub use build_type::BuildTypeSelector;
pub use build_type::BuildTypeSelectorList;
pub use build_type::DEFAULT_CONDITION_KEY;
pub use conversion_exception::ConversionException;
pub use file_type::FileType;
pub use file_type::FileTypeSet;
pub use label_converter::LabelConverter;
pub use license::License;
pub use license::LicenseParsingException;
pub use license::LicenseType;
pub use select::FrozenSelectorList;
pub use select::FrozenSelectorValue;
pub use select::SelectorList;
pub use select::SelectorListGen;
pub use select::SelectorValue;
pub use select::SelectorValueGen;
pub use select::select_impl;
pub use tri_state::TriState;
pub use type_::LabelClass;
pub use type_::LabelVisitor;
pub use type_::Type;

#[cfg(test)]
pub(crate) mod test_util;
#[cfg(test)]
mod tests_build_type;
#[cfg(test)]
mod tests_license;
#[cfg(test)]
mod tests_select;
#[cfg(test)]
mod tests_type;
