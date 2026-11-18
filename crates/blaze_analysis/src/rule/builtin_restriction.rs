//! Port of `packages/BuiltinRestriction.java` (the allowlist checks analysis-time builtins use).
//!
//! PORT: move to `blaze_packages` (its `builtin_providers.rs` has a private copy of the
//! empty-allowlist case marked the same way).

use blaze_cmdline::{Label, RepositoryMapping, RepositoryName};
use blaze_packages::bzl_context::BazelModuleContext;
use blaze_vfs::PathFragment;
use starlark::eval::Evaluator;

/// An entry of an allowlist: an apparent repo name and a package prefix.
// Java: com/google/devtools/build/lib/packages/BuiltinRestriction.java:115 BuiltinRestriction.AllowlistEntry
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AllowlistEntry {
    apparent_repo_name: &'static str,
    package_prefix: &'static str,
}

/// Java `BuiltinRestriction.allowlistEntry(apparentRepoName, packagePrefix)`.
// Java: com/google/devtools/build/lib/packages/BuiltinRestriction.java:157 BuiltinRestriction.allowlistEntry
pub const fn allowlist_entry(
    apparent_repo_name: &'static str,
    package_prefix: &'static str,
) -> AllowlistEntry {
    AllowlistEntry {
        apparent_repo_name,
        package_prefix,
    }
}

/// The "default" allowlist for restricted APIs added to aid the Java to Starlark migration.
// Java: com/google/devtools/build/lib/packages/BuiltinRestriction.java:36 BuiltinRestriction.INTERNAL_STARLARK_API_ALLOWLIST
pub const INTERNAL_STARLARK_API_ALLOWLIST: &[AllowlistEntry] = &[
    // Testing
    allowlist_entry("", "test"),
    allowlist_entry("", "bazel_internal/test_rules"),
    // BuildInfo
    allowlist_entry("", "tools/build_defs/build_info"),
    allowlist_entry("bazel_tools", "tools/build_defs/build_info"),
    // Android rules
    allowlist_entry("", "bazel_internal/test_rules/cc"),
    allowlist_entry("", "tools/build_defs/android"),
    allowlist_entry("", "third_party/bazel_rules/rules_android"),
    allowlist_entry("rules_android", ""),
    allowlist_entry("build_bazel_rules_android", ""),
    // Apple rules
    allowlist_entry("", "third_party/apple_crosstool"),
    allowlist_entry("", "third_party/cpptoolchains/portable_llvm/build_defs"),
    allowlist_entry("", "third_party/bazel_rules/rules_apple"),
    allowlist_entry("rules_apple", ""),
    allowlist_entry("build_bazel_rules_apple", ""),
    // Cc rules
    allowlist_entry("", "third_party/bazel_rules/rules_cc"),
    allowlist_entry("", "tools/build_defs/cc"),
    allowlist_entry("rules_cc", ""),
    // Java rules
    allowlist_entry("", "third_party/bazel_rules/rules_java/java"),
    allowlist_entry("rules_java", "java"),
    // Rust rules
    allowlist_entry("", "third_party/bazel_rules/rules_rust/rust/private"),
    allowlist_entry("", "third_party/crubit"),
    allowlist_entry("rules_rust", "rust/private"),
    // CUDA rules
    allowlist_entry("", "third_party/gpus/cuda"),
    // Packaging rules
    allowlist_entry("", "tools/build_defs/packaging"),
    // Go rules
    allowlist_entry("", "tools/build_defs/go"),
    // Proto rules
    allowlist_entry("", "third_party/protobuf"),
    allowlist_entry("protobuf", ""),
    allowlist_entry("com_google_protobuf", ""),
    // Shell rules
    allowlist_entry("rules_shell", ""),
];

impl AllowlistEntry {
    // Java: com/google/devtools/build/lib/packages/BuiltinRestriction.java:124 BuiltinRestriction.AllowlistEntry.allows
    fn allows(&self, label: &Label, repo_mapping: &RepositoryMapping) -> bool {
        Self::repos_match(
            self.apparent_repo_name,
            label.get_repository(),
            repo_mapping,
        ) && label
            .get_package_fragment()
            .starts_with(&PathFragment::create(self.package_prefix))
    }

    // Java: com/google/devtools/build/lib/packages/BuiltinRestriction.java:129 BuiltinRestriction.AllowlistEntry.reposMatch
    fn repos_match(
        allowed_name: &str,
        given_name: &RepositoryName,
        repo_mapping: &RepositoryMapping,
    ) -> bool {
        if given_name.is_main() {
            // The main repository may be one of the allowlisted rulesets, in which case we need
            // to fall back to interpreting allowedName as the apparent repo name.
            return allowed_name == blaze_cmdline::repository_name::MAIN.get_name()
                || repo_mapping.get(allowed_name).is_main();
        }
        if *given_name == *blaze_cmdline::repository_name::BAZEL_TOOLS {
            return allowed_name == blaze_cmdline::repository_name::BAZEL_TOOLS.get_name();
        }
        // allowedName is a module name and givenName is a real canonical repo name, so it
        // belongs to any version of that module if and only if it contains <allowedName>+ as a
        // prefix.
        given_name
            .get_name()
            .starts_with(&format!("{allowed_name}+"))
    }
}

/// Returns true if the given label is not within either 1) the builtins repository, or 2) a
/// package or subpackage of an entry in the given allowlist.
// Java: com/google/devtools/build/lib/packages/BuiltinRestriction.java:211 BuiltinRestriction.isNotAllowed
pub fn is_not_allowed(
    label: &Label,
    repo_mapping: &RepositoryMapping,
    allowlist: &[AllowlistEntry],
) -> bool {
    if *label.get_repository() == *blaze_cmdline::repository_name::BUILTINS {
        return false;
    }
    !allowlist.iter().any(|e| e.allows(label, repo_mapping))
}

/// Throws if the innermost Starlark function's file is outside `allowlist` and builtins.
// Java: com/google/devtools/build/lib/packages/BuiltinRestriction.java:169 BuiltinRestriction.failIfCalledOutsideAllowlist
pub fn fail_if_called_outside_allowlist(
    eval: &Evaluator,
    allowlist: &[AllowlistEntry],
) -> starlark::Result<()> {
    let module_context = BazelModuleContext::of_innermost_bzl(eval)
        .expect("NullPointerException: no BazelModuleContext for the innermost .bzl");
    // Java: com/google/devtools/build/lib/packages/BuiltinRestriction.java:199 BuiltinRestriction.failIfLabelOutsideAllowlist
    if is_not_allowed(
        &module_context.label,
        &module_context.repo_mapping,
        allowlist,
    ) {
        return Err(starlark::Error::new_other(anyhow::anyhow!(
            "file '{}' cannot use private API",
            module_context.label.get_canonical_form()
        )));
    }
    Ok(())
}

/// Throws if the call is made outside of the default allowlist or outside of builtins.
// Java: com/google/devtools/build/lib/packages/BuiltinRestriction.java:181 BuiltinRestriction.failIfCalledOutsideDefaultAllowlist
pub fn fail_if_called_outside_default_allowlist(eval: &Evaluator) -> starlark::Result<()> {
    fail_if_called_outside_allowlist(eval, INTERNAL_STARLARK_API_ALLOWLIST)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn module_repo_prefix_matches() {
        let mapping = blaze_cmdline::repository_mapping::EMPTY.clone();
        let l = Label::parse_canonical_unchecked("@@rules_cc+//cc/private:foo.bzl");
        assert!(!is_not_allowed(
            &l,
            &mapping,
            INTERNAL_STARLARK_API_ALLOWLIST
        ));
        let l = Label::parse_canonical_unchecked("@@other+//cc:foo.bzl");
        assert!(is_not_allowed(
            &l,
            &mapping,
            INTERNAL_STARLARK_API_ALLOWLIST
        ));
        let l = Label::parse_canonical_unchecked("@@_builtins//:foo.bzl");
        assert!(!is_not_allowed(&l, &mapping, &[]));
    }
}
