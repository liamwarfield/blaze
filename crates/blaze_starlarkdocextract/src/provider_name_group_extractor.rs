//! Port of `starlarkdocextract/ProviderNameGroupExtractor.java`.

use blaze_packages::rule_class::provider::{ProviderKey, StarlarkProviderIdentifier};
use blaze_protos::stardoc_output::{OriginKey, ProviderNameGroup};

use crate::extractor_context::ExtractorContext;

// Java: com/google/devtools/build/lib/starlarkdocextract/ProviderNameGroupExtractor.java:29 ProviderNameGroupExtractor.buildProviderNameGroup
pub fn build_provider_name_group<'a>(
    context: &ExtractorContext,
    provider_group: impl IntoIterator<Item = &'a StarlarkProviderIdentifier>,
) -> ProviderNameGroup {
    let mut provider_name_group = ProviderNameGroup::default();
    for provider in provider_group {
        provider_name_group
            .provider_name
            .push(context.get_documented_provider_name(provider));
        let mut provider_key = OriginKey {
            name: provider.to_string(),
            ..OriginKey::default()
        };
        match provider.get_key() {
            ProviderKey::Starlark {
                extension_label, ..
            } => {
                provider_key.file = context.label_renderer.render(extension_label);
            }
            ProviderKey::Builtin { .. } => {
                provider_key.file = "<native>".to_owned();
            }
        }
        provider_name_group.origin_key.push(provider_key);
    }
    provider_name_group
}
