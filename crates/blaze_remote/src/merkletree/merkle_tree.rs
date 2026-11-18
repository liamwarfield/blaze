//! Port of `lib/remote/merkletree/MerkleTree.java`.

use std::cmp::Ordering;
use std::collections::BTreeMap;
use std::io;

use blaze_exec::actions::{ActionInputRef, FileArtifactValue};
use futures::future::BoxFuture;
use indexmap::IndexMap;

use super::merkle_tree_computer::{ArtifactPathResolver, PATH_ACTION_INPUT_RESOLVER};
use super::merkle_tree_uploader::MerkleTreeUploader;
use crate::common::remote_action_execution_context::RemoteActionExecutionContext;
use crate::common::remote_path_resolver::RemotePathResolver;
use crate::reapi::Digest;
use crate::util::digest_util::{DigestUtil, digest_comparator};

/// A representation of the inputs to a remotely executed action represented as a Merkle tree.
///
/// Every tree has a digest, which is the digest of the tree's root directory. The subtrees and
/// the blobs they contain may have been discarded or never computed in the first place, for
/// example, because they have already been uploaded to the remote cache or because the tree is
/// being built only to check for a remote cache hit.
// Java: com/google/devtools/build/lib/remote/merkletree/MerkleTree.java:46 MerkleTree
#[derive(Clone, Debug)]
pub enum MerkleTree {
    /// `MerkleTree.RootOnly`.
    RootOnly(MerkleTreeRootOnly),
    /// `MerkleTree.Uploadable`.
    Uploadable(MerkleTreeUploadable),
}

impl MerkleTree {
    /// The digest of the tree's root directory.
    // Java: com/google/devtools/build/lib/remote/merkletree/MerkleTree.java:48 MerkleTree.digest
    pub fn digest(&self) -> &Digest {
        match self {
            MerkleTree::RootOnly(root_only) => root_only.digest(),
            MerkleTree::Uploadable(uploadable) => uploadable.digest(),
        }
    }

    /// The total number of regular files and symlinks in this tree, including all subtrees.
    // Java: com/google/devtools/build/lib/remote/merkletree/MerkleTree.java:51 MerkleTree.inputFiles
    pub fn input_files(&self) -> i64 {
        match self {
            MerkleTree::RootOnly(root_only) => root_only.input_files(),
            MerkleTree::Uploadable(uploadable) => uploadable.input_files(),
        }
    }

    /// The total number of content bytes in this tree, including all subtrees. This includes
    /// both file contents and the protos describing directories.
    // Java: com/google/devtools/build/lib/remote/merkletree/MerkleTree.java:57 MerkleTree.inputBytes
    pub fn input_bytes(&self) -> i64 {
        match self {
            MerkleTree::RootOnly(root_only) => root_only.input_bytes(),
            MerkleTree::Uploadable(uploadable) => uploadable.input_bytes(),
        }
    }

    /// Returns the root of this tree, which may be the current instance.
    // Java: com/google/devtools/build/lib/remote/merkletree/MerkleTree.java:60 MerkleTree.root
    pub fn root(&self) -> &MerkleTreeRootOnly {
        match self {
            MerkleTree::RootOnly(root_only) => root_only.root(),
            MerkleTree::Uploadable(uploadable) => uploadable.root(),
        }
    }
}

impl From<MerkleTreeRootOnly> for MerkleTree {
    fn from(root_only: MerkleTreeRootOnly) -> MerkleTree {
        MerkleTree::RootOnly(root_only)
    }
}

impl From<MerkleTreeUploadable> for MerkleTree {
    fn from(uploadable: MerkleTreeUploadable) -> MerkleTree {
        MerkleTree::Uploadable(uploadable)
    }
}

/// A `MerkleTree` that doesn't retain any blobs, either because they have already been uploaded
/// or because only the root digest is needed (e.g., for a remote cache check).
// Java: com/google/devtools/build/lib/remote/merkletree/MerkleTree.java:66 MerkleTree.RootOnly
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum MerkleTreeRootOnly {
    /// A `MerkleTree` that retains no blobs since all of them have recently been uploaded to the
    /// remote cache.
    // Java: com/google/devtools/build/lib/remote/merkletree/MerkleTree.java:76 MerkleTree.RootOnly.BlobsUploaded
    BlobsUploaded {
        digest: Digest,
        input_files: i64,
        input_bytes: i64,
    },
    /// A `MerkleTree` that retains no blobs since they were discarded during the computation
    /// (e.g., because they aren't needed for a remote cache check).
    // Java: com/google/devtools/build/lib/remote/merkletree/MerkleTree.java:82 MerkleTree.RootOnly.BlobsDiscarded
    BlobsDiscarded {
        digest: Digest,
        input_files: i64,
        input_bytes: i64,
    },
}

impl MerkleTreeRootOnly {
    /// `new MerkleTree.RootOnly.BlobsUploaded(digest, inputFiles, inputBytes)`.
    pub fn blobs_uploaded(digest: Digest, input_files: i64, input_bytes: i64) -> Self {
        MerkleTreeRootOnly::BlobsUploaded {
            digest,
            input_files,
            input_bytes,
        }
    }

    /// `new MerkleTree.RootOnly.BlobsDiscarded(digest, inputFiles, inputBytes)`.
    pub fn blobs_discarded(digest: Digest, input_files: i64, input_bytes: i64) -> Self {
        MerkleTreeRootOnly::BlobsDiscarded {
            digest,
            input_files,
            input_bytes,
        }
    }

    /// `digest()`.
    pub fn digest(&self) -> &Digest {
        match self {
            MerkleTreeRootOnly::BlobsUploaded { digest, .. }
            | MerkleTreeRootOnly::BlobsDiscarded { digest, .. } => digest,
        }
    }

    /// `inputFiles()`.
    pub fn input_files(&self) -> i64 {
        match self {
            MerkleTreeRootOnly::BlobsUploaded { input_files, .. }
            | MerkleTreeRootOnly::BlobsDiscarded { input_files, .. } => *input_files,
        }
    }

    /// `inputBytes()`.
    pub fn input_bytes(&self) -> i64 {
        match self {
            MerkleTreeRootOnly::BlobsUploaded { input_bytes, .. }
            | MerkleTreeRootOnly::BlobsDiscarded { input_bytes, .. } => *input_bytes,
        }
    }

    /// `root()`.
    // Java: com/google/devtools/build/lib/remote/merkletree/MerkleTree.java:68 MerkleTree.RootOnly.root
    pub fn root(&self) -> &MerkleTreeRootOnly {
        self
    }

    /// `this instanceof MerkleTree.RootOnly.BlobsUploaded`.
    pub fn is_blobs_uploaded(&self) -> bool {
        matches!(self, MerkleTreeRootOnly::BlobsUploaded { .. })
    }

    /// `this instanceof MerkleTree.RootOnly.BlobsDiscarded`.
    pub fn is_blobs_discarded(&self) -> bool {
        matches!(self, MerkleTreeRootOnly::BlobsDiscarded { .. })
    }
}

/// A key of the blobs map of a [`MerkleTreeUploadable`] (Java: `/* Digest | FileArtifactValue */
/// Object`), ordered by `DIGEST_AND_METADATA_COMPARATOR`.
#[derive(Clone, Debug)]
pub(super) enum BlobKey {
    Digest(Digest),
    Metadata(FileArtifactValue),
}

impl PartialEq for BlobKey {
    fn eq(&self, other: &BlobKey) -> bool {
        digest_and_metadata_comparator(self, other) == Ordering::Equal
    }
}

impl Eq for BlobKey {}

impl PartialOrd for BlobKey {
    fn partial_cmp(&self, other: &BlobKey) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for BlobKey {
    fn cmp(&self, other: &BlobKey) -> Ordering {
        digest_and_metadata_comparator(self, other)
    }
}

/// A value of the blobs map of a [`MerkleTreeUploadable`] (Java: `/* byte[] | ActionInput */
/// Object`).
#[derive(Clone, Debug)]
pub enum MerkleTreeBlob {
    /// `byte[]`: the serialized `Directory` message.
    Bytes(bytes::Bytes),
    /// `ActionInput` (a `VirtualActionInput` when `as_virtual_action_input()` says so).
    ActionInput(ActionInputRef),
}

/// `FileArtifactValue.getDigest()` of a value used as a blob key.
fn metadata_digest(metadata: &FileArtifactValue) -> &[u8] {
    match metadata.get_digest() {
        Some(digest) => digest,
        None => panic!("NullPointerException: FileArtifactValue.getDigest()"),
    }
}

// Java: com/google/devtools/build/lib/remote/merkletree/MerkleTree.java:97 MerkleTree.Uploadable.FILE_ARTIFACT_VALUE_COMPARATOR
fn file_artifact_value_comparator(a: &FileArtifactValue, b: &FileArtifactValue) -> Ordering {
    // UnsignedBytes.lexicographicalComparator(), then the size.
    metadata_digest(a)
        .cmp(metadata_digest(b))
        .then(a.get_size().cmp(&b.get_size()))
}

// Java: com/google/devtools/build/lib/remote/merkletree/MerkleTree.java:100 MerkleTree.Uploadable.DIGEST_AND_METADATA_COMPARATOR
pub(super) fn digest_and_metadata_comparator(o1: &BlobKey, o2: &BlobKey) -> Ordering {
    match o1 {
        BlobKey::Digest(digest1) => match o2 {
            BlobKey::Digest(digest2) => digest_comparator(digest1, digest2),
            BlobKey::Metadata(_) => digest_comparator(digest1, &adapt_to_digest(o2)),
        },
        BlobKey::Metadata(metadata1) => match o2 {
            BlobKey::Metadata(metadata2) => file_artifact_value_comparator(metadata1, metadata2),
            BlobKey::Digest(digest2) => digest_comparator(&adapt_to_digest(o1), digest2),
        },
    }
}

// Java: com/google/devtools/build/lib/remote/merkletree/MerkleTree.java:201 MerkleTree.Uploadable.adaptToDigest
fn adapt_to_digest(key: &BlobKey) -> Digest {
    match key {
        BlobKey::Digest(digest) => digest.clone(),
        BlobKey::Metadata(metadata) => {
            DigestUtil::build_digest_from_bytes(metadata_digest(metadata), metadata.get_size())
        }
    }
}

/// A `MerkleTree` that retains all blobs that still need to be uploaded.
///
/// The empty blob doesn't have to be uploaded and is thus never included in the blobs map.
///
/// See `RemoteExecutionServiceTest#buildRemoteAction_goldenTest` for a test that verifies the
/// memory footprint of this class. Since there can be thousands of inflight remote executions
/// that may have to retain their blobs until all inputs have been uploaded, it's crucial to keep
/// the memory footprint of this class as low as possible.
// Java: com/google/devtools/build/lib/remote/merkletree/MerkleTree.java:96 MerkleTree.Uploadable
#[derive(Clone, Debug)]
pub struct MerkleTreeUploadable {
    /// Always a `BlobsUploaded`.
    root: MerkleTreeRootOnly,
    /// `ImmutableSortedMap</* Digest | FileArtifactValue */ Object, /* byte[] | ActionInput */
    /// Object>`: two flat arrays sorted by `DIGEST_AND_METADATA_COMPARATOR`.
    blobs: Vec<(BlobKey, MerkleTreeBlob)>,
    // PORT: ArtifactPathResolver is the exec root (missing hook:
    // SpawnExecutionContext.getPathResolver). Java asks the spawn execution context of the
    // upload for its resolver; the tree carries the resolver of the computer that built it.
    exec_root_path_resolver: ArtifactPathResolver,
}

impl MerkleTreeUploadable {
    // Java: com/google/devtools/build/lib/remote/merkletree/MerkleTree.java:126 MerkleTree.Uploadable.<init>
    pub(super) fn new(
        root: MerkleTreeRootOnly,
        blobs: BTreeMap<BlobKey, MerkleTreeBlob>,
        exec_root_path_resolver: ArtifactPathResolver,
    ) -> MerkleTreeUploadable {
        blaze_util::check_argument!(root.is_blobs_uploaded());
        MerkleTreeUploadable {
            root,
            // A sorted map requires less memory than a regular hash map as it only stores two
            // flat sorted arrays. Access performance is not critical since it's only used to
            // find missing blobs, which always require network access.
            blobs: blobs.into_iter().collect(),
            exec_root_path_resolver,
        }
    }

    /// `digest()`.
    // Java: com/google/devtools/build/lib/remote/merkletree/MerkleTree.java:138 MerkleTree.Uploadable.digest
    pub fn digest(&self) -> &Digest {
        self.root().digest()
    }

    /// `inputFiles()`.
    // Java: com/google/devtools/build/lib/remote/merkletree/MerkleTree.java:143 MerkleTree.Uploadable.inputFiles
    pub fn input_files(&self) -> i64 {
        self.root().input_files()
    }

    /// `inputBytes()`.
    // Java: com/google/devtools/build/lib/remote/merkletree/MerkleTree.java:148 MerkleTree.Uploadable.inputBytes
    pub fn input_bytes(&self) -> i64 {
        self.root().input_bytes()
    }

    /// `allDigests()`.
    // Java: com/google/devtools/build/lib/remote/merkletree/MerkleTree.java:152 MerkleTree.Uploadable.allDigests
    pub fn all_digests(&self) -> Vec<Digest> {
        self.blobs
            .iter()
            .map(|(key, _)| adapt_to_digest(key))
            .collect()
    }

    /// `blobs()` (`@VisibleForTesting`).
    // Java: com/google/devtools/build/lib/remote/merkletree/MerkleTree.java:157 MerkleTree.Uploadable.blobs
    pub fn blobs(&self) -> IndexMap<Digest, MerkleTreeBlob> {
        self.blobs
            .iter()
            .map(|(key, value)| (adapt_to_digest(key), value.clone()))
            .collect()
    }

    /// `root()`.
    // Java: com/google/devtools/build/lib/remote/merkletree/MerkleTree.java:163 MerkleTree.Uploadable.root
    pub fn root(&self) -> &MerkleTreeRootOnly {
        &self.root
    }

    /// `blobs.get(digest)`.
    fn get_blob(&self, digest: &Digest) -> Option<&MerkleTreeBlob> {
        let key = BlobKey::Digest(digest.clone());
        self.blobs
            .binary_search_by(|(k, _)| digest_and_metadata_comparator(k, &key))
            .ok()
            .map(|index| &self.blobs[index].1)
    }

    /// Returns a future that tracks the upload of the blob with the given digest, or `None` if
    /// there is no blob with the given digest.
    // Java: com/google/devtools/build/lib/remote/merkletree/MerkleTree.java:171 MerkleTree.Uploadable.upload
    pub fn upload<'a>(
        &'a self,
        uploader: &'a dyn MerkleTreeUploader,
        context: &'a RemoteActionExecutionContext<'a>,
        remote_path_resolver: Option<&'a dyn RemotePathResolver>,
        digest: &'a Digest,
        force: bool,
    ) -> Option<BoxFuture<'a, io::Result<()>>> {
        match self.get_blob(digest) {
            Some(MerkleTreeBlob::Bytes(data)) => {
                Some(uploader.upload_blob(context, digest, data.clone(), force))
            }
            Some(MerkleTreeBlob::ActionInput(action_input)) => {
                if let Some(virtual_action_input) = action_input.as_virtual_action_input() {
                    return Some(uploader.upload_virtual_action_input(
                        context,
                        digest,
                        virtual_action_input,
                        force,
                    ));
                }
                let spawn_execution_context = context.get_spawn_execution_context();
                // This can only be null when uploading a tree created by
                // MerkleTreeComputer#buildForFiles, which only happens for remote repo execution
                // and tests. Only the latter actually reach this code path since remote repo
                // execution doesn't upload any inputs.
                let path = if spawn_execution_context.is_some() {
                    self.exec_root_path_resolver.to_path(&**action_input)
                } else {
                    PATH_ACTION_INPUT_RESOLVER.to_path(&**action_input)
                };
                Some(Box::pin(async move {
                    uploader
                        .upload_file(context, remote_path_resolver, digest, &path, force)
                        .await
                }))
            }
            None => None,
        }
    }
}
