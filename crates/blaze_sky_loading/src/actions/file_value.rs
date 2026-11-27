//! Port of `FileValue`: a file's existence, type and real path, fully accounting for symlinks.
//!
//! Java: com/google/devtools/build/lib/actions/FileValue.java. The Java interface and its
//! implementations become one enum, one variant per class.

use std::fmt;

use blaze_vfs::{PathFragment, RootedPath};

use crate::actions::file_state_value::{FileStateType, FileStateValue};

/// A value that corresponds to a file (or directory or symlink or non-existent file), fully
/// accounting for symlinks (e.g. proper dependencies on ancestor symlinks so as to be incrementally
/// correct). Anything in Skyframe that cares about the fully resolved path of a file (e.g.
/// anything that cares about the contents of a file) should have a dependency on the corresponding
/// FileValue.
///
/// Note that since a FileValue doesn't necessarily store its corresponding SkyKey, it's possible
/// for the FileValues for two different paths to be the same.
// Java: com/google/devtools/build/lib/actions/FileValue.java:49 FileValue
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum FileValue {
    /// A path whose fully resolved path is the same as the requested path. Java: the
    /// `FileStateValue` itself (a `RegularFileValue`).
    // Java: com/google/devtools/build/lib/actions/FileValue.java:226 FileValue.RegularFileValue
    Regular(FileStateValue),
    /// A non-symlink with an ancestor symlink whose resolution traversed a symlink chain caused by
    /// a symlink pointing to its own ancestor, but which eventually points to a real file.
    // Java: com/google/devtools/build/lib/actions/FileValue.java:255 FileValue.DifferentRealPathFileValueWithUnboundedAncestorExpansion
    DifferentRealPathWithUnboundedAncestorExpansion {
        real_rooted_path: RootedPath,
        real_file_state_value: FileStateValue,
        logical_chain_during_resolution: Vec<RootedPath>,
        path_to_unbounded_ancestor_symlink_expansion_chain: Vec<RootedPath>,
        unbounded_ancestor_symlink_expansion_chain: Vec<RootedPath>,
    },
    /// A path whose fully resolved path differs from the requested path, but which is not
    /// itself a symlink.
    // Java: com/google/devtools/build/lib/actions/FileValue.java:332 FileValue.DifferentRealPathFileValueWithStoredChain
    DifferentRealPathWithStoredChain {
        real_rooted_path: RootedPath,
        real_file_state_value: FileStateValue,
        logical_chain_during_resolution: Vec<RootedPath>,
    },
    /// Same as `DifferentRealPathWithStoredChain`, without the logical chain.
    // Java: com/google/devtools/build/lib/actions/FileValue.java:406 FileValue.DifferentRealPathFileValueWithoutStoredChain
    DifferentRealPathWithoutStoredChain {
        real_rooted_path: RootedPath,
        real_file_state_value: FileStateValue,
    },
    /// A symlink whose resolution traversed a symlink chain caused by a symlink pointing to its
    /// own ancestor.
    // Java: com/google/devtools/build/lib/actions/FileValue.java:474 FileValue.SymlinkFileValueWithUnboundedAncestorExpansion
    SymlinkWithUnboundedAncestorExpansion {
        real_rooted_path: RootedPath,
        real_file_state_value: FileStateValue,
        logical_chain_during_resolution: Vec<RootedPath>,
        link_target: PathFragment,
        path_to_unbounded_ancestor_symlink_expansion_chain: Vec<RootedPath>,
        unbounded_ancestor_symlink_expansion_chain: Vec<RootedPath>,
    },
    /// A path that is itself a symlink.
    // Java: com/google/devtools/build/lib/actions/FileValue.java:551 FileValue.SymlinkFileValueWithStoredChain
    SymlinkWithStoredChain {
        real_rooted_path: RootedPath,
        real_file_state_value: FileStateValue,
        logical_chain_during_resolution: Vec<RootedPath>,
        link_target: PathFragment,
    },
    /// Same as `SymlinkWithStoredChain`, without the logical chain.
    // Java: com/google/devtools/build/lib/actions/FileValue.java:607 FileValue.SymlinkFileValueWithoutStoredChain
    SymlinkWithoutStoredChain {
        real_rooted_path: RootedPath,
        real_file_state_value: FileStateValue,
        link_target: PathFragment,
    },
}

impl FileValue {
    // Java: com/google/devtools/build/lib/actions/FileValue.java:50 FileValue.exists
    pub fn exists(&self) -> bool {
        self.real_file_state_value().get_type() != FileStateType::Nonexistent
    }

    /// Returns true if the original path is a symlink; the target path can never be a symlink.
    // Java: com/google/devtools/build/lib/actions/FileValue.java:55 FileValue.isSymlink
    // Java: com/google/devtools/build/lib/actions/FileValue.java:564 FileValue.SymlinkFileValueWithStoredChain.isSymlink
    // Java: com/google/devtools/build/lib/actions/FileValue.java:618 FileValue.SymlinkFileValueWithoutStoredChain.isSymlink
    pub fn is_symlink(&self) -> bool {
        matches!(
            self,
            FileValue::SymlinkWithUnboundedAncestorExpansion { .. }
                | FileValue::SymlinkWithStoredChain { .. }
                | FileValue::SymlinkWithoutStoredChain { .. }
        )
    }

    /// Returns true if this value corresponds to a file or symlink to an existing regular or
    /// special file. If so, its parent directory is guaranteed to exist.
    // Java: com/google/devtools/build/lib/actions/FileValue.java:63 FileValue.isFile
    pub fn is_file(&self) -> bool {
        let t = self.real_file_state_value().get_type();
        t == FileStateType::RegularFile || t == FileStateType::SpecialFile
    }

    /// Returns true if this value corresponds to a special file or symlink to a special file. If
    /// so, its parent directory is guaranteed to exist.
    // Java: com/google/devtools/build/lib/actions/FileValue.java:72 FileValue.isSpecialFile
    pub fn is_special_file(&self) -> bool {
        self.real_file_state_value().get_type() == FileStateType::SpecialFile
    }

    /// Returns true if the file is a directory or a symlink to an existing directory. If so, its
    /// parent directory is guaranteed to exist.
    // Java: com/google/devtools/build/lib/actions/FileValue.java:80 FileValue.isDirectory
    pub fn is_directory(&self) -> bool {
        self.real_file_state_value().get_type() == FileStateType::Directory
    }

    /// If `!is_file() && exists()`, returns an ordered list of the [`RootedPath`]s that were
    /// considered when determining `real_rooted_path()`.
    ///
    /// This information is used to detect unbounded symlink expansions.
    ///
    /// As a memory optimization, we don't store this information when `is_file() || !exists()`
    /// -- this information is only needed for resolving ancestors, and an existing file or a
    /// non-existent directory has no descendants, by definition. Panics for those values.
    // Java: com/google/devtools/build/lib/actions/FileValue.java:94 FileValue.logicalChainDuringResolution
    // Java: com/google/devtools/build/lib/actions/FileValue.java:227 FileValue.RegularFileValue.logicalChainDuringResolution
    // Java: com/google/devtools/build/lib/actions/FileValue.java:357 FileValue.DifferentRealPathFileValueWithStoredChain.logicalChainDuringResolution
    // Java: com/google/devtools/build/lib/actions/FileValue.java:427 FileValue.DifferentRealPathFileValueWithoutStoredChain.logicalChainDuringResolution
    pub fn logical_chain_during_resolution(
        &self,
        initial_rooted_path: &RootedPath,
    ) -> Vec<RootedPath> {
        match self {
            FileValue::Regular(_) => vec![initial_rooted_path.clone()],
            FileValue::DifferentRealPathWithUnboundedAncestorExpansion {
                logical_chain_during_resolution,
                ..
            }
            | FileValue::DifferentRealPathWithStoredChain {
                logical_chain_during_resolution,
                ..
            }
            | FileValue::SymlinkWithUnboundedAncestorExpansion {
                logical_chain_during_resolution,
                ..
            }
            | FileValue::SymlinkWithStoredChain {
                logical_chain_during_resolution,
                ..
            } => logical_chain_during_resolution.clone(),
            FileValue::DifferentRealPathWithoutStoredChain { .. }
            | FileValue::SymlinkWithoutStoredChain { .. } => {
                panic!("IllegalStateException: {}", self)
            }
        }
    }

    /// If a symlink pointing back to its own ancestor was encountered during the resolution of
    /// this value, returns the path to it. Otherwise, returns `None`.
    // Java: com/google/devtools/build/lib/actions/FileValue.java:100 FileValue.pathToUnboundedAncestorSymlinkExpansionChain
    // Java: com/google/devtools/build/lib/actions/FileValue.java:232 FileValue.RegularFileValue.pathToUnboundedAncestorSymlinkExpansionChain
    // Java: com/google/devtools/build/lib/actions/FileValue.java:273 FileValue.DifferentRealPathFileValueWithUnboundedAncestorExpansion.pathToUnboundedAncestorSymlinkExpansionChain
    // Java: com/google/devtools/build/lib/actions/FileValue.java:362 FileValue.DifferentRealPathFileValueWithStoredChain.pathToUnboundedAncestorSymlinkExpansionChain
    // Java: com/google/devtools/build/lib/actions/FileValue.java:432 FileValue.DifferentRealPathFileValueWithoutStoredChain.pathToUnboundedAncestorSymlinkExpansionChain
    // Java: com/google/devtools/build/lib/actions/FileValue.java:493 FileValue.SymlinkFileValueWithUnboundedAncestorExpansion.pathToUnboundedAncestorSymlinkExpansionChain
    pub fn path_to_unbounded_ancestor_symlink_expansion_chain(&self) -> Option<&[RootedPath]> {
        match self {
            FileValue::DifferentRealPathWithUnboundedAncestorExpansion {
                path_to_unbounded_ancestor_symlink_expansion_chain,
                ..
            }
            | FileValue::SymlinkWithUnboundedAncestorExpansion {
                path_to_unbounded_ancestor_symlink_expansion_chain,
                ..
            } => Some(path_to_unbounded_ancestor_symlink_expansion_chain),
            _ => None,
        }
    }

    /// If a symlink pointing back to its own ancestor was encountered during the resolution of
    /// this value, returns the symlinks in the cycle. Otherwise, returns `None`.
    ///
    /// If you're about to attempt a recursive directory traversal starting at the original path,
    /// you should first use this method to check if there's an unbounded ancestor symlink
    /// expansion. If there is, you should either error out and give up, or you should perform the
    /// traversal carefully (e.g. with a visited set) lest the traversal never terminate.
    // Java: com/google/devtools/build/lib/actions/FileValue.java:111 FileValue.unboundedAncestorSymlinkExpansionChain
    // Java: com/google/devtools/build/lib/actions/FileValue.java:238 FileValue.RegularFileValue.unboundedAncestorSymlinkExpansionChain
    // Java: com/google/devtools/build/lib/actions/FileValue.java:278 FileValue.DifferentRealPathFileValueWithUnboundedAncestorExpansion.unboundedAncestorSymlinkExpansionChain
    // Java: com/google/devtools/build/lib/actions/FileValue.java:367 FileValue.DifferentRealPathFileValueWithStoredChain.unboundedAncestorSymlinkExpansionChain
    // Java: com/google/devtools/build/lib/actions/FileValue.java:437 FileValue.DifferentRealPathFileValueWithoutStoredChain.unboundedAncestorSymlinkExpansionChain
    // Java: com/google/devtools/build/lib/actions/FileValue.java:498 FileValue.SymlinkFileValueWithUnboundedAncestorExpansion.unboundedAncestorSymlinkExpansionChain
    pub fn unbounded_ancestor_symlink_expansion_chain(&self) -> Option<&[RootedPath]> {
        match self {
            FileValue::DifferentRealPathWithUnboundedAncestorExpansion {
                unbounded_ancestor_symlink_expansion_chain,
                ..
            }
            | FileValue::SymlinkWithUnboundedAncestorExpansion {
                unbounded_ancestor_symlink_expansion_chain,
                ..
            } => Some(unbounded_ancestor_symlink_expansion_chain),
            _ => None,
        }
    }

    /// Returns the real rooted path of the file, taking ancestor symlinks into account. For
    /// example, the rooted path ['root']/['a/b'] is really ['root']/['c/b'] if 'a' is a symlink
    /// to 'c'. Note that ancestor symlinks outside the root boundary are not taken into
    /// consideration.
    // Java: com/google/devtools/build/lib/actions/FileValue.java:118 FileValue.realRootedPath
    // Java: com/google/devtools/build/lib/actions/FileValue.java:244 FileValue.RegularFileValue.realRootedPath
    // Java: com/google/devtools/build/lib/actions/FileValue.java:347 FileValue.DifferentRealPathFileValueWithStoredChain.realRootedPath
    // Java: com/google/devtools/build/lib/actions/FileValue.java:417 FileValue.DifferentRealPathFileValueWithoutStoredChain.realRootedPath
    pub fn real_rooted_path(&self, initial_rooted_path: &RootedPath) -> RootedPath {
        match self {
            FileValue::Regular(_) => initial_rooted_path.clone(),
            FileValue::DifferentRealPathWithUnboundedAncestorExpansion {
                real_rooted_path, ..
            }
            | FileValue::DifferentRealPathWithStoredChain {
                real_rooted_path, ..
            }
            | FileValue::DifferentRealPathWithoutStoredChain {
                real_rooted_path, ..
            }
            | FileValue::SymlinkWithUnboundedAncestorExpansion {
                real_rooted_path, ..
            }
            | FileValue::SymlinkWithStoredChain {
                real_rooted_path, ..
            }
            | FileValue::SymlinkWithoutStoredChain {
                real_rooted_path, ..
            } => real_rooted_path.clone(),
        }
    }

    // Java: com/google/devtools/build/lib/actions/FileValue.java:120 FileValue.realFileStateValue
    // Java: com/google/devtools/build/lib/actions/FileValue.java:352 FileValue.DifferentRealPathFileValueWithStoredChain.realFileStateValue
    // Java: com/google/devtools/build/lib/actions/FileValue.java:422 FileValue.DifferentRealPathFileValueWithoutStoredChain.realFileStateValue
    pub fn real_file_state_value(&self) -> &FileStateValue {
        match self {
            FileValue::Regular(state) => state,
            FileValue::DifferentRealPathWithUnboundedAncestorExpansion {
                real_file_state_value,
                ..
            }
            | FileValue::DifferentRealPathWithStoredChain {
                real_file_state_value,
                ..
            }
            | FileValue::DifferentRealPathWithoutStoredChain {
                real_file_state_value,
                ..
            }
            | FileValue::SymlinkWithUnboundedAncestorExpansion {
                real_file_state_value,
                ..
            }
            | FileValue::SymlinkWithStoredChain {
                real_file_state_value,
                ..
            }
            | FileValue::SymlinkWithoutStoredChain {
                real_file_state_value,
                ..
            } => real_file_state_value,
        }
    }

    /// Returns the unresolved link target if [`FileValue::is_symlink`]; panics otherwise.
    ///
    /// This is useful if the caller wants to, for example, duplicate a relative symlink. An
    /// actual example could be a build rule that copies a set of input files to the output
    /// directory, but upon encountering symbolic links it can decide between copying or
    /// following them.
    // Java: com/google/devtools/build/lib/actions/FileValue.java:129 FileValue.getUnresolvedLinkTarget
    // Java: com/google/devtools/build/lib/actions/FileValue.java:569 FileValue.SymlinkFileValueWithStoredChain.getUnresolvedLinkTarget
    // Java: com/google/devtools/build/lib/actions/FileValue.java:623 FileValue.SymlinkFileValueWithoutStoredChain.getUnresolvedLinkTarget
    pub fn get_unresolved_link_target(&self) -> &PathFragment {
        match self {
            FileValue::SymlinkWithUnboundedAncestorExpansion { link_target, .. }
            | FileValue::SymlinkWithStoredChain { link_target, .. }
            | FileValue::SymlinkWithoutStoredChain { link_target, .. } => link_target,
            _ => panic!("IllegalStateException: {}", self),
        }
    }

    // Java: com/google/devtools/build/lib/actions/FileValue.java:133 FileValue.getSize
    pub fn get_size(&self) -> i64 {
        blaze_util::check_state!(self.is_file(), "{}", self);
        self.real_file_state_value().get_size()
    }

    // Java: com/google/devtools/build/lib/actions/FileValue.java:139 FileValue.getDigest
    pub fn get_digest(&self) -> Option<&[u8]> {
        blaze_util::check_state!(self.is_file(), "{}", self);
        self.real_file_state_value().get_digest()
    }

    /// Only intended to be used by FileFunction. Should not be used for symlink cycles.
    // Java: com/google/devtools/build/lib/actions/FileValue.java:154 FileValue.value
    pub fn value(
        logical_chain_during_resolution: Vec<RootedPath>,
        path_to_unbounded_ancestor_symlink_expansion_chain: Option<Vec<RootedPath>>,
        unbounded_ancestor_symlink_expansion_chain: Option<Vec<RootedPath>>,
        original_rooted_path: &RootedPath,
        file_state_value_from_ancestors: FileStateValue,
        real_rooted_path: RootedPath,
        real_file_state_value: FileStateValue,
    ) -> FileValue {
        if *original_rooted_path == real_rooted_path {
            blaze_util::check_state!(
                file_state_value_from_ancestors.get_type() != FileStateType::Symlink,
                "originalRootedPath: {}, fileStateValueFromAncestors: {}, realRootedPath: {}, \
                 fileStateValueFromAncestors: {}",
                original_rooted_path,
                file_state_value_from_ancestors,
                real_rooted_path,
                real_file_state_value
            );
            blaze_util::check_state!(
                !real_file_state_value.get_type().exists()
                    || real_file_state_value.get_type().is_file()
                    || (logical_chain_during_resolution.len() == 1
                        && logical_chain_during_resolution[0] == *original_rooted_path),
                "logicalChainDuringResolution: {}, originalRootedPath: {}",
                fmt_list(&logical_chain_during_resolution),
                original_rooted_path
            );
            return FileValue::Regular(file_state_value_from_ancestors);
        }
        let should_store_chain = match real_file_state_value.get_type() {
            FileStateType::RegularFile
            | FileStateType::SpecialFile
            | FileStateType::Nonexistent => false,
            FileStateType::Symlink | FileStateType::Directory => true,
        };
        if file_state_value_from_ancestors.get_type() == FileStateType::Symlink {
            let symlink_target = file_state_value_from_ancestors.get_symlink_target().clone();
            if let Some(path_to_chain) = path_to_unbounded_ancestor_symlink_expansion_chain {
                FileValue::SymlinkWithUnboundedAncestorExpansion {
                    real_rooted_path,
                    real_file_state_value,
                    logical_chain_during_resolution,
                    link_target: symlink_target,
                    path_to_unbounded_ancestor_symlink_expansion_chain: path_to_chain,
                    unbounded_ancestor_symlink_expansion_chain:
                        unbounded_ancestor_symlink_expansion_chain.unwrap_or_default(),
                }
            } else if should_store_chain {
                FileValue::SymlinkWithStoredChain {
                    real_rooted_path,
                    real_file_state_value,
                    logical_chain_during_resolution,
                    link_target: symlink_target,
                }
            } else {
                FileValue::SymlinkWithoutStoredChain {
                    real_rooted_path,
                    real_file_state_value,
                    link_target: symlink_target,
                }
            }
        } else if let Some(path_to_chain) = path_to_unbounded_ancestor_symlink_expansion_chain {
            FileValue::DifferentRealPathWithUnboundedAncestorExpansion {
                real_rooted_path,
                real_file_state_value,
                logical_chain_during_resolution,
                path_to_unbounded_ancestor_symlink_expansion_chain: path_to_chain,
                unbounded_ancestor_symlink_expansion_chain:
                    unbounded_ancestor_symlink_expansion_chain.unwrap_or_default(),
            }
        } else if should_store_chain {
            FileValue::DifferentRealPathWithStoredChain {
                real_rooted_path,
                real_file_state_value,
                logical_chain_during_resolution,
            }
        } else {
            FileValue::DifferentRealPathWithoutStoredChain {
                real_rooted_path,
                real_file_state_value,
            }
        }
    }
}

/// Java `ImmutableList.toString`: `[a, b]`.
pub(crate) fn fmt_list<T: fmt::Display>(items: &[T]) -> String {
    let parts: Vec<String> = items.iter().map(|i| i.to_string()).collect();
    format!("[{}]", parts.join(", "))
}

impl fmt::Display for FileValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FileValue::Regular(state) => fmt::Display::fmt(state, f),
            // Java: com/google/devtools/build/lib/actions/FileValue.java:318 DifferentRealPathFileValueWithUnboundedAncestorExpansion.toString
            FileValue::DifferentRealPathWithUnboundedAncestorExpansion {
                real_rooted_path,
                real_file_state_value,
                logical_chain_during_resolution,
                path_to_unbounded_ancestor_symlink_expansion_chain,
                unbounded_ancestor_symlink_expansion_chain,
            } => write!(
                f,
                "symlink ancestor (real_path={}, real_state={}, chain={}, path={}, cycle={})",
                real_rooted_path,
                real_file_state_value,
                fmt_list(logical_chain_during_resolution),
                fmt_list(path_to_unbounded_ancestor_symlink_expansion_chain),
                fmt_list(unbounded_ancestor_symlink_expansion_chain)
            ),
            // Java: com/google/devtools/build/lib/actions/FileValue.java:393 DifferentRealPathFileValueWithStoredChain.toString
            FileValue::DifferentRealPathWithStoredChain {
                real_rooted_path,
                real_file_state_value,
                logical_chain_during_resolution,
            } => write!(
                f,
                "symlink ancestor (real_path={}, real_state={}, chain={})",
                real_rooted_path,
                real_file_state_value,
                fmt_list(logical_chain_during_resolution)
            ),
            // Java: com/google/devtools/build/lib/actions/FileValue.java:463 DifferentRealPathFileValueWithoutStoredChain.toString
            FileValue::DifferentRealPathWithoutStoredChain {
                real_rooted_path,
                real_file_state_value,
            } => write!(
                f,
                "symlink ancestor (real_path={}, real_state={})",
                real_rooted_path, real_file_state_value
            ),
            // Java: com/google/devtools/build/lib/actions/FileValue.java:536 SymlinkFileValueWithUnboundedAncestorExpansion.toString
            FileValue::SymlinkWithUnboundedAncestorExpansion {
                real_rooted_path,
                real_file_state_value,
                logical_chain_during_resolution,
                link_target,
                path_to_unbounded_ancestor_symlink_expansion_chain,
                unbounded_ancestor_symlink_expansion_chain,
            } => write!(
                f,
                "symlink ancestor (real_path={}, real_state={}, target={}, chain={}, path={}, \
                 cycle={})",
                real_rooted_path,
                real_file_state_value,
                link_target,
                fmt_list(logical_chain_during_resolution),
                fmt_list(path_to_unbounded_ancestor_symlink_expansion_chain),
                fmt_list(unbounded_ancestor_symlink_expansion_chain)
            ),
            // Java: com/google/devtools/build/lib/actions/FileValue.java:596 SymlinkFileValueWithStoredChain.toString
            FileValue::SymlinkWithStoredChain {
                real_rooted_path,
                real_file_state_value,
                logical_chain_during_resolution,
                link_target,
            } => write!(
                f,
                "symlink (real_path={}, real_state={}, link_value={}, chain={})",
                real_rooted_path,
                real_file_state_value,
                link_target,
                fmt_list(logical_chain_during_resolution)
            ),
            // Java: com/google/devtools/build/lib/actions/FileValue.java:646 SymlinkFileValueWithoutStoredChain.toString
            FileValue::SymlinkWithoutStoredChain {
                real_rooted_path,
                real_file_state_value,
                link_target,
            } => write!(
                f,
                "symlink (real_path={}, real_state={}, link_value={})",
                real_rooted_path, real_file_state_value, link_target
            ),
        }
    }
}
