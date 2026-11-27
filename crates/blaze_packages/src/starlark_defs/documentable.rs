//! What `starlark_doc_extract` reads from the documentable values defined outside this crate:
//! repository rules (Java `StarlarkRepoRule` / `RepoRule`) and module extensions (Java
//! `ModuleExtension` / `TagClass`). Those values live in `blaze_repo_rules`, which the analysis
//! side (where the `starlark_doc_extract` rule is) cannot depend on; they provide these
//! descriptions through `StarlarkValue::provide`, and the extractor requests them.

use blaze_cmdline::Label;
use starlark::any::ProvidesStaticType;
use starlark::values::Value;

use crate::attribute::Attribute;

/// A `repository_rule` value as `ModuleInfoExtractor.visitRepositoryRule` reads it.
#[derive(Debug, Clone, ProvidesStaticType)]
pub struct DocumentableRepoRule {
    /// `StarlarkExportable.isExported()`.
    pub exported: bool,
    /// `repoRule.doc()`.
    pub doc: Option<String>,
    /// `repoRule.id().ruleName()` (set when exported).
    pub rule_name: Option<String>,
    /// `repoRule.id().bzlFileLabel()`.
    pub bzl_file_label: Label,
    /// `repoRule.attributes()`.
    pub attributes: Vec<Attribute>,
    /// `repoRule.environ()`.
    pub environ: Vec<String>,
}

/// A tag class of a module extension (`TagClass.doc()`, `attributes()`).
#[derive(Debug, Clone)]
pub struct DocumentableTagClass {
    pub name: String,
    pub doc: Option<String>,
    pub attributes: Vec<Attribute>,
}

/// A `module_extension` value as `ModuleInfoExtractor.visitModuleExtension` reads it.
#[derive(Debug, Clone, ProvidesStaticType)]
pub struct DocumentableModuleExtension {
    /// `moduleExtension.doc()`.
    pub doc: Option<String>,
    /// `moduleExtension.definingBzlFileLabel()`.
    pub defining_bzl_file_label: Label,
    /// `moduleExtension.tagClasses()`, in order.
    pub tag_classes: Vec<DocumentableTagClass>,
}

/// The repository rule description `value` provides, if it is a `repository_rule`.
pub fn documentable_repo_rule(value: Value) -> Option<DocumentableRepoRule> {
    value.request_value::<DocumentableRepoRule>()
}

/// The module extension description `value` provides, if it is a `module_extension`.
pub fn documentable_module_extension(value: Value) -> Option<DocumentableModuleExtension> {
    value.request_value::<DocumentableModuleExtension>()
}
