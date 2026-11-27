//! Port of `starlarkdocextract/ExtractorContext.java`.

use std::collections::HashMap;

use blaze_packages::rule_class::provider::{ProviderKey, StarlarkProviderIdentifier};

use crate::label_renderer::LabelRenderer;

/// The context of an extraction: the label renderer, the qualified names of the providers
/// loadable from the module, and whether natively defined attributes are extracted.
// Java: com/google/devtools/build/lib/starlarkdocextract/ExtractorContext.java:27 ExtractorContext
#[derive(Debug, Clone)]
pub struct ExtractorContext {
    pub label_renderer: LabelRenderer,
    pub provider_qualified_names: HashMap<ProviderKey, String>,
    pub extract_natively_defined_attrs: bool,
}

impl ExtractorContext {
    /// `ExtractorContext.builder()...build()` with the given renderer and the builder's defaults.
    // Java: com/google/devtools/build/lib/starlarkdocextract/ExtractorContext.java:38 ExtractorContext.builder
    pub fn new(label_renderer: LabelRenderer) -> Self {
        ExtractorContext {
            label_renderer,
            provider_qualified_names: HashMap::new(),
            extract_natively_defined_attrs: false,
        }
    }

    /// Returns true if the name should be, by default, considered for documentation extraction or
    /// for recursing into.
    // Java: com/google/devtools/build/lib/starlarkdocextract/ExtractorContext.java:68 ExtractorContext.isPublicName
    pub fn is_public_name(name: &str) -> bool {
        // Character.isAlphabetic(name.charAt(0))
        name.chars().next().is_some_and(char::is_alphabetic)
    }

    /// Returns the human-readable provider name suitable for use in a given module's
    /// documentation. For a provider loadable from that module, this is intended to be the
    /// qualified name (or more precisely, the first qualified name) under which a user of this
    /// module may access it. For local providers and for providers loaded but not re-exported
    /// via a global, it's the provider key name (a.k.a. `provider.toString()`).
    // Java: com/google/devtools/build/lib/starlarkdocextract/ExtractorContext.java:80 ExtractorContext.getDocumentedProviderName
    pub fn get_documented_provider_name(&self, provider: &StarlarkProviderIdentifier) -> String {
        if let Some(qualified_name) = self.provider_qualified_names.get(provider.get_key()) {
            return qualified_name.clone();
        }
        provider.to_string()
    }
}
