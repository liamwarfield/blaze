//! The include-validation logic of `com.google.devtools.build.lib.rules.cpp.CppCompileAction`, as
//! pure functions over the action's artifact sets.

use std::collections::HashSet;

use blaze_collect::NestedSet;
use blaze_vfs::PathFragment;

use crate::actions::{ActionExecutionException, ArtifactRef};
use crate::include_problems::IncludeProblems;

/// Java: `include.isFileType(CppFileTypes.CPP_MODULE)`.
// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:280 CppFileTypes.CPP_MODULE
fn is_cpp_module(include: &ArtifactRef) -> bool {
    let path = include.get_exec_path_string();
    [".pcm", ".gcm", ".ifc"]
        .iter()
        .any(|ext| path.ends_with(ext))
}

/// Java: `include.isFileType(CppFileTypes.OBJC_MODULE_MAP)`.
// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:281 CppFileTypes.OBJC_MODULE_MAP
fn is_objc_module_map(include: &ArtifactRef) -> bool {
    include.get_exec_path_string().ends_with("module.modulemap")
}

// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:919 FileSystemUtils.startsWithAny
fn starts_with_any(path: &PathFragment, prefixes: &[PathFragment]) -> bool {
    prefixes.iter().any(|prefix| path.starts_with(prefix))
}

// Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1117 CppCompileAction.validateInclude
pub fn validate_include(
    allowed_includes: &HashSet<ArtifactRef>,
    ignore_dirs: &[PathFragment],
    include: &ArtifactRef,
) -> bool {
    // Only declared modules are added to an action and so they are always valid.
    is_cpp_module(include)
        // TODO(b/145253507): Exclude objc module maps from check, due to bad interaction with
        // local_objc_modules feature.
        || is_objc_module_map(include)
        // It's a declared include/
        || allowed_includes.contains(include)
        // Ignore headers from built-in include directories.
        || starts_with_any(&include.get_exec_path(), ignore_dirs)
}

/// Enforce that the includes actually visited during the compile were properly declared in the
/// rules.
///
/// `ignore_dirs` is `getBuiltInIncludeDirectories()` under `--strict_system_includes`, otherwise
/// [`get_validation_ignored_dirs`].
// Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1152 CppCompileAction.validateInclusions
pub fn validate_inclusions(
    needs_include_validation: bool,
    mandatory_inputs: &NestedSet<ArtifactRef>,
    declared_include_srcs: &NestedSet<ArtifactRef>,
    additional_prunable_headers: &NestedSet<ArtifactRef>,
    ignore_dirs: &[PathFragment],
    inputs_for_validation: &NestedSet<ArtifactRef>,
    owner_label: &str,
    source_file: &ArtifactRef,
) -> Result<(), ActionExecutionException> {
    if !needs_include_validation {
        return Ok(());
    }
    let mut errors = IncludeProblems::new();
    let mut allowed_includes: HashSet<ArtifactRef> = HashSet::new();
    allowed_includes.extend(mandatory_inputs.to_list().iter().cloned());
    allowed_includes.extend(declared_include_srcs.to_list().iter().cloned());
    allowed_includes.extend(additional_prunable_headers.to_list().iter().cloned());

    for input in inputs_for_validation.to_list().iter() {
        if !validate_include(&allowed_includes, ignore_dirs, input) {
            errors.add(input.get_exec_path().get_path_string());
        }
    }
    errors.assert_problem_free(&format!(
        "undeclared inclusion(s) in rule '{}':\nthis rule is missing dependency declarations for \
         the following files included by '{}':",
        owner_label,
        source_file.pretty_print()
    ))
}

/// The built-in include directories followed by the compilation context's system and external
/// include directories.
// Java: com/google/devtools/build/lib/rules/cpp/CppCompileAction.java:1187 CppCompileAction.getValidationIgnoredDirs
pub fn get_validation_ignored_dirs(
    cxx_system_include_dirs: &[PathFragment],
    system_include_dirs: &[PathFragment],
    external_include_dirs: &[PathFragment],
) -> Vec<PathFragment> {
    cxx_system_include_dirs
        .iter()
        .chain(system_include_dirs)
        .chain(external_include_dirs)
        .cloned()
        .collect()
}
