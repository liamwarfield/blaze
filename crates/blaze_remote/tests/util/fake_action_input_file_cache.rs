//! Port of the test class `lib/remote/FakeActionInputFileCache.java`.

use std::collections::HashMap;
use std::io;
use std::sync::Arc;

use blaze_exec::actions::{
    ActionInputRef, FileArtifactValue, FileContentsProxy, FilesetOutputTree, InputMetadataProvider,
    RunfilesArtifactValue, RunfilesTree, TreeArtifactValue,
};
use blaze_remote::reapi::{Digest, Tree};
use blaze_remote::util::DigestUtil;
use blaze_vfs::{Path, PathFragment};
use parking_lot::Mutex;

/// A fake implementation of the `InputMetadataProvider` interface.
///
/// PORT: Java's maps are plain fields mutated through the fake's methods; here they sit behind
/// mutexes so that the fake can be shared (`Arc`) with the spawn execution context and still be
/// filled in by the test.
// Java: com/google/devtools/build/lib/remote/FakeActionInputFileCache.java:43 FakeActionInputFileCache
#[allow(dead_code)]
pub struct FakeActionInputFileCache {
    exec_root: Path,
    cas: Mutex<HashMap<PathFragment, FileArtifactValue>>,
    inputs: Mutex<HashMap<PathFragment, ActionInputRef>>,
    runfiles_map: Mutex<HashMap<ActionInputRef, Arc<RunfilesArtifactValue>>>,
    trees: Mutex<HashMap<ActionInputRef, Arc<TreeArtifactValue>>>,
    runfiles_trees: Mutex<Vec<Arc<dyn RunfilesTree>>>,
    digest_util: DigestUtil,
}

#[allow(dead_code)]
impl FakeActionInputFileCache {
    // Java: com/google/devtools/build/lib/remote/FakeActionInputFileCache.java:51 FakeActionInputFileCache.<init>
    pub fn new(exec_root: Path) -> Self {
        let digest_util = DigestUtil::new(exec_root.get_file_system().get_digest_function());
        FakeActionInputFileCache {
            exec_root,
            cas: Mutex::new(HashMap::new()),
            inputs: Mutex::new(HashMap::new()),
            runfiles_map: Mutex::new(HashMap::new()),
            trees: Mutex::new(HashMap::new()),
            runfiles_trees: Mutex::new(Vec::new()),
            digest_util,
        }
    }

    /// `getRunfilesTrees()`.
    // PORT: not part of blaze_exec's `InputMetadataProvider` trait, kept as an inherent method.
    // Java: com/google/devtools/build/lib/remote/FakeActionInputFileCache.java:96 FakeActionInputFileCache.getRunfilesTrees
    pub fn get_runfiles_trees(&self) -> Vec<Arc<dyn RunfilesTree>> {
        self.runfiles_trees.lock().clone()
    }

    // Java: com/google/devtools/build/lib/remote/FakeActionInputFileCache.java:106 FakeActionInputFileCache.setMetadata
    fn set_metadata(&self, input: &ActionInputRef, metadata: FileArtifactValue) {
        self.cas
            .lock()
            .insert(input.get_exec_path().clone(), metadata);
        self.inputs
            .lock()
            .insert(input.get_exec_path().clone(), input.clone());
    }

    /// This test-only implementation does *not* support looking up tree artifact children via
    /// `get_input`.
    // Java: com/google/devtools/build/lib/remote/FakeActionInputFileCache.java:115 FakeActionInputFileCache.addTreeArtifact
    pub fn add_tree_artifact(&self, tree_artifact: &ActionInputRef, value: TreeArtifactValue) {
        self.trees
            .lock()
            .insert(tree_artifact.clone(), Arc::new(value));
    }

    /// `addRunfilesTree(runfilesTreeArtifact, runfilesTree)`.
    // Java: com/google/devtools/build/lib/remote/FakeActionInputFileCache.java:119 FakeActionInputFileCache.addRunfilesTree
    pub fn add_runfiles_tree(
        &self,
        runfiles_tree_artifact: &ActionInputRef,
        runfiles_tree: Arc<dyn RunfilesTree>,
    ) {
        // PORT: blaze_exec's `RunfilesArtifactValue` has no fileset lists (see its doc); the
        // other four lists are empty as in Java.
        self.runfiles_map.lock().insert(
            runfiles_tree_artifact.clone(),
            Arc::new(RunfilesArtifactValue::new(
                runfiles_tree.clone(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
                Vec::new(),
            )),
        );
        self.runfiles_trees.lock().push(runfiles_tree);
    }

    /// `createScratchInput(input, content)`.
    // Java: com/google/devtools/build/lib/remote/FakeActionInputFileCache.java:133 FakeActionInputFileCache.createScratchInput
    pub fn create_scratch_input(
        &self,
        input: &ActionInputRef,
        content: &str,
    ) -> io::Result<Digest> {
        let input_file = self.exec_root.get_relative(input.get_exec_path());
        input_file
            .get_parent_directory()
            .expect("NullPointerException: parent directory")
            .create_directory_and_parents()?;
        // `FileSystemUtils.writeContentAsLatin1(inputFile, content)`.
        // PORT: StringEncoding identity: Java's content is an internal string (Latin-1 chars
        // holding UTF-8 bytes), so its Latin-1 encoding is the UTF-8 encoding of the Rust string.
        blaze_repo_fetch::fs_utils::write_content(&input_file, content.as_bytes())?;
        let digest = self.digest_util.compute_path(&input_file)?;
        self.set_metadata(
            input,
            FileArtifactValue::create_for_normal_file(
                Some(DigestUtil::to_binary_digest(&digest)),
                Some(FileContentsProxy::create(&input_file.stat()?)),
                // `content.length()` of the internal string: its number of bytes.
                content.len() as i64,
            ),
        );
        Ok(digest)
    }

    /// `createScratchInputDirectory(input, content)`.
    // Java: com/google/devtools/build/lib/remote/FakeActionInputFileCache.java:147 FakeActionInputFileCache.createScratchInputDirectory
    pub fn create_scratch_input_directory(
        &self,
        input: &ActionInputRef,
        content: &Tree,
    ) -> io::Result<Digest> {
        let input_file = self.exec_root.get_relative(input.get_exec_path());
        input_file.create_directory_and_parents()?;
        let digest = self.digest_util.compute_message(content);
        // `FileArtifactValue.createForDirectoryWithHash(digest)`.
        self.set_metadata(
            input,
            FileArtifactValue::HashedDirectory {
                digest: DigestUtil::to_binary_digest(&digest),
            },
        );
        Ok(digest)
    }

    /// `createScratchInputSymlink(input, target)`.
    // Java: com/google/devtools/build/lib/remote/FakeActionInputFileCache.java:156 FakeActionInputFileCache.createScratchInputSymlink
    pub fn create_scratch_input_symlink(
        &self,
        input: &ActionInputRef,
        target: &str,
    ) -> io::Result<()> {
        blaze_util::check_argument!(input.is_symlink());
        // PORT: `input.getPath()`: blaze_exec's `Artifact` has no path; the artifacts of these
        // tests live under the exec root, so it is the exec path resolved against it.
        let input_file = self.exec_root.get_relative(input.get_exec_path());
        input_file
            .get_parent_directory()
            .expect("NullPointerException: parent directory")
            .create_directory_and_parents()?;
        input_file.create_symbolic_link_to_fragment(&PathFragment::create(target))?;
        // `FileArtifactValue.createForUnresolvedSymlink(input)`
        // (`UnresolvedSymlinkArtifactValue(input.getPath())`): the digest of the target string,
        // its first byte flipped.
        // PORT: blaze_exec has no `create_for_unresolved_symlink`; this is its body
        // (FileArtifactValue.java:1149). `hashString(target, ISO_8859_1)` of an internal string
        // is the hash of its bytes (StringEncoding identity).
        // Java: com/google/devtools/build/lib/actions/FileArtifactValue.java:1149 UnresolvedSymlinkArtifactValue.<init>
        let symlink_target = input_file
            .read_symbolic_link()?
            .get_path_string()
            .to_string();
        let mut digest = self.digest_util.hash(symlink_target.as_bytes());
        digest[0] ^= 0xff;
        self.set_metadata(
            input,
            FileArtifactValue::UnresolvedSymlink {
                target: symlink_target,
                digest,
            },
        );
        Ok(())
    }
}

impl InputMetadataProvider for FakeActionInputFileCache {
    /// `getInputMetadataChecked(input)`.
    // Java: com/google/devtools/build/lib/remote/FakeActionInputFileCache.java:58 FakeActionInputFileCache.getInputMetadataChecked
    fn get_input_metadata(&self, input: &ActionInputRef) -> io::Result<Option<FileArtifactValue>> {
        match self.cas.lock().get(input.get_exec_path()) {
            Some(metadata) => Ok(Some(metadata.clone())),
            None => panic!(
                "NullPointerException: No metadata for input '{}' (exec path: '{}')",
                input,
                input.get_exec_path()
            ),
        }
    }

    // Java: com/google/devtools/build/lib/remote/FakeActionInputFileCache.java:68 FakeActionInputFileCache.getTreeMetadata
    fn get_tree_metadata(&self, input: &ActionInputRef) -> Option<Arc<TreeArtifactValue>> {
        self.trees.lock().get(input).cloned()
    }

    // Java: com/google/devtools/build/lib/remote/FakeActionInputFileCache.java:80 FakeActionInputFileCache.getFileset
    fn get_fileset(&self, _input: &ActionInputRef) -> Option<Arc<FilesetOutputTree>> {
        panic!("UnsupportedOperationException");
    }

    // Java: com/google/devtools/build/lib/remote/FakeActionInputFileCache.java:91 FakeActionInputFileCache.getRunfilesMetadata
    fn get_runfiles_metadata(&self, input: &ActionInputRef) -> Option<Arc<RunfilesArtifactValue>> {
        self.runfiles_map.lock().get(input).cloned()
    }

    // Java: com/google/devtools/build/lib/remote/FakeActionInputFileCache.java:102 FakeActionInputFileCache.getInput
    fn get_input(&self, exec_path: &PathFragment) -> Option<ActionInputRef> {
        self.inputs.lock().get(exec_path).cloned()
    }
}
