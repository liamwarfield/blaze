//! Ports of `com/google/devtools/build/lib/starlarkdocextract` (structured API documentation of a
//! loaded Starlark module, the `stardoc_output.ModuleInfo` proto) and of
//! `src/tools/starlark/java/com/google/devtools/starlark/common/DocstringUtils.java`.

pub mod attribute_info_extractor;
pub mod docstring_utils;
pub mod extraction_exception;
pub mod extractor_context;
pub mod label_renderer;
pub mod provider_name_group_extractor;
pub mod rule_info_extractor;
pub mod starlark_function_info_extractor;
pub mod module_info_extractor;
