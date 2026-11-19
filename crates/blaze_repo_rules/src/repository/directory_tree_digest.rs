//! DIRECTORY_TREE_DIGEST: the recursive digest of a directory tree.
//!
//! Java: com/google/devtools/build/lib/skyframe/DirectoryTreeDigestFunction.java and
//! DirectoryTreeDigestValue.java.
//!
//! PORT: Java keeps these in `lib/skyframe`; they live here because only repository rules
//! (`repository_ctx.watch_tree`, `RepoRecordedInput.DirTree`) use them.

use std::fmt;

use blaze_sky_loading::actions::FileStateType;
use blaze_sky_loading::actions::FileValue;
use blaze_sky_loading::directory_listing::DirectoryListingKey;
use blaze_sky_loading::file::FileKey;
use blaze_sky_loading::io::IOException;
use blaze_skyframe::ComputeError;
use blaze_skyframe::Environment;
use blaze_skyframe::Interrupted;
use blaze_skyframe::Key;
use blaze_skyframe::SkyFunctionException;
use blaze_skyframe::SkyFunctionName;
use blaze_skyframe::SkyKey;
use blaze_util::fingerprint::Fingerprint;
use blaze_vfs::RootedPath;
use indexmap::IndexSet;

// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:36 SkyFunctions.DIRECTORY_TREE_DIGEST
pub const DIRECTORY_TREE_DIGEST: SkyFunctionName = SkyFunctionName("DIRECTORY_TREE_DIGEST");

/// Contains information about the recursive digest of a directory tree, including all transitive
/// descendant files and their contents.
// Java: com/google/devtools/build/lib/skyframe/DirectoryTreeDigestValue.java:28 DirectoryTreeDigestValue
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct DirectoryTreeDigestValue {
    pub hex_digest: String,
}

impl DirectoryTreeDigestValue {
    // Java: com/google/devtools/build/lib/skyframe/DirectoryTreeDigestValue.java:33 DirectoryTreeDigestValue.of
    pub fn of(hex_digest: String) -> DirectoryTreeDigestValue {
        DirectoryTreeDigestValue { hex_digest }
    }

    // Java: com/google/devtools/build/lib/skyframe/DirectoryTreeDigestValue.java:28 DirectoryTreeDigestValue.hexDigest
    pub fn hex_digest(&self) -> &str {
        &self.hex_digest
    }
}

impl fmt::Debug for DirectoryTreeDigestValue {
    // Java record toString.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "DirectoryTreeDigestValue[hexDigest={}]", self.hex_digest)
    }
}

/// Key type for [`DirectoryTreeDigestValue`]. `Debug` prints `DIRECTORY_TREE_DIGEST:<rooted
/// path>` (Java `AbstractSkyKey.toString`).
// Java: com/google/devtools/build/lib/skyframe/DirectoryTreeDigestValue.java:42 DirectoryTreeDigestValue.Key
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct DirectoryTreeDigestKey(pub RootedPath);

impl DirectoryTreeDigestKey {
    // Java: com/google/devtools/build/lib/skyframe/DirectoryTreeDigestValue.java:37 DirectoryTreeDigestValue.key
    pub fn key(path: RootedPath) -> DirectoryTreeDigestKey {
        DirectoryTreeDigestKey(path)
    }

    pub fn argument(&self) -> &RootedPath {
        &self.0
    }
}

impl fmt::Debug for DirectoryTreeDigestKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}:{}", DIRECTORY_TREE_DIGEST, self.0)
    }
}

impl Key for DirectoryTreeDigestKey {
    type Value = DirectoryTreeDigestValue;
    const FUNCTION: SkyFunctionName = DIRECTORY_TREE_DIGEST;
}

/// Java `FileStateType.ordinal()`.
fn file_state_type_ordinal(t: FileStateType) -> i32 {
    match t {
        FileStateType::RegularFile => 0,
        FileStateType::SpecialFile => 1,
        FileStateType::Directory => 2,
        FileStateType::Symlink => 3,
        FileStateType::Nonexistent => 4,
    }
}

/// A SkyFunction for [`DirectoryTreeDigestValue`]s.
///
/// Errors: Java `DirectoryTreeDigestFunctionException(IOException)`, TRANSIENT, carrying a
/// [`IOException`].
// Java: com/google/devtools/build/lib/skyframe/DirectoryTreeDigestFunction.java:40 DirectoryTreeDigestFunction.compute
pub fn compute(
    sky_key: &DirectoryTreeDigestKey,
    env: &mut Environment<'_>,
) -> Result<Option<DirectoryTreeDigestValue>, ComputeError> {
    let rooted_path = &sky_key.0;
    let Some(dir_listing_value) = env.get_value(DirectoryListingKey::key(rooted_path.clone()))?
    else {
        return Ok(None);
    };

    // Get the names of entries directly in this directory, and sort them. This sets the basis for
    // subsequent digests.
    // Java sorts the internal names by UTF-16 code unit: `str` order for chars <= U+00FF.
    let mut sorted_dirents: Vec<String> = dir_listing_value
        .get_dirents()
        .iter()
        .map(|d| d.get_name().to_owned())
        .collect();
    sorted_dirents.sort();
    sorted_dirents.dedup();

    // Turn each entry into a FileValue.
    let Some(file_values) = get_file_values(env, &sorted_dirents, rooted_path)? else {
        return Ok(None);
    };

    // For each entry that is a directory (or a symlink to a directory), find its own
    // DirectoryTreeDigestValue.
    let Some(sub_dir_tree_digests) = get_sub_dir_tree_digests(env, &file_values)? else {
        return Ok(None);
    };

    // Finally, we're ready to digest everything together!
    let mut fp = Fingerprint::new();
    fp.add_strings(&sorted_dirents);
    fp.add_strings(&sub_dir_tree_digests);
    for (dirent_rooted_path, file_value) in &file_values {
        fp.add_int(file_state_type_ordinal(
            file_value.real_file_state_value().get_type(),
        ));
        if file_value.is_file() {
            let digest = match file_value.real_file_state_value().get_digest() {
                Some(digest) => digest.to_vec(),
                // Fast digest not available, or it would have been in the FileValue.
                None => match file_value
                    .real_rooted_path(dirent_rooted_path)
                    .as_path()
                    .get_digest()
                {
                    Ok(digest) => digest,
                    Err(e) => {
                        // Java: DirectoryTreeDigestFunctionException(e), TRANSIENT.
                        return Err(
                            SkyFunctionException::transient(IOException::from_io(&e)).into()
                        );
                    }
                },
            };
            fp.add_bytes(&digest);
        }
    }

    Ok(Some(DirectoryTreeDigestValue::of(
        fp.hex_digest_and_reset(),
    )))
}

// Java: com/google/devtools/build/lib/skyframe/DirectoryTreeDigestFunction.java:97 DirectoryTreeDigestFunction.getFileValues
fn get_file_values(
    env: &mut Environment<'_>,
    sorted_dirents: &[String],
    rooted_path: &RootedPath,
) -> Result<Option<Vec<(RootedPath, std::sync::Arc<FileValue>)>>, Interrupted> {
    let file_value_keys: IndexSet<FileKey> = sorted_dirents
        .iter()
        .map(|dirent| {
            FileKey::key(RootedPath::to_rooted_path(
                rooted_path.get_root().clone(),
                &rooted_path
                    .get_root_relative_path()
                    .get_relative_str(dirent),
            ))
        })
        .collect();
    let sky_keys: Vec<SkyKey> = file_value_keys.iter().cloned().map(SkyKey::new).collect();
    let result = env.get_values_and_exceptions(&sky_keys)?;
    if env.values_missing() {
        return Ok(None);
    }
    let mut file_values = Vec::with_capacity(file_value_keys.len());
    let mut missing = false;
    for k in &file_value_keys {
        match result.get_typed(k) {
            Some(v) => file_values.push((k.0.clone(), v)),
            None => missing = true,
        }
    }
    if missing || env.values_missing() {
        return Ok(None);
    }
    Ok(Some(file_values))
}

// Java: com/google/devtools/build/lib/skyframe/DirectoryTreeDigestFunction.java:124 DirectoryTreeDigestFunction.getSubDirTreeDigests
fn get_sub_dir_tree_digests(
    env: &mut Environment<'_>,
    file_values: &[(RootedPath, std::sync::Arc<FileValue>)],
) -> Result<Option<Vec<String>>, Interrupted> {
    // toImmutableSet: first occurrence order, duplicates dropped (two symlinks to the same
    // directory contribute one digest).
    let dir_tree_digest_value_keys: IndexSet<DirectoryTreeDigestKey> = file_values
        .iter()
        .filter(|(_, v)| v.is_directory())
        .map(|(p, v)| DirectoryTreeDigestKey::key(v.real_rooted_path(p)))
        .collect();
    let sky_keys: Vec<SkyKey> = dir_tree_digest_value_keys
        .iter()
        .cloned()
        .map(SkyKey::new)
        .collect();
    let result = env.get_values_and_exceptions(&sky_keys)?;
    if env.values_missing() {
        return Ok(None);
    }
    let mut digests = Vec::with_capacity(dir_tree_digest_value_keys.len());
    for k in &dir_tree_digest_value_keys {
        match result.get_typed(k) {
            Some(v) => digests.push(v.hex_digest.clone()),
            None => return Ok(None),
        }
    }
    Ok(Some(digests))
}
