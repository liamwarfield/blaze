//! `ActionOutputMetadataStoreTest.fileArtifactValueForSymlink_readFromCache`, the one test of
//! that class that uses `DigestUtils`'s process-wide digest cache.
//!
//! PORT: the other methods of the class are in `src/action_output_metadata_store_tests.rs`.
//! This one has its own test binary: the cache is a process static, and tests of the same binary
//! run in parallel (Java runs a class's tests one after the other), so another test's digests
//! would evict the one entry of a cache of size 1 or add hits.

use std::any::Any;
use std::fmt;
use std::hash::{DefaultHasher, Hash, Hasher};

use blaze_actions::actions::action_lookup::ActionLookupKey;
use blaze_actions::{ActionLookupKeyRef, Artifact, ArtifactRoot, RootType};
use blaze_cmdline::Label;
use blaze_exec::actions::FileArtifactValue;
use blaze_exec::actions::action_adapters::artifact_input;
use blaze_exec::actions::cache::OutputMetadataStore;
use blaze_exec::actions::cache::digest_utils::DigestUtils;
use blaze_sky_exec::skyframe_action_executor::{ActionOutputMetadataStore, OutputPermissions};
use blaze_vfs::{DigestHashFunction, FileSystemExt, FileSystemRef, Path, UnixFileSystem};

/// `ActionsTestUtil.NULL_ARTIFACT_OWNER`.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct NullArtifactOwner;

impl fmt::Display for NullArtifactOwner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("NULL_ARTIFACT_OWNER")
    }
}

impl ActionLookupKey for NullArtifactOwner {
    fn get_label(&self) -> Option<Label> {
        Some(Label::parse_canonical("//null/action:owner").unwrap())
    }
    fn get_configuration_checksum(&self) -> Option<String> {
        None
    }
    fn function_name(&self) -> &'static str {
        "NULL_ARTIFACT_OWNER"
    }
    fn key_equals(&self, other: &dyn ActionLookupKey) -> bool {
        other.as_any().is::<NullArtifactOwner>()
    }
    fn key_hash(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.hash(&mut h);
        h.finish()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// `ActionsTestUtil.createArtifactWithRootRelativePath(outputRoot, path)`.
fn create_artifact_with_root_relative_path(root: &ArtifactRoot, path: &str) -> Artifact {
    let exec_path = root.get_exec_path().get_relative_str(path);
    Artifact::create_derived_artifact(
        root.clone(),
        exec_path,
        ActionLookupKeyRef::new(NullArtifactOwner),
    )
}

fn get_output_metadata(
    store: &ActionOutputMetadataStore,
    artifact: &Artifact,
) -> FileArtifactValue {
    store
        .get_output_metadata(&artifact_input(artifact))
        .unwrap_or_else(|e| panic!("{e:?}"))
        .unwrap()
}

// Java: src/test/java/com/google/devtools/build/lib/skyframe/ActionOutputMetadataStoreTest.java:755 ActionOutputMetadataStoreTest.fileArtifactValueForSymlink_readFromCache
#[test]
fn file_artifact_value_for_symlink_read_from_cache() {
    // createRootDirs()
    let dir = tempfile::tempdir().unwrap();
    let fs: FileSystemRef = UnixFileSystem::create(DigestHashFunction::Sha256);
    let exec_root = fs.get_path(&format!("{}/workspace", dir.path().to_str().unwrap()));
    let output_root = ArtifactRoot::as_derived_root(&exec_root, RootType::OUTPUT, &["out"]);
    output_root
        .get_root()
        .as_path()
        .unwrap()
        .create_directory_and_parents()
        .unwrap();

    DigestUtils::configure_cache(1);
    let target = create_artifact_with_root_relative_path(&output_root, "target");
    let target_path: Path = target.get_path();
    std::fs::write(target_path.get_path_string(), "contents\n").unwrap();
    let symlink = create_artifact_with_root_relative_path(&output_root, "symlink");
    symlink
        .get_path()
        .create_symbolic_link(&target.get_path())
        .unwrap();
    let store = ActionOutputMetadataStore::create(
        /* archived_tree_artifacts_enabled= */ false,
        OutputPermissions::READONLY,
        vec![target.clone(), symlink.clone()],
    );
    let target_metadata = get_output_metadata(&store, &target);
    assert_eq!(DigestUtils::get_cache_stats().hit_count, 0);

    let symlink_metadata = get_output_metadata(&store, &symlink);

    assert_eq!(symlink_metadata, target_metadata);
    assert_eq!(DigestUtils::get_cache_stats().hit_count, 1);

    // The store made the outputs read-only; let the temporary directory be deleted.
    make_writable(dir.path());
}

fn make_writable(dir: &std::path::Path) {
    use std::os::unix::fs::PermissionsExt;
    let Ok(metadata) = std::fs::symlink_metadata(dir) else {
        return;
    };
    if !metadata.is_dir() {
        return;
    }
    let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755));
    if let Ok(entries) = std::fs::read_dir(dir) {
        for entry in entries.flatten() {
            make_writable(&entry.path());
        }
    }
}
