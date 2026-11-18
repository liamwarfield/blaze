//! `CcToolchainProvider`: the Java view over rules_cc's `CcToolchainInfo` provider instance
//! (`CcToolchainProvider.create(starlarkInfo)` / `wrap(info)`).
//!
//! Getters read the provider fields on demand from the (live or frozen) Starlark value, with
//! Java's `Structure.getValue(name, type)` errors.

use std::sync::{Arc, LazyLock};

use blaze_actions::Artifact;
use blaze_actions::analysis::starlark::starlark_file::artifact_from_value;
use blaze_cc_features::cc_toolchain_features::CcToolchainFeatures;
use blaze_cmdline::Label;
use blaze_collect::NestedSet;
use blaze_packages::rule_class::ProviderKey;
use blaze_packages::starlark_defs::provider::{StarlarkProvider, provider_view};
use blaze_starlark::depset::Depset;
use blaze_starlark::structs::StarlarkInfo;
use blaze_vfs::PathFragment;
use starlark::values::{Heap, Value};

use super::starlark_values::cc_toolchain_features_of;
use super::util::{depset_get_set_artifacts, errorf, field_type_error, sequence_elements};

/// Java `STARLARK_NAME`.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainProvider.java:52 CcToolchainProvider.STARLARK_NAME
pub const STARLARK_NAME: &str = "CcToolchainInfo";

/// The keys of Java's `RULES_CC_PROVIDER`, `BAZEL_PROVIDER` and `GOOGLE_PROVIDER`: the
/// `CcToolchainInfo` symbol of `cc/private/rules_impl:cc_toolchain_info.bzl` in the main
/// repository (rules_cc built as the main module), in `@rules_cc+`, or in Google's source tree.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainProvider.java:92 CcToolchainProvider.RulesCcCcToolchainInfoProvider
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainProvider.java:101 CcToolchainProvider.BazelCcToolchainInfoProvider
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainProvider.java:111 CcToolchainProvider.GoogleCcToolchainInfoProvider
pub fn cc_toolchain_info_provider_keys() -> &'static [ProviderKey; 3] {
    static KEYS: LazyLock<[ProviderKey; 3]> = LazyLock::new(|| {
        let key = |bzl: &str| ProviderKey::Starlark {
            extension_label: Label::parse_canonical_unchecked(bzl),
            exported_name: STARLARK_NAME.to_owned(),
        };
        [
            key("//cc/private/rules_impl:cc_toolchain_info.bzl"),
            key("@rules_cc+//cc/private/rules_impl:cc_toolchain_info.bzl"),
            key("//third_party/bazel_rules/rules_cc/cc/private/rules_impl:cc_toolchain_info.bzl"),
        ]
    });
    &KEYS
}

/// Information about a C++ compiler used by the `cc_*` rules.
// Java: com/google/devtools/build/lib/rules/cpp/CcToolchainProvider.java:50 CcToolchainProvider
#[derive(Clone, Copy)]
pub struct CcToolchainProvider<'v> {
    value: &'v StarlarkInfo<'v>,
}

impl<'v> CcToolchainProvider<'v> {
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainProvider.java:205 CcToolchainProvider.create
    pub fn create(value: &'v StarlarkInfo<'v>) -> CcToolchainProvider<'v> {
        CcToolchainProvider { value }
    }

    /// Java `CcToolchainProvider.wrapOrThrowEvalException(info)`: `info` must be an instance (with
    /// schema) of one of the three `CcToolchainInfo` providers, identified by their keys
    /// ([`cc_toolchain_info_provider_keys`]). A provider that is merely named `CcToolchainInfo`
    /// is rejected.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainProvider.java:59 CcToolchainProvider.wrapOrThrowEvalException
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainProvider.java:124 CcToolchainInfoProvider.wrapOrThrowEvalException
    pub fn wrap(v: Value<'v>) -> starlark::Result<CcToolchainProvider<'v>> {
        if let Some(info) = StarlarkInfo::from_value(v) {
            let provider = info.provider();
            // Java: `value instanceof StarlarkInfoWithSchema`: the provider declares its fields.
            let with_schema =
                StarlarkProvider::from_value(provider).is_some_and(|p| p.get_fields().is_some());
            // Java: `value.getProvider().getKey().equals(getKey())`.
            let key_matches = provider_view(provider)
                .and_then(|view| view.key)
                .is_some_and(|key| cc_toolchain_info_provider_keys().contains(&key));
            if with_schema && key_matches {
                return Ok(CcToolchainProvider { value: info });
            }
        }
        Err(errorf(format!(
            "got value of type '{}', want 'CcToolchainInfo'",
            v.get_type()
        )))
    }

    /// `CcToolchainProvider.create((StarlarkInfo) v)` for a parameter value.
    pub fn of_param(v: Value<'v>, function: &str, param: &str) -> starlark::Result<Self> {
        match StarlarkInfo::from_value(v) {
            Some(info) => Ok(CcToolchainProvider::create(info)),
            None => Err(errorf(format!(
                "in call to {function}(), parameter '{param}' got value of type '{}', want \
                 'struct'",
                v.get_type()
            ))),
        }
    }

    fn get(&self, key: &str) -> Option<Value<'v>> {
        self.value.get_field(key)
    }

    /// `value.getValue(key, Depset.class).getSet(Artifact.class)` with `TypeException` wrapped
    /// in `EvalException`.
    fn get_artifact_set(&self, key: &str) -> starlark::Result<NestedSet<Artifact>> {
        let Some(v) = self.get(key) else {
            panic!("NullPointerException: CcToolchainInfo.{key}");
        };
        if Depset::from_value(v).is_none() {
            return Err(field_type_error(key, v, "depset"));
        }
        depset_get_set_artifacts(v).map_err(errorf)
    }

    fn get_string(&self, key: &str) -> starlark::Result<Option<String>> {
        match self.get(key) {
            None => Ok(None),
            Some(v) => match v.unpack_str() {
                Some(s) => Ok(Some(s.to_owned())),
                None => Err(field_type_error(key, v, "string")),
            },
        }
    }

    fn get_bool(&self, key: &str) -> starlark::Result<bool> {
        match self.get(key) {
            None => panic!("NullPointerException: CcToolchainInfo.{key}"),
            Some(v) => v
                .unpack_bool()
                .ok_or_else(|| field_type_error(key, v, "bool")),
        }
    }

    /// Whether the toolchains supports header parsing.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainProvider.java:228 CcToolchainProvider.supportsHeaderParsing
    pub fn supports_header_parsing(&self) -> starlark::Result<bool> {
        self.get_bool("_supports_header_parsing")
    }

    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainProvider.java:279 CcToolchainProvider.getBuiltInIncludeDirectories
    pub fn get_built_in_include_directories(
        &self,
        heap: Heap<'v>,
    ) -> starlark::Result<Vec<PathFragment>> {
        // Java: convertStarlarkListToPathFragments(value, "built_in_include_directories").
        let key = "built_in_include_directories";
        let Some(v) = self.get(key) else {
            panic!("NullPointerException: CcToolchainInfo.{key}");
        };
        if !super::util::is_sequence(v) {
            return Err(field_type_error(key, v, "sequence"));
        }
        let strings = super::util::sequence_cast_strings(v, key, heap)?;
        Ok(strings.iter().map(|s| PathFragment::create(s)).collect())
    }

    /// Returns the identifier of the toolchain as specified in the `CToolchain` proto.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainProvider.java:284 CcToolchainProvider.getToolchainIdentifier
    pub fn get_toolchain_identifier(&self) -> starlark::Result<Option<String>> {
        self.get_string("toolchain_id")
    }

    /// Returns all the files in Crosstool.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainProvider.java:289 CcToolchainProvider.getAllFiles
    pub fn get_all_files(&self) -> starlark::Result<NestedSet<Artifact>> {
        self.get_artifact_set("all_files")
    }

    /// Returns the files necessary for compilation.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainProvider.java:307 CcToolchainProvider.getCompilerFiles
    pub fn get_compiler_files(&self) -> starlark::Result<NestedSet<Artifact>> {
        self.get_artifact_set("_compiler_files")
    }

    /// Returns the files necessary for compilation excluding headers, assuming that included
    /// files will be discovered by input discovery.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainProvider.java:319 CcToolchainProvider.getCompilerFilesWithoutIncludes
    pub fn get_compiler_files_without_includes(&self) -> starlark::Result<NestedSet<Artifact>> {
        self.get_artifact_set("_compiler_files_without_includes")
    }

    /// Returns the files necessary for an 'as' invocation.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainProvider.java:333 CcToolchainProvider.getAsFiles
    pub fn get_as_files(&self) -> starlark::Result<NestedSet<Artifact>> {
        self.get_artifact_set("_as_files")
    }

    /// Returns the configured features of the toolchain.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainProvider.java:389 CcToolchainProvider.getFeatures
    pub fn get_features(&self) -> starlark::Result<Option<Arc<CcToolchainFeatures>>> {
        let key = "_toolchain_features";
        match self.get(key) {
            None => Ok(None),
            Some(v) if v.is_none() => {
                // Java: getValue(key, CcToolchainFeatures.class) casts None and fails.
                Err(field_type_error(key, v, "CcToolchainFeatures"))
            }
            Some(v) => cc_toolchain_features_of(v)
                .map(Some)
                .ok_or_else(|| field_type_error(key, v, "CcToolchainFeatures")),
        }
    }

    /// Return the set of include files that may be included even if they are not mentioned in
    /// the source file or any of the headers included by it.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainProvider.java:427 CcToolchainProvider.getBuiltinIncludeFiles
    pub fn get_builtin_include_files(&self, heap: Heap<'v>) -> starlark::Result<Vec<Artifact>> {
        let key = "_builtin_include_files";
        let Some(v) = self.get(key) else {
            panic!("NullPointerException: CcToolchainInfo.{key}");
        };
        if !super::util::is_sequence(v) {
            return Err(field_type_error(key, v, "sequence"));
        }
        super::util::sequence_cast_artifacts(v, key, heap)
    }

    /// Returns the grep-includes tool which is needing during linking because of linkstamping.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainProvider.java:437 CcToolchainProvider.getGrepIncludes
    pub fn get_grep_includes(&self) -> starlark::Result<Option<Artifact>> {
        let key = "_grep_includes";
        match self.get(key) {
            None => Ok(None),
            Some(v) if v.is_none() => Ok(None),
            Some(v) => artifact_from_value(v)
                .map(Some)
                .ok_or_else(|| field_type_error(key, v, "File")),
        }
    }

    /// Returns the legacy value of the CC_FLAGS Make variable.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainProvider.java:470 CcToolchainProvider.getLegacyCcFlagsMakeVariable
    pub fn get_legacy_cc_flags_make_variable(&self) -> starlark::Result<Option<String>> {
        self.get_string("_legacy_cc_flags_make_variable")
    }

    /// Return the name of the directory (relative to the bin directory) that holds mangled links
    /// to shared libraries.
    // Java: com/google/devtools/build/lib/rules/cpp/CcToolchainProvider.java:401 CcToolchainProvider.getSolibDirectory
    pub fn get_solib_directory(&self) -> starlark::Result<Option<String>> {
        self.get_string("_solib_dir")
    }

    /// The elements of a sequence field, for callers that need raw values.
    pub fn get_sequence(&self, key: &str, heap: Heap<'v>) -> starlark::Result<Vec<Value<'v>>> {
        match self.get(key) {
            None => panic!("NullPointerException: CcToolchainInfo.{key}"),
            Some(v) => sequence_elements(v, heap),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The three provider keys of CcToolchainProvider.java:92-118 (`keyForBuild(label)` and the
    /// exported name `CcToolchainInfo`): a provider with another key is not a `CcToolchainInfo`,
    /// whatever its name (analysis review S2-13).
    #[test]
    fn cc_toolchain_info_provider_keys_are_javas() {
        let keys: Vec<(String, String)> = cc_toolchain_info_provider_keys()
            .iter()
            .map(|key| match key {
                ProviderKey::Starlark {
                    extension_label,
                    exported_name,
                } => (
                    extension_label.get_unambiguous_canonical_form(),
                    exported_name.clone(),
                ),
                ProviderKey::Builtin { .. } => panic!("not a Starlark provider key"),
            })
            .collect();
        assert_eq!(
            keys,
            vec![
                (
                    "@@//cc/private/rules_impl:cc_toolchain_info.bzl".to_owned(),
                    "CcToolchainInfo".to_owned()
                ),
                (
                    "@@rules_cc+//cc/private/rules_impl:cc_toolchain_info.bzl".to_owned(),
                    "CcToolchainInfo".to_owned()
                ),
                (
                    "@@//third_party/bazel_rules/rules_cc/cc/private/rules_impl:cc_toolchain_info.bzl"
                        .to_owned(),
                    "CcToolchainInfo".to_owned()
                ),
            ]
        );
        // A provider of the same name defined elsewhere has another key.
        let user_key = ProviderKey::Starlark {
            extension_label: Label::parse_canonical_unchecked("//my:defs.bzl"),
            exported_name: STARLARK_NAME.to_owned(),
        };
        assert!(!cc_toolchain_info_provider_keys().contains(&user_key));
    }
}
