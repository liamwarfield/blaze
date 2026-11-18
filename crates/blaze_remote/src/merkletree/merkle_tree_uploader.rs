//! Port of `lib/remote/merkletree/MerkleTreeUploader.java`.

use std::io;

use async_trait::async_trait;
use blaze_exec::actions::VirtualActionInput;
use blaze_vfs::Path;

use super::merkle_tree::MerkleTreeUploadable;
use crate::common::remote_action_execution_context::RemoteActionExecutionContext;
use crate::common::remote_path_resolver::RemotePathResolver;
use crate::reapi::Digest;

/// The basic cache operations needed to upload a `MerkleTree` and its associated blobs.
///
/// PORT: `remotePathResolver` is `Option` because Java passes `null` for trees built by
/// `MerkleTreeComputer.buildForFiles` (`RemoteExecutionCache` declares it `@Nullable`).
// Java: com/google/devtools/build/lib/remote/merkletree/MerkleTreeUploader.java:25 MerkleTreeUploader
#[async_trait]
pub trait MerkleTreeUploader: Send + Sync {
    /// Uploads an in-memory blob to the remote cache.
    // Java: com/google/devtools/build/lib/remote/merkletree/MerkleTreeUploader.java:27 MerkleTreeUploader.uploadBlob
    async fn upload_blob(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        data: bytes::Bytes,
        force: bool,
    ) -> io::Result<()>;

    /// Uploads a local file to the remote cache.
    // Java: com/google/devtools/build/lib/remote/merkletree/MerkleTreeUploader.java:31 MerkleTreeUploader.uploadFile
    async fn upload_file(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        remote_path_resolver: Option<&dyn RemotePathResolver>,
        digest: &Digest,
        path: &Path,
        force: bool,
    ) -> io::Result<()>;

    /// Uploads a virtual action input to the remote cache.
    // Java: com/google/devtools/build/lib/remote/merkletree/MerkleTreeUploader.java:39 MerkleTreeUploader.uploadVirtualActionInput
    async fn upload_virtual_action_input(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        digest: &Digest,
        virtual_action_input: &dyn VirtualActionInput,
        force: bool,
    ) -> io::Result<()>;

    /// Ensures that all inputs as well as metadata protos in the given Merkle tree are present
    /// in the remote cache by querying for and uploading missing blobs.
    ///
    /// * `force`: if true, all blobs in the tree are uploaded even if they have already been
    ///   uploaded before.
    ///
    /// PORT: blocking in Java (`throws IOException, InterruptedException`), where
    /// `MerkleTreeComputer` calls it on a virtual thread of its upload pool; `async` here
    /// because it is awaited inside the sub-tree computation.
    // Java: com/google/devtools/build/lib/remote/merkletree/MerkleTreeUploader.java:52 MerkleTreeUploader.ensureInputsPresent
    async fn ensure_inputs_present(
        &self,
        context: &RemoteActionExecutionContext<'_>,
        merkle_tree: &MerkleTreeUploadable,
        force: bool,
        remote_path_resolver: Option<&dyn RemotePathResolver>,
    ) -> io::Result<()>;
}
