//! DIRECTORY_LISTING_STATE and DIRECTORY_LISTING.
//!
//! Java: com/google/devtools/build/lib/skyframe/{DirectoryListingStateFunction,
//! DirectoryListingStateValue,DirectoryListingFunction,DirectoryListingValue,DirectoryListingKey,
//! CompactSortedDirents,Dirents}.java.

use std::fmt;
use std::sync::Arc;

use blaze_skyframe::{
    ComputeError, Environment, FunctionHermeticity, Key, SkyFunctionException, SkyFunctionName,
};
use blaze_vfs::{Dirent, RootedPath, Symlinks};

use crate::file::FileKey;
use crate::file_state::{ExternalFilesHelper, NonexistentImmutableExternalFileException};
use crate::io::IOException;
use crate::sky_functions;

/// A sorted, immutable dirent structure: iterating over the entries in a directory and getting
/// the entry, if any, for a given basename.
///
/// PORT: Java packs the types into a `BitSet` for memory; Blaze keeps a sorted `Vec<Dirent>`.
// Java: com/google/devtools/build/lib/skyframe/CompactSortedDirents.java:31 CompactSortedDirents
// Java: com/google/devtools/build/lib/skyframe/Dirents.java:24 Dirents
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct Dirents {
    dirents: Vec<Dirent>,
}

impl Dirents {
    // Java: com/google/devtools/build/lib/skyframe/CompactSortedDirents.java:41 CompactSortedDirents.create
    pub fn create(dirents: Vec<Dirent>) -> Dirents {
        let mut dirents = dirents;
        // Java sorts by Dirent.compareTo (the name), stably.
        dirents.sort_by(|a, b| a.get_name().cmp(b.get_name()));
        Dirents { dirents }
    }

    // Java: com/google/devtools/build/lib/skyframe/CompactSortedDirents.java:76 CompactSortedDirents.maybeGetDirent
    pub fn maybe_get_dirent(&self, base_name: &str) -> Option<&Dirent> {
        self.dirents
            .binary_search_by(|d| d.get_name().cmp(base_name))
            .ok()
            .map(|pos| &self.dirents[pos])
    }

    pub fn iter(&self) -> std::slice::Iter<'_, Dirent> {
        self.dirents.iter()
    }

    // Java: com/google/devtools/build/lib/skyframe/CompactSortedDirents.java:103 CompactSortedDirents.size
    pub fn size(&self) -> usize {
        self.dirents.len()
    }
}

impl<'a> IntoIterator for &'a Dirents {
    type Item = &'a Dirent;
    type IntoIter = std::slice::Iter<'a, Dirent>;
    fn into_iter(self) -> Self::IntoIter {
        self.dirents.iter()
    }
}

/// Encapsulates the filesystem operations needed to get the directory entries of a directory.
// Java: com/google/devtools/build/lib/skyframe/DirectoryListingStateValue.java:36 DirectoryListingStateValue
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct DirectoryListingStateValue {
    compact_sorted_dirents: Dirents,
}

impl DirectoryListingStateValue {
    // Java: com/google/devtools/build/lib/skyframe/DirectoryListingStateValue.java:45 DirectoryListingStateValue.create
    pub fn create(dirents: Vec<Dirent>) -> DirectoryListingStateValue {
        DirectoryListingStateValue {
            compact_sorted_dirents: Dirents::create(dirents),
        }
    }

    /// Returns the directory entries for this directory, in a stable order. Symlinks are not
    /// expanded.
    // Java: com/google/devtools/build/lib/skyframe/DirectoryListingStateValue.java:87 DirectoryListingStateValue.getDirents
    pub fn get_dirents(&self) -> &Dirents {
        &self.compact_sorted_dirents
    }
}

/// Key type for [`DirectoryListingStateValue`].
// Java: com/google/devtools/build/lib/skyframe/DirectoryListingStateValue.java:56 DirectoryListingStateValue.Key
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct DirectoryListingStateKey(pub RootedPath);

impl DirectoryListingStateKey {
    // Java: com/google/devtools/build/lib/skyframe/DirectoryListingStateValue.java:50 DirectoryListingStateValue.key
    pub fn key(rooted_path: RootedPath) -> Self {
        DirectoryListingStateKey(rooted_path)
    }
}

impl fmt::Debug for DirectoryListingStateKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", Self::FUNCTION, self.0)
    }
}

impl Key for DirectoryListingStateKey {
    type Value = DirectoryListingStateValue;
    const FUNCTION: SkyFunctionName = sky_functions::DIRECTORY_LISTING_STATE;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:33 SkyFunctions.DIRECTORY_LISTING_STATE (createNonHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Nonhermetic;
    /// Java: `CompactSortedDirents.equals`, the dirents sorted by name, each name and type.
    // Java: com/google/devtools/build/lib/skyframe/DirectoryListingStateValue.java:99 DirectoryListingStateValue.equals
    fn values_equal(old: &DirectoryListingStateValue, new: &DirectoryListingStateValue) -> bool {
        old == new
    }
}

/// A SkyFunction for [`DirectoryListingStateValue`]s.
///
/// Merely calls `DirectoryListingStateValue.create`, but also has special handling for
/// directories outside the package roots (see [`ExternalFilesHelper`]).
///
/// PORT: the `SyscallCache` is not ported; directories are always read.
// Java: com/google/devtools/build/lib/skyframe/DirectoryListingStateFunction.java:34 DirectoryListingStateFunction
pub struct DirectoryListingStateFunction {
    external_files_helper: Arc<ExternalFilesHelper>,
}

impl DirectoryListingStateFunction {
    // Java: com/google/devtools/build/lib/skyframe/DirectoryListingStateFunction.java:45 DirectoryListingStateFunction.<init>
    pub fn new(external_files_helper: Arc<ExternalFilesHelper>) -> Self {
        DirectoryListingStateFunction {
            external_files_helper,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/DirectoryListingStateFunction.java:51 DirectoryListingStateFunction.compute
    pub fn compute(
        &self,
        sky_key: &DirectoryListingStateKey,
        env: &mut Environment<'_>,
    ) -> Result<Option<DirectoryListingStateValue>, ComputeError> {
        let dir_rooted_path = &sky_key.0;
        match self
            .external_files_helper
            .maybe_handle_external_file(dir_rooted_path, env)?
        {
            Ok(_file_type) => {}
            Err(NonexistentImmutableExternalFileException) => {
                // DirectoryListingStateValue.key assumes the path exists. This exception here is
                // therefore indicative of a programming bug.
                panic!("IllegalStateException: {}", dir_rooted_path);
            }
        }
        if env.values_missing() {
            return Ok(None);
        }
        // PORT: Java reads EXTERNAL_REPO directories without the syscall cache and others through
        // it; Blaze has no cache, so both read the directory (NOFOLLOW).
        match dir_rooted_path.as_path().readdir(Symlinks::Nofollow) {
            Ok(dirents) => Ok(Some(DirectoryListingStateValue::create(dirents))),
            // Java: DirectoryListingStateFunctionException(IOException): TRANSIENT.
            Err(e) => Err(SkyFunctionException::transient(IOException::from_io(&e)).into()),
        }
    }
}

/// A value that represents the dirents (name and type of child entries) in a given directory
/// under a given package path root, fully accounting for symlinks in the directory's path.
/// Anything in Skyframe that cares about the contents of a directory should have a dependency on
/// the corresponding value.
///
/// Note that dirents that are themselves symlinks are *not* resolved. Consumers of such a dirent
/// are responsible for resolving the symlink entry via an appropriate FileValue.
// Java: com/google/devtools/build/lib/skyframe/DirectoryListingValue.java:41 DirectoryListingValue
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum DirectoryListingValue {
    /// Normal value.
    // Java: com/google/devtools/build/lib/skyframe/DirectoryListingValue.java:77 DirectoryListingValue.RegularDirectoryListingValue
    Regular(Arc<DirectoryListingStateValue>),
    /// A value with a different real root.
    // Java: com/google/devtools/build/lib/skyframe/DirectoryListingValue.java:108 DirectoryListingValue.DifferentRealPathDirectoryListingValue
    DifferentRealPath {
        real_dir_rooted_path: RootedPath,
        directory_listing_state_value: Arc<DirectoryListingStateValue>,
    },
}

impl DirectoryListingValue {
    /// Returns the directory entries for this directory, in a stable order. Symlinks are not
    /// expanded.
    // Java: com/google/devtools/build/lib/skyframe/DirectoryListingValue.java:48 DirectoryListingValue.getDirents
    pub fn get_dirents(&self) -> &Dirents {
        self.get_directory_listing_state_value().get_dirents()
    }

    // Java: com/google/devtools/build/lib/skyframe/DirectoryListingValue.java:52 DirectoryListingValue.getDirectoryListingStateValue
    pub fn get_directory_listing_state_value(&self) -> &DirectoryListingStateValue {
        match self {
            DirectoryListingValue::Regular(v) => v,
            DirectoryListingValue::DifferentRealPath {
                directory_listing_state_value,
                ..
            } => directory_listing_state_value,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/DirectoryListingValue.java:64 DirectoryListingValue.value
    fn value(
        dir_rooted_path: &RootedPath,
        dir_file_value: &crate::actions::FileValue,
        real_directory_listing_state_value: Arc<DirectoryListingStateValue>,
    ) -> DirectoryListingValue {
        let real_rooted_path = dir_file_value.real_rooted_path(dir_rooted_path);
        if real_rooted_path == *dir_rooted_path {
            DirectoryListingValue::Regular(real_directory_listing_state_value)
        } else {
            DirectoryListingValue::DifferentRealPath {
                real_dir_rooted_path: real_rooted_path,
                directory_listing_state_value: real_directory_listing_state_value,
            }
        }
    }
}

/// Key for [`directory_listing_function`]. The path is assumed to be an existing directory (e.g.
/// via `FileValue.is_directory` or from a directory listing on its parent directory).
// Java: com/google/devtools/build/lib/skyframe/DirectoryListingKey.java:25 DirectoryListingKey
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct DirectoryListingKey(pub RootedPath);

impl DirectoryListingKey {
    // Java: com/google/devtools/build/lib/skyframe/DirectoryListingValue.java:60 DirectoryListingValue.key
    pub fn key(directory_under_root: RootedPath) -> Self {
        DirectoryListingKey(directory_under_root)
    }
}

impl fmt::Debug for DirectoryListingKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", Self::FUNCTION, self.0)
    }
}

impl Key for DirectoryListingKey {
    type Value = DirectoryListingValue;
    const FUNCTION: SkyFunctionName = sky_functions::DIRECTORY_LISTING;
    // Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:35 SkyFunctions.DIRECTORY_LISTING (createHermetic)
    const HERMETICITY: FunctionHermeticity = FunctionHermeticity::Hermetic;
    /// Java: the state value (and, for a different real path, the real path).
    // Java: com/google/devtools/build/lib/skyframe/DirectoryListingValue.java:85 DirectoryListingValue.RegularDirectoryListingValue.equals
    fn values_equal(old: &DirectoryListingValue, new: &DirectoryListingValue) -> bool {
        old == new
    }
}

/// A SkyFunction for [`DirectoryListingValue`]s.
// Java: com/google/devtools/build/lib/skyframe/DirectoryListingFunction.java:31 DirectoryListingFunction.compute
pub fn directory_listing_function(
    sky_key: &DirectoryListingKey,
    env: &mut Environment<'_>,
) -> Result<Option<DirectoryListingValue>, ComputeError> {
    let dir_rooted_path = &sky_key.0;
    let Some(dir_file_value) = env.get_value(FileKey::key(dir_rooted_path.clone()))? else {
        return Ok(None);
    };
    let real_dir_rooted_path = dir_file_value.real_rooted_path(dir_rooted_path);
    if !dir_file_value.is_directory() {
        // Recall that the directory is assumed to exist (see DirectoryListingValue#key).
        // Java: DirectoryListingFunctionException(InconsistentFilesystemException): TRANSIENT.
        return Err(
            SkyFunctionException::transient(IOException::inconsistent_filesystem(format!(
                "{} is no longer an existing directory. Did you delete it during the build?",
                dir_rooted_path.as_path()
            )))
            .into(),
        );
    }
    let Some(directory_listing_state_value) =
        env.get_value(DirectoryListingStateKey::key(real_dir_rooted_path))?
    else {
        return Ok(None);
    };
    Ok(Some(DirectoryListingValue::value(
        dir_rooted_path,
        &dir_file_value,
        directory_listing_state_value,
    )))
}
