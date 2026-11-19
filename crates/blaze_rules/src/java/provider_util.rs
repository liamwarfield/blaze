//! The parts of `packages/StarlarkProviderWrapper.java`, `packages/StarlarkInfoWithSchema.java`
//! and `packages/Info.java` the Java rule natives use on provider instances defined in
//! rules_java.

use blaze_analysis::info::info_provider_key;
use blaze_cmdline::Label;
use blaze_packages::rule_class::ProviderKey;
use blaze_packages::starlark_defs::provider::StarlarkProvider;
use blaze_starlark::structs::StarlarkInfo;
use starlark::values::Value;

use super::java_semantics::RULES_JAVA_PROVIDER_LABELS_PREFIX;

/// Java `StarlarkProviderWrapper.getKey()` of a wrapper constructed with
/// `keyForBuild(Label.parseCanonicalUnchecked(bzl))` and the provider `name`.
// Java: com/google/devtools/build/lib/packages/StarlarkProviderWrapper.java:47 StarlarkProviderWrapper.<init>
pub fn starlark_provider_key(bzl: &str, name: &str) -> ProviderKey {
    ProviderKey::Starlark {
        extension_label: Label::parse_canonical_unchecked(bzl),
        exported_name: name.to_owned(),
    }
}

/// The key of a provider rules_java defines, as Bazel's `PROVIDER` wrappers name it
/// (`RULES_JAVA_PROVIDER_LABELS_PREFIX + bzl`).
pub fn rules_java_provider_key(bzl: &str, name: &str) -> ProviderKey {
    starlark_provider_key(&format!("{RULES_JAVA_PROVIDER_LABELS_PREFIX}{bzl}"), name)
}

/// The key of the `RULES_JAVA_PROVIDER` wrappers (rules_java as the main repository:
/// `keyForBuild(Label.parseCanonicalUnchecked("//" + bzl))`).
pub fn main_repo_provider_key(bzl: &str, name: &str) -> ProviderKey {
    starlark_provider_key(&format!("//{bzl}"), name)
}

/// Java `info.getProvider().getKey()`; `None` when `v` is not an `Info`.
pub fn info_key(v: Value) -> Option<ProviderKey> {
    info_provider_key(v)
}

/// Java `value instanceof StarlarkInfoWithSchema`.
pub fn is_starlark_info_with_schema(v: Value) -> bool {
    StarlarkInfo::from_value(v)
        .and_then(|info| StarlarkProvider::from_value(info.provider()))
        .is_some_and(|p| p.get_fields().is_some())
}

/// Java `value instanceof StructImpl` (Starlark provider instances and structs; native infos
/// with fields are not wrapped by the Java rule natives).
pub fn is_struct_impl(v: Value) -> bool {
    StarlarkInfo::from_value(v).is_some()
}

/// Java `Starlark.type(x)`.
pub fn starlark_type(v: Value) -> String {
    v.get_type().to_owned()
}
