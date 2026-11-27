//! Port of `lib/rules/cpp/SolibSymlinkAction.java` (analysis-time parts), with the pieces of
//! `Link.SHARED_LIBRARY_FILETYPES` (`CppFileTypes`, `FileType`) and `Actions.escapedPath` it uses.

use std::any::Any;

use blaze_cmdline::Label;
use blaze_collect::{NestedSetBuilder, Order};
use blaze_util::{Fingerprint, check_argument};
use blaze_vfs::PathFragment;
use indexmap::IndexMap;

use crate::actions::action::{AbstractAction, Action, ComputeKeyError, InputMetadataProvider};
use crate::actions::action_key_context::ActionKeyContext;
use crate::actions::action_owner::ActionOwner;
use crate::actions::artifact::Artifact;
use crate::actions::fingerprint_util::FingerprintPathExt;
use crate::actions::platform_info::{PlatformInfoRef, empty_platform_info};

/// Creates mangled symlinks in the solib directory for all shared libraries. For shared libraries
/// that have potential to contain a SONAME field, create a link to the shared library parent
/// directory instead - so that the name of the library file is preserved.
///
/// Such symlinks are used by the linker to ensure that all rpath entries can be specified
/// relative to the $ORIGIN.
// Java: com/google/devtools/build/lib/rules/cpp/SolibSymlinkAction.java:63 SolibSymlinkAction
#[derive(Debug)]
pub struct SolibSymlinkAction {
    base: AbstractAction,
    symlink: Artifact,
}

impl SolibSymlinkAction {
    // Java: com/google/devtools/build/lib/rules/cpp/SolibSymlinkAction.java:66 SolibSymlinkAction.<init>
    fn new(
        owner: ActionOwner,
        primary_input: Artifact,
        primary_output: Artifact,
    ) -> SolibSymlinkAction {
        let base = AbstractAction::new(
            owner,
            NestedSetBuilder::create(Order::STABLE_ORDER, [primary_input.clone()]),
            [primary_output.clone()],
        );
        check_argument!(shared_library_filetypes_matches(
            primary_input.get_filename()
        ));
        SolibSymlinkAction {
            base,
            symlink: primary_output,
        }
    }

    /// The symlink artifact (the primary output).
    pub fn get_symlink(&self) -> &Artifact {
        &self.symlink
    }
}

/// `getDynamicLibrarySymlink(actionConstructionContext, solibDir, library, preserveName,
/// prefixConsumer)`: the root-relative path of the mangled symlink.
///
/// PORT: Java takes the `ActionConstructionContext` and also creates the artifact and registers
/// the action. Here `label` is `getOwner().getLabel()` and `configuration_mnemonic` is
/// `getConfiguration().getMnemonic()`; the caller creates the artifact with
/// `getShareableArtifact(path, getBinDirectory())` and the action with
/// [`get_dynamic_library_symlink_internal`].
// Java: com/google/devtools/build/lib/rules/cpp/SolibSymlinkAction.java:158 SolibSymlinkAction.getDynamicLibrarySymlink
pub fn get_dynamic_library_symlink_path(
    label: &Label,
    solib_dir: &str,
    configuration_mnemonic: &str,
    library: &Artifact,
    preserve_name: bool,
    prefix_consumer: bool,
) -> PathFragment {
    get_mangled_name(
        label,
        solib_dir,
        configuration_mnemonic,
        &library.get_root_relative_path(),
        preserve_name,
        prefix_consumer,
    )
}

/// `getDynamicLibrarySymlink(actionConstructionContext, solibDir, library, path)`: the
/// root-relative path of the user-specified symlink (see
/// [`get_dynamic_library_symlink_path`] for the context-based parts).
// Java: com/google/devtools/build/lib/rules/cpp/SolibSymlinkAction.java:189 SolibSymlinkAction.getDynamicLibrarySymlink
pub fn get_dynamic_library_symlink_path_for_path(
    solib_dir: &str,
    library: &Artifact,
    path: &PathFragment,
) -> PathFragment {
    check_argument!(shared_library_filetypes_matches(library.get_filename()));
    check_argument!(shared_library_filetypes_matches(path.get_base_name()));
    check_argument!(
        !library
            .get_root_relative_path()
            .get_path_string()
            .starts_with("_solib_")
    );

    let solib_dir_path = PathFragment::create(solib_dir);
    solib_dir_path.get_relative(path)
}

/// Version of [`get_dynamic_library_symlink_path`] for the special case of C++ runtime
/// libraries. These are handled differently than other libraries: neither their names nor
/// directories are mangled, i.e. libstdc++.so.6 is symlinked from _solib_[arch]/libstdc++.so.6
// Java: com/google/devtools/build/lib/rules/cpp/SolibSymlinkAction.java:209 SolibSymlinkAction.getCppRuntimeSymlink
// PORT: returns the symlink path; the caller creates the artifact and the action as for
// `get_dynamic_library_symlink_path`.
pub fn get_cpp_runtime_symlink_path(
    library: &Artifact,
    toolchain_provided_solib_dir: &str,
    solib_dir_override: Option<&str>,
) -> PathFragment {
    let solib_dir =
        PathFragment::create(solib_dir_override.unwrap_or(toolchain_provided_solib_dir));
    solib_dir.get_relative_str(library.get_root_relative_path().get_base_name())
}

/// Internal implementation that takes a pre-determined symlink; supports both the generic
/// `getDynamicLibrarySymlink` and the specialized `getCppRuntimeSymlink`.
///
/// PORT: Java creates `symlink` itself (`getShareableArtifact(symlinkName, getBinDirectory())`)
/// and registers the returned action; the caller does both here. `action_owner` is
/// `getActionOwner()`.
// Java: com/google/devtools/build/lib/rules/cpp/SolibSymlinkAction.java:228 SolibSymlinkAction.getDynamicLibrarySymlinkInternal
pub fn get_dynamic_library_symlink_internal(
    action_owner: ActionOwner,
    library: Artifact,
    symlink: Artifact,
) -> SolibSymlinkAction {
    check_argument!(
        shared_library_filetypes_matches(library.get_filename()),
        "Library '{}' does not match expected filetype",
        library.get_filename()
    );
    check_argument!(
        !library
            .get_root_relative_path()
            .get_path_string()
            .starts_with("_solib_")
    );
    SolibSymlinkAction::new(action_owner, library, symlink)
}

// Java: com/google/devtools/build/lib/rules/cpp/SolibSymlinkAction.java:247 SolibSymlinkAction.MAX_FILENAME_LENGTH
pub const MAX_FILENAME_LENGTH: usize = 255;

// Java: com/google/devtools/build/lib/rules/cpp/SolibSymlinkAction.java:249 SolibSymlinkAction.maybeHashPreserveExtension
fn maybe_hash_preserve_extension(filename: &str) -> String {
    // Invariant: `filename` is an internal string (see `actions::fingerprint_util`). Java's
    // `length()` counts its chars, and `Hashing.sha256().hashString(filename, UTF_8)` hashes its
    // UTF-8, as `get_hex_digest` does.
    if filename.chars().count() <= MAX_FILENAME_LENGTH {
        filename.to_string()
    } else {
        let hashed_name = Fingerprint::get_hex_digest(filename);
        // Guava `Files.getFileExtension`.
        let extension = match filename.rfind('.') {
            None => "",
            Some(i) => &filename[i + 1..],
        };
        if extension.is_empty() {
            hashed_name
        } else {
            format!("{}.{}", hashed_name, extension)
        }
    }
}

/// Returns the name of the symlink that will be created for a library, given its name.
///
/// Returns a root relative path name.
// Java: com/google/devtools/build/lib/rules/cpp/SolibSymlinkAction.java:273 SolibSymlinkAction.getMangledName
pub fn get_mangled_name(
    label: &Label,
    solib_dir: &str,
    mnemonic: &str,
    library_path: &PathFragment,
    preserve_name: bool,
    prefix_consumer: bool,
) -> PathFragment {
    let escaped_rule_path = escaped_path(&format!("_{}", label));
    let soname = get_dynamic_library_soname(library_path, preserve_name, mnemonic);
    let solib_dir_path = PathFragment::create(solib_dir);
    if preserve_name {
        let parent = library_path
            .get_parent_directory()
            .expect("library path has a parent directory");
        let escaped_library_path = escaped_path(&format!("_{}", parent.get_path_string()));
        let escaped_full_path = if prefix_consumer {
            format!("{}__{}", escaped_rule_path, escaped_library_path)
        } else {
            escaped_library_path
        };
        let mangled_dir =
            solib_dir_path.get_relative_str(&maybe_hash_preserve_extension(&escaped_full_path));
        mangled_dir.get_relative_str(&soname)
    } else {
        let filename = if prefix_consumer {
            format!("{}__{}", escaped_rule_path, soname)
        } else {
            soname
        };
        solib_dir_path.get_relative_str(&maybe_hash_preserve_extension(&filename))
    }
}

/// Compute the SONAME to use for a dynamic library. This name is basically the name of the shared
/// library in its final symlinked location.
///
/// `mnemonic` is the output directory mnemonic, to be mangled in for nondefault configurations.
// Java: com/google/devtools/build/lib/rules/cpp/SolibSymlinkAction.java:306 SolibSymlinkAction.getDynamicLibrarySoname
pub fn get_dynamic_library_soname(
    library_path: &PathFragment,
    preserve_name: bool,
    mnemonic: &str,
) -> String {
    if preserve_name {
        library_path.get_base_name().to_string()
    } else {
        let mut mnemonic_mangling = String::new();
        if let Some(i) = mnemonic.find("ST-") {
            mnemonic_mangling = format!("{}_", &mnemonic[i..]);
        }
        format!(
            "lib{}{}",
            mnemonic_mangling,
            escaped_path(library_path.get_path_string())
        )
    }
}

/// Returns the escaped name for a given relative path as a string. This takes a short relative
/// path and turns it into a string suitable for use as a filename. Invalid filename characters
/// are escaped with an '_' + a single character token.
// Java: com/google/devtools/build/lib/actions/Actions.java:376 Actions.escapedPath
// PORT: `Actions` is not ported in the core yet; its `PATH_ESCAPER` (Actions.java:45) is here.
pub fn escaped_path(path: &str) -> String {
    let mut out = String::with_capacity(path.len());
    for c in path.chars() {
        match c {
            '_' => out.push_str("_U"),
            '/' => out.push_str("_S"),
            '\\' => out.push_str("_B"),
            ':' => out.push_str("_C"),
            '@' => out.push_str("_A"),
            c => out.push(c),
        }
    }
    out
}

/// `Link.SHARED_LIBRARY_FILETYPES.matches(filename)`: `CppFileTypes.SHARED_LIBRARY`,
/// `VERSIONED_SHARED_LIBRARY`, `INTERFACE_SHARED_LIBRARY` and `FileType.NO_EXTENSION`.
// Java: com/google/devtools/build/lib/rules/cpp/Link.java:39 Link.SHARED_LIBRARY_FILETYPES
// PORT: `FileType.hasExtension` is case-insensitive only on Windows (skipped).
pub fn shared_library_filetypes_matches(path: &str) -> bool {
    // Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:243 CppFileTypes.SHARED_LIBRARY
    const SHARED_LIBRARY: [&str; 7] = [".so", ".dylib", ".dll", ".pyd", ".wasm", ".tgt", ".vpi"];
    // Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:247 CppFileTypes.INTERFACE_SHARED_LIBRARY
    const INTERFACE_SHARED_LIBRARY: [&str; 4] = [".ifso", ".tbd", ".lib", ".dll.a"];
    SHARED_LIBRARY.iter().any(|ext| path.ends_with(ext))
        || is_versioned_shared_library(path)
        || INTERFACE_SHARED_LIBRARY
            .iter()
            .any(|ext| path.ends_with(ext))
        || is_no_extension(path)
}

// Java: com/google/devtools/build/lib/util/FileType.java:34 FileType.NO_EXTENSION
fn is_no_extension(path: &str) -> bool {
    let start = path.rfind('/').map(|i| i + 1).unwrap_or(0);
    !path[start..].contains('.')
}

/// Matches shared libraries with version names in the extension, i.e. libmylib.so.2 or
/// libmylib.so.2.10 or libmylib.so.1a_b35: `^.+\.((so)|(dylib))(\.\d\w*)+$`.
// Java: com/google/devtools/build/lib/rules/cpp/CppFileTypes.java:258 CppFileTypes.VERSIONED_SHARED_LIBRARY
fn is_versioned_shared_library(path: &str) -> bool {
    // Because regex matching can be slow, we first do a quick check for ".so." and ".dylib."
    // substring before risking the full-on regex match. This should eliminate the performance
    // hit on practically every non-qualifying file type.
    if !path.contains(".so.") && !path.contains(".dylib.") {
        return false;
    }
    // PORT: the regex (no regex crate here) as a direct matcher: some split point leaves a
    // non-empty prefix of non-line-terminators (`.+`), then `.so`/`.dylib`, then one or more
    // `\.\d\w*` groups (`\d` = [0-9], `\w` = [a-zA-Z_0-9]).
    let is_line_terminator =
        |c: char| matches!(c, '\n' | '\r' | '\u{0085}' | '\u{2028}' | '\u{2029}');
    for (i, _) in path.char_indices().skip(1) {
        if path[..i].chars().any(is_line_terminator) {
            return false;
        }
        let rest = &path[i..];
        let after = if let Some(r) = rest.strip_prefix(".so") {
            r
        } else if let Some(r) = rest.strip_prefix(".dylib") {
            r
        } else {
            continue;
        };
        if is_version_groups(after) {
            return true;
        }
    }
    false
}

/// `(\.\d\w*)+` matching all of `s`.
fn is_version_groups(s: &str) -> bool {
    let Some(body) = s.strip_prefix('.') else {
        return false;
    };
    body.split('.').all(|group| {
        let mut chars = group.chars();
        matches!(chars.next(), Some(c) if c.is_ascii_digit())
            && chars.all(|c| c.is_ascii_alphanumeric() || c == '_')
    })
}

impl Action for SolibSymlinkAction {
    fn abstract_action(&self) -> &AbstractAction {
        &self.base
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    // Java: com/google/devtools/build/lib/rules/cpp/SolibSymlinkAction.java:322 SolibSymlinkAction.mayInsensitivelyPropagateInputs
    fn may_insensitively_propagate_inputs(&self) -> bool {
        true
    }

    // Java: com/google/devtools/build/lib/rules/cpp/SolibSymlinkAction.java:132 SolibSymlinkAction.getMnemonic
    fn get_mnemonic(&self) -> String {
        "SolibSymlink".to_string()
    }

    // Java: com/google/devtools/build/lib/rules/cpp/SolibSymlinkAction.java:123 SolibSymlinkAction.computeKey
    fn compute_key(
        &self,
        _action_key_context: &ActionKeyContext,
        _input_metadata_provider: Option<&dyn InputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        fp.add_path_fragment(self.symlink.get_exec_path());
        let primary_input = self
            .get_primary_input()
            .expect("SolibSymlinkAction has a primary input");
        fp.add_path_fragment(primary_input.get_exec_path());
        Ok(())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/SolibSymlinkAction.java:138 SolibSymlinkAction.getRawProgressMessage
    fn get_raw_progress_message(&self) -> Option<String> {
        None
    }

    // Java: com/google/devtools/build/lib/rules/cpp/SolibSymlinkAction.java:327 SolibSymlinkAction.getExecutionPlatform
    fn get_execution_platform(&self) -> Option<PlatformInfoRef> {
        Some(empty_platform_info())
    }

    // Java: com/google/devtools/build/lib/rules/cpp/SolibSymlinkAction.java:332 SolibSymlinkAction.getExecProperties
    fn get_exec_properties(&self) -> IndexMap<String, String> {
        // SolibSymlinkAction is platform agnostic.
        IndexMap::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn shared_library_file_types() {
        assert!(shared_library_filetypes_matches("libfoo.so"));
        assert!(shared_library_filetypes_matches("libfoo.so.1.0"));
        assert!(shared_library_filetypes_matches("libfoo.so.1a_b35"));
        assert!(shared_library_filetypes_matches("libfoo.ifso"));
        assert!(shared_library_filetypes_matches("dir.x/libfoo"));
        assert!(!shared_library_filetypes_matches("libfoo.so.x"));
        assert!(!shared_library_filetypes_matches("libfoo.a"));
        assert!(!shared_library_filetypes_matches(".so.1"));
    }

    #[test]
    fn escaped_path_escapes() {
        assert_eq!(escaped_path("_@a/b:c\\d"), "_U_Aa_Sb_Cc_Bd");
    }
}
