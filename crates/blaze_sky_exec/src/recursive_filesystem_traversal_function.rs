//! Port of `RecursiveFilesystemTraversalFunction`, its value (`RecursiveFilesystemTraversalValue`,
//! `ResolvedFile`), its key (`TraversalRequest` and its only production implementation,
//! `ArtifactFunction.DirectoryArtifactTraversalRequest`) and `TrackSourceDirectoriesFlag`.
//!
//! `ArtifactFunction` requests a traversal for every source artifact that is a directory, because
//! Bazel tracks source directories by default: the directory's metadata is a hash of every file
//! under it.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use blaze_actions::Artifact;
use blaze_cmdline::{PackageIdentifier, RepositoryName};
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_exec::actions::FileArtifactValue;
use blaze_packages::exceptions::{
    FailureCode, NoSuchPackageException, NoSuchPackageKind, PackageLoadingCode,
};
use blaze_sky_loading::actions::{FileStateValue, FileValue};
use blaze_sky_loading::directory_listing::DirectoryListingKey;
use blaze_sky_loading::file::FileKey;
use blaze_sky_loading::file_chain_uniqueness::FileSymlinkInfiniteExpansionUniquenessKey;
use blaze_sky_loading::io::IOException;
use blaze_sky_loading::package_lookup::PackageLookupKey;
use blaze_sky_loading::precomputed::STARLARK_SEMANTICS;
use blaze_skyframe::{
    ComputeError, Environment, Key, SkyFunctionException, SkyFunctionName, SkyKey,
};
use blaze_util::fingerprint::Fingerprint;
use blaze_vfs::{DirentType, Path, PathFragment, Root, RootedPath};

/// A flag to enable / disable tracking of source directories.
///
/// Note that Bazel and Blaze (Google's) have different defaults; this is Bazel's.
// Java: com/google/devtools/build/lib/skyframe/TrackSourceDirectoriesFlag.java:23 TrackSourceDirectoriesFlag
pub struct TrackSourceDirectoriesFlag;

impl TrackSourceDirectoriesFlag {
    /// The default value, which differs between Bazel and Blaze (rewritten by Copybara).
    // Java: com/google/devtools/build/lib/skyframe/TrackSourceDirectoriesFlag.java:26 TrackSourceDirectoriesFlag.TRACK_SOURCE_DIRECTORIES_DEFAULT
    const TRACK_SOURCE_DIRECTORIES_DEFAULT: bool = true;

    /// The effective value.
    ///
    /// PORT: Java reads the JVM system property `BAZEL_TRACK_SOURCE_DIRECTORIES` ("0" false,
    /// "1" true, anything else the default), which a user can only set through the startup
    /// option `--host_jvm_args=-DBAZEL_TRACK_SOURCE_DIRECTORIES=...`. Blaze has no JVM and no
    /// system properties, so the effective value is always the documented default; the client
    /// must refuse that `--host_jvm_args` (StartupOptions.unported_host_jvm_arg_error, owned by
    /// blaze_client) rather than ignore it.
    // Java: com/google/devtools/build/lib/skyframe/TrackSourceDirectoriesFlag.java:30 TrackSourceDirectoriesFlag.TRACK_SOURCE_DIRECTORIES
    // Java: com/google/devtools/build/lib/skyframe/TrackSourceDirectoriesFlag.java:37 TrackSourceDirectoriesFlag.trackSourceDirectories
    pub fn track_source_directories() -> bool {
        Self::TRACK_SOURCE_DIRECTORIES_DEFAULT
    }
}

/// `SkyFunctions.RECURSIVE_FILESYSTEM_TRAVERSAL`.
pub const RECURSIVE_FILESYSTEM_TRAVERSAL: SkyFunctionName =
    SkyFunctionName("RECURSIVE_FILESYSTEM_TRAVERSAL");

/// The root of a traversal: a root and a path under it.
///
/// PORT: Java's `getOutputArtifact()` is always null for the only `TraversalRequest` that Bazel
/// creates (`DirectoryArtifactTraversalRequest`, built with `forRootedPath`), so the field is
/// not carried.
// Java: com/google/devtools/build/lib/skyframe/TraversalRequest.java:126 TraversalRequest.DirectTraversalRoot
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct DirectTraversalRoot {
    root_part: Root,
    relative_part: PathFragment,
}

impl DirectTraversalRoot {
    pub fn get_root_part(&self) -> &Root {
        &self.root_part
    }

    pub fn get_relative_part(&self) -> &PathFragment {
        &self.relative_part
    }

    // Java: com/google/devtools/build/lib/skyframe/TraversalRequest.java:135 DirectTraversalRoot.asPath
    pub fn as_path(&self) -> Path {
        self.root_part.get_relative(&self.relative_part)
    }

    // Java: com/google/devtools/build/lib/skyframe/TraversalRequest.java:139 DirectTraversalRoot.asRootedPath
    pub fn as_rooted_path(&self) -> RootedPath {
        RootedPath::to_rooted_path(self.root_part.clone(), &self.relative_part)
    }

    // Java: com/google/devtools/build/lib/skyframe/TraversalRequest.java:167 DirectTraversalRoot.forRootedPath
    pub fn for_rooted_path(rooted_path: &RootedPath) -> DirectTraversalRoot {
        Self::for_root_and_path(
            rooted_path.get_root().clone(),
            rooted_path.get_root_relative_path().clone(),
        )
    }

    // Java: com/google/devtools/build/lib/skyframe/TraversalRequest.java:171 DirectTraversalRoot.forRootAndPath
    pub fn for_root_and_path(root_part: Root, relative_part: PathFragment) -> DirectTraversalRoot {
        DirectTraversalRoot {
            root_part,
            relative_part,
        }
    }
}

impl fmt::Display for DirectTraversalRoot {
    // AutoValue toString.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "DirectTraversalRoot{{outputArtifact=null, rootPart={}, relativePart={}}}",
            self.root_part, self.relative_part
        )
    }
}

/// Key for depending on all files under a source directory. Only requested when
/// [tracking source directories](TrackSourceDirectoriesFlag).
///
/// It is Bazel's only production `TraversalRequest`, so the abstract class and this subclass are
/// one type: `isRootGenerated()` is false, `strictOutputFiles()` and `emitEmptyDirectoryNodes()`
/// are true.
// Java: com/google/devtools/build/lib/skyframe/ArtifactFunction.java:570 ArtifactFunction.DirectoryArtifactTraversalRequest
// Java: com/google/devtools/build/lib/skyframe/TraversalRequest.java:38 TraversalRequest
#[derive(Clone)]
pub struct DirectoryArtifactTraversalRequest {
    root: DirectTraversalRoot,
    skip_testing_for_subpackage: bool,
    /// Only for error info: not part of equality or the hash code.
    artifact: Artifact,
}

impl DirectoryArtifactTraversalRequest {
    // Java: com/google/devtools/build/lib/skyframe/ArtifactFunction.java:575 DirectoryArtifactTraversalRequest.create
    pub fn create(
        root: DirectTraversalRoot,
        skip_testing_for_subpackage: bool,
        artifact: Artifact,
    ) -> DirectoryArtifactTraversalRequest {
        DirectoryArtifactTraversalRequest {
            root,
            skip_testing_for_subpackage,
            artifact,
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/ArtifactFunction.java:593 DirectoryArtifactTraversalRequest.root
    pub fn root(&self) -> &DirectTraversalRoot {
        &self.root
    }

    // Java: com/google/devtools/build/lib/skyframe/ArtifactFunction.java:598 DirectoryArtifactTraversalRequest.isRootGenerated
    fn is_root_generated(&self) -> bool {
        false
    }

    // Java: com/google/devtools/build/lib/skyframe/ArtifactFunction.java:603 DirectoryArtifactTraversalRequest.strictOutputFiles
    fn strict_output_files(&self) -> bool {
        true
    }

    // Java: com/google/devtools/build/lib/skyframe/ArtifactFunction.java:608 DirectoryArtifactTraversalRequest.skipTestingForSubpackage
    pub fn skip_testing_for_subpackage(&self) -> bool {
        self.skip_testing_for_subpackage
    }

    // Java: com/google/devtools/build/lib/skyframe/ArtifactFunction.java:613 DirectoryArtifactTraversalRequest.emitEmptyDirectoryNodes
    fn emit_empty_directory_nodes(&self) -> bool {
        true
    }

    // Java: com/google/devtools/build/lib/skyframe/ArtifactFunction.java:618 DirectoryArtifactTraversalRequest.errorInfo
    fn error_info(&self) -> String {
        format!("Directory artifact {}", self.artifact.pretty_print())
    }

    // Java: com/google/devtools/build/lib/skyframe/ArtifactFunction.java:623 DirectoryArtifactTraversalRequest.duplicateWithOverrides
    fn duplicate_with_overrides(
        &self,
        new_root: DirectTraversalRoot,
        new_skip_testing_for_subpackage: bool,
    ) -> DirectoryArtifactTraversalRequest {
        Self::create(
            new_root,
            new_skip_testing_for_subpackage,
            self.artifact.clone(),
        )
    }

    // Java: com/google/devtools/build/lib/skyframe/TraversalRequest.java:73 TraversalRequest.forChildEntry
    fn for_child_entry(&self, child: &str) -> DirectoryArtifactTraversalRequest {
        let new_traversal_root = DirectTraversalRoot::for_root_and_path(
            self.root.get_root_part().clone(),
            self.root.get_relative_part().get_relative_str(child),
        );
        self.duplicate_with_overrides(
            new_traversal_root,
            /* skip_testing_for_subpackage= */ false,
        )
    }

    // Java: com/google/devtools/build/lib/skyframe/TraversalRequest.java:80 TraversalRequest.forChangedRootPath
    fn for_changed_root_path(&self, new_root: Root) -> DirectoryArtifactTraversalRequest {
        let new_traversal_root =
            DirectTraversalRoot::for_root_and_path(new_root, self.root.get_relative_part().clone());
        self.duplicate_with_overrides(new_traversal_root, self.skip_testing_for_subpackage)
    }
}

impl PartialEq for DirectoryArtifactTraversalRequest {
    // Java: com/google/devtools/build/lib/skyframe/ArtifactFunction.java:640 DirectoryArtifactTraversalRequest.equals
    fn eq(&self, other: &Self) -> bool {
        // Artifact is only for error info and not considered in hash code or equality.
        self.root == other.root
            && self.skip_testing_for_subpackage == other.skip_testing_for_subpackage
    }
}

impl Eq for DirectoryArtifactTraversalRequest {}

impl Hash for DirectoryArtifactTraversalRequest {
    // Java: com/google/devtools/build/lib/skyframe/ArtifactFunction.java:634 DirectoryArtifactTraversalRequest.hashCode
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.root.hash(state);
        self.skip_testing_for_subpackage.hash(state);
    }
}

impl fmt::Debug for DirectoryArtifactTraversalRequest {
    // Java: com/google/devtools/build/lib/skyframe/TraversalRequest.java:92 TraversalRequest.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "DirectoryArtifactTraversalRequest{{root={}, isRootGenerated={}, \
             strictOutputFiles={}, skipTestingForSubpackage={}, errorInfo={}}}",
            self.root,
            self.is_root_generated(),
            self.strict_output_files(),
            self.skip_testing_for_subpackage,
            self.error_info()
        )
    }
}

impl Key for DirectoryArtifactTraversalRequest {
    type Value = RecursiveFilesystemTraversalValue;
    const FUNCTION: SkyFunctionName = RECURSIVE_FILESYSTEM_TRAVERSAL;
}

/// A value that has a digest (`HasDigest`): a `FileArtifactValue` or a `ByteStringDigest`.
///
/// Only the digest takes part in hashing (equal values have equal digests).
// Java: com/google/devtools/build/lib/actions/HasDigest.java:21 HasDigest
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HasDigest {
    FileArtifactValue(FileArtifactValue),
    // Java: com/google/devtools/build/lib/actions/HasDigest.java:27 HasDigest.ByteStringDigest
    ByteStringDigest(Vec<u8>),
}

impl HasDigest {
    // Java: com/google/devtools/build/lib/actions/HasDigest.java:24 HasDigest.EMPTY
    pub fn empty() -> HasDigest {
        HasDigest::ByteStringDigest(Vec::new())
    }

    /// `getDigest()`.
    pub fn get_digest(&self) -> Option<&[u8]> {
        match self {
            HasDigest::FileArtifactValue(v) => v.get_digest(),
            HasDigest::ByteStringDigest(bytes) => Some(bytes),
        }
    }
}

impl Hash for HasDigest {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.get_digest().hash(state);
    }
}

impl fmt::Display for HasDigest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            HasDigest::FileArtifactValue(v) => write!(
                f,
                "{}",
                blaze_exec::actions::cache::metadata_digest_utils::file_artifact_value_to_string(v)
            ),
            // Java: HasDigest.ByteStringDigest.toString is Object's; not reached by output.
            HasDigest::ByteStringDigest(bytes) => {
                write!(f, "ByteStringDigest({})", hex::encode(bytes))
            }
        }
    }
}

/// Type information about the filesystem entry residing at a path.
// Java: com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalValue.java:96 RecursiveFilesystemTraversalValue.FileType
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum FileType {
    /// A regular file.
    File,
    /// A symlink to a regular file.
    SymlinkToFile,
    /// A directory.
    Directory,
    /// A symlink to a directory.
    SymlinkToDirectory,
    /// A dangling symlink, i.e. one whose target is known not to exist.
    DanglingSymlink,
    /// A path that does not exist or should be ignored.
    Nonexistent,
}

impl FileType {
    pub fn is_file(self) -> bool {
        match self {
            FileType::File | FileType::SymlinkToFile => true,
            FileType::DanglingSymlink => panic!("UnsupportedOperationException"),
            _ => false,
        }
    }

    pub fn is_directory(self) -> bool {
        match self {
            FileType::Directory | FileType::SymlinkToDirectory => true,
            FileType::DanglingSymlink => panic!("UnsupportedOperationException"),
            _ => false,
        }
    }

    pub fn is_symlink(self) -> bool {
        matches!(
            self,
            FileType::SymlinkToFile | FileType::SymlinkToDirectory | FileType::DanglingSymlink
        )
    }

    pub fn exists(self) -> bool {
        matches!(
            self,
            FileType::File
                | FileType::SymlinkToFile
                | FileType::Directory
                | FileType::SymlinkToDirectory
        )
    }
}

impl fmt::Display for FileType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            FileType::File => "<f>",
            FileType::SymlinkToFile => "<lf>",
            FileType::Directory => "<d>",
            FileType::SymlinkToDirectory => "<ld>",
            FileType::DanglingSymlink => "<l?>",
            FileType::Nonexistent => "<?>",
        })
    }
}

/// Path and type information about a single file or symlink.
// Java: com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalValue.java:323 RecursiveFilesystemTraversalValue.ResolvedFile
// Java: com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalValue.java:376 RegularFile
// Java: com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalValue.java:422 Directory
// Java: com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalValue.java:466 DanglingSymlink
// Java: com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalValue.java:512 SymlinkToFile
// Java: com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalValue.java:567 SymlinkToDirectory
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub enum ResolvedFile {
    RegularFile {
        path: RootedPath,
        metadata: HasDigest,
    },
    Directory {
        path: RootedPath,
    },
    DanglingSymlink {
        link_name: RootedPath,
        unresolved_link_target: PathFragment,
        metadata: HasDigest,
    },
    SymlinkToFile {
        target_path: RootedPath,
        link_name: RootedPath,
        unresolved_link_target: PathFragment,
        metadata: HasDigest,
    },
    SymlinkToDirectory {
        target_path: RootedPath,
        link_name: RootedPath,
        unresolved_link_target: PathFragment,
        metadata: HasDigest,
    },
}

impl ResolvedFile {
    /// Type of the entity under this path.
    pub fn get_type(&self) -> FileType {
        match self {
            ResolvedFile::RegularFile { .. } => FileType::File,
            ResolvedFile::Directory { .. } => FileType::Directory,
            ResolvedFile::DanglingSymlink { .. } => FileType::DanglingSymlink,
            ResolvedFile::SymlinkToFile { .. } => FileType::SymlinkToFile,
            ResolvedFile::SymlinkToDirectory { .. } => FileType::SymlinkToDirectory,
        }
    }

    /// Path of the file or the symlink itself.
    pub fn get_path(&self) -> &RootedPath {
        match self {
            ResolvedFile::RegularFile { path, .. } | ResolvedFile::Directory { path } => path,
            ResolvedFile::DanglingSymlink { link_name, .. }
            | ResolvedFile::SymlinkToFile { link_name, .. }
            | ResolvedFile::SymlinkToDirectory { link_name, .. } => link_name,
        }
    }

    /// Associated metadata.
    pub fn get_metadata(&self) -> &HasDigest {
        static EMPTY: std::sync::LazyLock<HasDigest> = std::sync::LazyLock::new(HasDigest::empty);
        match self {
            ResolvedFile::Directory { .. } => &EMPTY,
            ResolvedFile::RegularFile { metadata, .. }
            | ResolvedFile::DanglingSymlink { metadata, .. }
            | ResolvedFile::SymlinkToFile { metadata, .. }
            | ResolvedFile::SymlinkToDirectory { metadata, .. } => metadata,
        }
    }

    /// Returns the path of the Fileset-output symlink relative to the output directory: the
    /// root-relative path of the file or of the symlink itself.
    pub fn get_name_in_symlink_tree(&self) -> &PathFragment {
        self.get_path().get_root_relative_path()
    }
}

/// Collection of files found while recursively traversing a path.
// Java: com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalValue.java:52 RecursiveFilesystemTraversalValue
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RecursiveFilesystemTraversalValue {
    /// The root of the traversal; `None` if the root did not exist.
    resolved_root: Option<ResolvedFile>,
    /// The transitive closure of [`ResolvedFile`]s.
    resolved_paths: NestedSet<ResolvedFile>,
}

impl RecursiveFilesystemTraversalValue {
    // Java: com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalValue.java:53 RecursiveFilesystemTraversalValue.EMPTY
    pub fn empty() -> RecursiveFilesystemTraversalValue {
        RecursiveFilesystemTraversalValue {
            resolved_root: None,
            resolved_paths: NestedSetBuilder::empty_set(Order::STABLE_ORDER),
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalValue.java:70 RecursiveFilesystemTraversalValue.of
    fn of(
        resolved_root: ResolvedFile,
        resolved_paths: NestedSet<ResolvedFile>,
    ) -> RecursiveFilesystemTraversalValue {
        if resolved_paths.is_empty() {
            Self::empty()
        } else {
            RecursiveFilesystemTraversalValue {
                resolved_root: Some(resolved_root),
                resolved_paths,
            }
        }
    }

    // Java: com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalValue.java:79 RecursiveFilesystemTraversalValue.of
    fn of_single(single_member: ResolvedFile) -> RecursiveFilesystemTraversalValue {
        RecursiveFilesystemTraversalValue {
            resolved_paths: NestedSetBuilder::create(Order::STABLE_ORDER, [single_member.clone()]),
            resolved_root: Some(single_member),
        }
    }

    /// Returns the root of the traversal; `None` if the root did not exist.
    pub fn get_resolved_root(&self) -> Option<&ResolvedFile> {
        self.resolved_root.as_ref()
    }

    /// Returns the transitive closure of [`ResolvedFile`]s.
    pub fn get_transitive_files(&self) -> &NestedSet<ResolvedFile> {
        &self.resolved_paths
    }
}

// Java: com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalValue.java:558 ResolvedFileFactory.danglingSymlink
fn dangling_symlink(link_name_path: RootedPath, link_value: PathFragment) -> ResolvedFile {
    use sha2::Digest;
    // PORT: `hashString(linkValue.getPathString(), ISO_8859_1)` of the internal (Latin-1) form
    // is the hash of the path's bytes (StringEncoding identity).
    let mut digest = sha2::Sha256::digest(link_value.get_path_string().as_bytes()).to_vec();
    // Ensure that the digest does not collide with that of a regular file.
    digest[0] ^= 1;
    ResolvedFile::DanglingSymlink {
        link_name: link_name_path,
        unresolved_link_target: link_value,
        metadata: HasDigest::ByteStringDigest(digest),
    }
}

/// Categories of errors that prevent normal [`RecursiveFilesystemTraversalFunction`] evaluation.
// Java: com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalFunction.java:96 RecursiveFilesystemTraversalException.Type
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RecursiveFilesystemTraversalExceptionType {
    /// The traversal encountered a subdirectory with a BUILD file but is not allowed to recurse
    /// into it.
    CannotCrossPackageBoundary,
    /// A dangling symlink was dereferenced.
    DanglingSymlink,
    /// A file operation failed.
    FileOperationFailure,
    /// A generated directory's root-relative path conflicts with a package's path.
    GeneratedPathConflict,
    /// A file/directory visited was part of a symlink cycle or infinite expansion.
    SymlinkCycleOrInfiniteExpansion,
    /// The filesystem told us inconsistent information.
    InconsistentFilesystem,
    /// The filesystem threw a `DetailedIOException`.
    DetailedIoException,
    /// A traversal of a source directory was requested.
    CannotTraverseSourceDirectory,
}

/// The exception that `RecursiveFilesystemTraversalFunctionException` wraps.
// Java: com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalFunction.java:90 RecursiveFilesystemTraversalException
#[derive(Clone, Debug)]
pub struct RecursiveFilesystemTraversalException {
    message: String,
    type_: RecursiveFilesystemTraversalExceptionType,
}

impl RecursiveFilesystemTraversalException {
    // Java: com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalFunction.java:132 RecursiveFilesystemTraversalException.<init>
    pub fn new(
        message: String,
        type_: RecursiveFilesystemTraversalExceptionType,
    ) -> RecursiveFilesystemTraversalException {
        blaze_util::check_argument!(
            type_ != RecursiveFilesystemTraversalExceptionType::DetailedIoException
        );
        RecursiveFilesystemTraversalException { message, type_ }
    }

    // Java: com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalFunction.java:139 RecursiveFilesystemTraversalException.getType
    pub fn get_type(&self) -> RecursiveFilesystemTraversalExceptionType {
        self.type_
    }

    /// `getMessage()`.
    pub fn get_message(&self) -> &str {
        &self.message
    }
}

impl fmt::Display for RecursiveFilesystemTraversalException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for RecursiveFilesystemTraversalException {}

// Java: com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalFunction.java:163 RecursiveFilesystemTraversalFunctionException
fn recursive_filesystem_traversal_function_exception(
    e: RecursiveFilesystemTraversalException,
) -> ComputeError {
    SkyFunctionException::persistent(e).into()
}

/// What `compute` catches (`IOException | BuildFileNotFoundException`).
enum TraversalError {
    Io(IOException),
    BuildFileNotFound(NoSuchPackageException),
}

impl TraversalError {
    fn get_message(&self) -> String {
        match self {
            TraversalError::Io(e) => e.get_message().to_owned(),
            TraversalError::BuildFileNotFound(e) => e.get_message(),
        }
    }
}

/// A step either completes (`Ok(Ok(Some))`), is missing a dependency (`Ok(Ok(None))`), throws an
/// exception that `compute` catches (`Ok(Err)`) or ends the function (`Err`: interrupted or a
/// `RecursiveFilesystemTraversalFunctionException`).
type Step<T> = Result<Result<Option<T>, TraversalError>, ComputeError>;

/// Information about a file or directory under the traversal.
// Java: com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalFunction.java:304 RecursiveFilesystemTraversalFunction.FileInfo
#[derive(Clone)]
struct FileInfo {
    type_: FileType,
    metadata: HasDigest,
    real_path: Option<RootedPath>,
    unresolved_symlink_target: Option<PathFragment>,
}

impl FileInfo {
    fn new(
        type_: FileType,
        metadata: HasDigest,
        real_path: Option<RootedPath>,
        unresolved_symlink_target: Option<PathFragment>,
    ) -> FileInfo {
        if metadata.get_digest().is_none() {
            panic!("NullPointerException: {metadata}");
        }
        FileInfo {
            type_,
            metadata,
            real_path,
            unresolved_symlink_target,
        }
    }
}

impl fmt::Display for FileInfo {
    // Java: com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalFunction.java:322 FileInfo.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let real_path = match &self.real_path {
            Some(p) => p.to_string(),
            None => "null".to_owned(),
        };
        if self.type_.is_symlink() {
            write!(
                f,
                "({}: link_value={}, real_path={})",
                self.type_,
                self.unresolved_symlink_target
                    .as_ref()
                    .map(|p| p.get_path_string().to_owned())
                    .unwrap_or_default(),
                real_path
            )
        } else {
            write!(f, "({}: real_path={})", self.type_, real_path)
        }
    }
}

/// The result of [`RecursiveFilesystemTraversalFunction::check_if_package`].
// Java: com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalFunction.java:496 RecursiveFilesystemTraversalFunction.PkgLookupResult
struct PkgLookupResult {
    type_: PkgLookupResultType,
    traversal: DirectoryArtifactTraversalRequest,
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum PkgLookupResultType {
    #[allow(dead_code)]
    Conflict,
    Directory,
    Pkg,
}

impl PkgLookupResult {
    fn is_package(&self) -> bool {
        self.type_ == PkgLookupResultType::Pkg
    }

    fn is_conflicting(&self) -> bool {
        self.type_ == PkgLookupResultType::Conflict
    }
}

/// A SkyFunction to build [`RecursiveFilesystemTraversalValue`]s.
// Java: com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalFunction.java:79 RecursiveFilesystemTraversalFunction
pub struct RecursiveFilesystemTraversalFunction {
    /// `<output_base>/external`.
    external_directory: Path,
}

impl RecursiveFilesystemTraversalFunction {
    /// PORT: the `SyscallCache` parameter is not ported (Blaze always stats; see
    /// `FileStateValue.create`).
    // Java: com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalFunction.java:173 RecursiveFilesystemTraversalFunction.<init>
    pub fn new(external_directory: Path) -> RecursiveFilesystemTraversalFunction {
        RecursiveFilesystemTraversalFunction { external_directory }
    }

    // Java: com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalFunction.java:180 RecursiveFilesystemTraversalFunction.compute
    pub fn compute(
        &self,
        traversal: &DirectoryArtifactTraversalRequest,
        env: &mut Environment<'_>,
    ) -> Result<Option<RecursiveFilesystemTraversalValue>, ComputeError> {
        let mut traversal = traversal.clone();
        // PORT: skipped, profiling (Profiler FILESYSTEM_TRAVERSAL).
        let e = match self.compute_internal(&mut traversal, env)? {
            Ok(v) => return Ok(v),
            Err(e) => e,
        };
        let message = format!(
            "Error while traversing directory {}: {}",
            traversal.root().get_relative_part(),
            e.get_message()
        );

        // PORT: Blaze's file systems throw no DetailedIOException.

        // Trying to stat the starting point of this root may have failed with a symlink cycle or
        // trying to get a package lookup value may have failed due to a symlink cycle.
        let mut exception_type = RecursiveFilesystemTraversalExceptionType::FileOperationFailure;
        if let TraversalError::Io(io) = &e {
            if io.is_inconsistent_filesystem() {
                exception_type = RecursiveFilesystemTraversalExceptionType::InconsistentFilesystem;
            }
            if io.is_file_symlink_exception() {
                exception_type =
                    RecursiveFilesystemTraversalExceptionType::SymlinkCycleOrInfiniteExpansion;
            }
        }
        if let TraversalError::BuildFileNotFound(bfnf) = &e {
            // `e instanceof DetailedException`: a BuildFileNotFoundException.
            let code = bfnf.get_detailed_exit_code().code;
            if code
                == FailureCode::PackageLoading(PackageLoadingCode::SymlinkCycleOrInfiniteExpansion)
            {
                exception_type =
                    RecursiveFilesystemTraversalExceptionType::SymlinkCycleOrInfiniteExpansion;
            }
        }
        Err(recursive_filesystem_traversal_function_exception(
            RecursiveFilesystemTraversalException::new(message, exception_type),
        ))
    }

    /// The `try` block of `compute`. `traversal` is updated as Java's local variable is, for the
    /// `catch` block's message.
    fn compute_internal(
        &self,
        traversal: &mut DirectoryArtifactTraversalRequest,
        env: &mut Environment<'_>,
    ) -> Step<RecursiveFilesystemTraversalValue> {
        // Stat the traversal root.
        let root_info = match look_up_file_info(env, traversal)? {
            Ok(Some(info)) => info,
            Ok(None) => return Ok(Ok(None)),
            Err(e) => return Ok(Err(e)),
        };

        if !root_info.type_.exists() {
            // May be a dangling symlink or a non-existent file. Handle gracefully.
            if root_info.type_.is_symlink() {
                return Ok(Ok(Some(RecursiveFilesystemTraversalValue::of_single(
                    dangling_symlink(
                        traversal.root().as_rooted_path(),
                        root_info
                            .unresolved_symlink_target
                            .clone()
                            .expect("unresolved symlink target"),
                    ),
                ))));
            } else {
                return Ok(Ok(Some(RecursiveFilesystemTraversalValue::empty())));
            }
        }

        if root_info.type_.is_file() {
            return Ok(
                result_for_file_root(&traversal.root().as_rooted_path(), &root_info)
                    .map(Some)
                    .map_err(TraversalError::Io),
            );
        }
        // PORT: the `rootInfo.metadata instanceof TreeArtifactValue` branch is unreachable: only
        // generated roots have a TreeArtifactValue, and this request's root is never generated.

        // Otherwise the root is a directory or a symlink to one.
        let pkg_lookup_result = match self.check_if_package(env, traversal, root_info.clone())? {
            Ok(Some(r)) => r,
            Ok(None) => return Ok(Ok(None)),
            Err(e) => return Ok(Err(e)),
        };
        *traversal = pkg_lookup_result.traversal.clone();

        if pkg_lookup_result.is_conflicting() {
            // The traversal was requested for an output directory whose root-relative path
            // conflicts with a source package. We can't handle that, bail out.
            return Err(create_generated_path_conflict_exception(traversal));
        } else if pkg_lookup_result.is_package() && !traversal.skip_testing_for_subpackage() {
            // The traversal was requested for a directory that defines a package which we should
            // not traverse and should complain loudly (display an error).
            let msg = format!(
                "{} crosses package boundary into package rooted at {}",
                traversal.error_info(),
                traversal.root().get_relative_part().get_path_string()
            );
            return Err(recursive_filesystem_traversal_function_exception(
                RecursiveFilesystemTraversalException::new(
                    msg,
                    RecursiveFilesystemTraversalExceptionType::CannotCrossPackageBoundary,
                ),
            ));
        }

        // We are free to traverse this directory.
        let subdir_traversals = match traverse_children(env, traversal)? {
            Ok(Some(v)) => v,
            Ok(None) => return Ok(Ok(None)),
            Err(e) => return Ok(Err(e)),
        };
        Ok(Ok(Some(result_for_directory(
            traversal,
            &root_info,
            &subdir_traversals,
        ))))
    }

    /// Checks whether the `traversal`'s path refers to a package directory.
    ///
    /// Returns the result of the lookup; it contains potentially new [`DirectoryArtifactTraversalRequest`]
    /// and [`FileInfo`] so the caller should use these instead of the old ones (this happens when
    /// a package is found, but under a different root than expected).
    // Java: com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalFunction.java:551 RecursiveFilesystemTraversalFunction.checkIfPackage
    fn check_if_package(
        &self,
        env: &mut Environment<'_>,
        traversal: &DirectoryArtifactTraversalRequest,
        root_info: FileInfo,
    ) -> Step<PkgLookupResult> {
        blaze_util::check_argument!(
            root_info.type_.exists() && !root_info.type_.is_file(),
            "{{{:?}}} {{{}}}",
            traversal,
            root_info
        );
        let root_path = traversal
            .root()
            .get_root_part()
            .as_path()
            .cloned()
            .expect("UnsupportedOperationException: absolute root");
        // The roots of external repositories are always children of the external directory,
        // even if those are symlinks to other locations (e.g. in the case of a
        // local_repository), so all other roots are part of the main repository.
        let repo_name = if root_path.starts_with(&self.external_directory) {
            let Some(semantics) = STARLARK_SEMANTICS.get(env)? else {
                return Ok(Ok(None));
            };
            if !semantics.incompatible_check_external_repo_source_dir_package_boundary {
                return Ok(Ok(Some(PkgLookupResult {
                    type_: PkgLookupResultType::Directory,
                    traversal: traversal.clone(),
                })));
            }
            blaze_util::check_state!(
                root_path
                    .relative_to(&self.external_directory)
                    .is_single_segment(),
                "{} is expected to be directly under {}",
                root_path,
                self.external_directory
            );
            RepositoryName::create_unvalidated(root_path.get_base_name())
        } else {
            blaze_cmdline::repository_name::MAIN.clone()
        };
        // PackageLookupFunction/dependencies can only throw IOException,
        // BuildFileNotFoundException, and RepositoryFetchException, and RepositoryFetchException
        // is not in play here. Note that run-of-the-mill circular symlinks will *not* throw here,
        // and will trigger later errors during the recursive traversal.
        let pkg_lookup = env.get_value_or_throw_with(
            PackageLookupKey::key(PackageIdentifier::create(
                repo_name,
                traversal.root().get_relative_part().clone(),
            )),
            |e| {
                if let Some(nspe) = e.downcast_ref::<NoSuchPackageException>() {
                    if nspe.kind() == NoSuchPackageKind::BuildFileNotFound {
                        return Some(TraversalError::BuildFileNotFound(nspe.clone()));
                    }
                }
                e.downcast_ref::<IOException>()
                    .map(|io| TraversalError::Io(io.clone()))
            },
        )?;
        let pkg_lookup = match pkg_lookup {
            Ok(v) => v,
            Err(e) => return Ok(Err(e)),
        };
        if env.values_missing() {
            return Ok(Ok(None));
        }
        let pkg_lookup = pkg_lookup.expect("values not missing");

        if pkg_lookup.package_exists() {
            if traversal.is_root_generated() {
                // The traversal's root was a generated directory, but its root-relative path
                // conflicts with an existing package.
                Ok(Ok(Some(PkgLookupResult {
                    type_: PkgLookupResultType::Conflict,
                    traversal: traversal.clone(),
                })))
            } else {
                // The traversal's root was a source directory and it defines a package.
                let pkg_root = pkg_lookup.get_root();
                let mut traversal = traversal.clone();
                if pkg_root != traversal.root().get_root_part() {
                    // However the root of this package is different from what we expected.
                    // stat() the real BUILD file of that package.
                    traversal = traversal.for_changed_root_path(pkg_root.clone());
                    let root_info = match look_up_file_info(env, &traversal)? {
                        Ok(Some(info)) => info,
                        // PORT: Java dereferences the null FileInfo (NullPointerException in
                        // the Verify); a missing dependency restarts the function here.
                        Ok(None) => return Ok(Ok(None)),
                        Err(e) => return Ok(Err(e)),
                    };
                    blaze_util::check_state!(
                        root_info.type_.exists(),
                        "VerifyException: {{{:?}}} {{{}}}",
                        traversal,
                        root_info
                    );
                }
                Ok(Ok(Some(PkgLookupResult {
                    type_: PkgLookupResultType::Pkg,
                    traversal,
                })))
            }
        } else {
            // The traversal's root was a directory (source or generated one), no package exists
            // under the same root-relative path.
            Ok(Ok(Some(PkgLookupResult {
                type_: PkgLookupResultType::Directory,
                traversal: traversal.clone(),
            })))
        }
    }
}

// Java: com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalFunction.java:292 RecursiveFilesystemTraversalFunction.createGeneratedPathConflictException
fn create_generated_path_conflict_exception(
    traversal: &DirectoryArtifactTraversalRequest,
) -> ComputeError {
    let message = format!(
        "Generated directory {} conflicts with package under the same path. Additional info: {}",
        traversal.root().get_relative_part().get_path_string(),
        traversal.error_info()
    );
    recursive_filesystem_traversal_function_exception(RecursiveFilesystemTraversalException::new(
        message,
        RecursiveFilesystemTraversalExceptionType::GeneratedPathConflict,
    ))
}

// Java: com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalFunction.java:335 RecursiveFilesystemTraversalFunction.lookUpFileInfo
fn look_up_file_info(
    env: &mut Environment<'_>,
    traversal: &DirectoryArtifactTraversalRequest,
) -> Step<FileInfo> {
    // PORT: the `traversal.isRootGenerated()` branch is unreachable (see
    // DirectoryArtifactTraversalRequest.isRootGenerated).
    // Stat the file.
    let rooted_path = traversal.root().as_rooted_path();
    let file_value =
        env.get_value_or_throw::<FileKey, IOException>(FileKey::key(rooted_path.clone()))?;
    let file_value = match file_value {
        Ok(v) => v,
        Err(e) => return Ok(Err(TraversalError::Io(e))),
    };

    if env.values_missing() {
        return Ok(Ok(None));
    }
    let file_value = file_value.expect("values not missing");
    to_file_info(
        &rooted_path,
        &file_value,
        env,
        Some(&traversal.root().as_path()),
    )
}

// Java: com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalFunction.java:413 RecursiveFilesystemTraversalFunction.toFileInfo
fn to_file_info(
    rooted_path: &RootedPath,
    file_value: &FileValue,
    env: &mut Environment<'_>,
    path: Option<&Path>,
) -> Step<FileInfo> {
    if let Some(chain) = file_value.unbounded_ancestor_symlink_expansion_chain() {
        let uniqueness_key = FileSymlinkInfiniteExpansionUniquenessKey::key(chain);
        env.get_value(uniqueness_key)?;
        if env.values_missing() {
            return Ok(Ok(None));
        }

        return Ok(Err(TraversalError::Io(
            IOException::file_symlink_infinite_expansion(
                file_value
                    .path_to_unbounded_ancestor_symlink_expansion_chain()
                    .map(|p| p.to_vec())
                    .unwrap_or_default(),
                chain.to_vec(),
            ),
        )));
    }

    if !file_value.exists() {
        // If it doesn't exist, or it's a dangling symlink, we still want to handle that
        // gracefully.
        let metadata = match with_digest(file_value.real_file_state_value(), None) {
            Ok(m) => m,
            Err(e) => return Ok(Err(TraversalError::Io(e))),
        };
        return Ok(Ok(Some(FileInfo::new(
            if file_value.is_symlink() {
                FileType::DanglingSymlink
            } else {
                FileType::Nonexistent
            },
            metadata,
            None,
            if file_value.is_symlink() {
                Some(file_value.get_unresolved_link_target().clone())
            } else {
                None
            },
        ))));
    }

    // If it exists, it may either be a symlink or a file/directory.
    let mut unresolved_link_target = None;
    let type_;
    if file_value.is_symlink() {
        unresolved_link_target = Some(file_value.get_unresolved_link_target().clone());
        type_ = if file_value.is_directory() {
            FileType::SymlinkToDirectory
        } else {
            FileType::SymlinkToFile
        };
    } else {
        type_ = if file_value.is_directory() {
            FileType::Directory
        } else {
            FileType::File
        };
    }
    let metadata = match with_digest(file_value.real_file_state_value(), path) {
        Ok(m) => m,
        Err(e) => return Ok(Err(TraversalError::Io(e))),
    };
    Ok(Ok(Some(FileInfo::new(
        type_,
        metadata,
        Some(file_value.real_rooted_path(rooted_path)),
        unresolved_link_target,
    ))))
}

/// Transform the HasDigest to the appropriate type based on the current state of the digest. If
/// fsVal is type RegularFileStateValue or FileArtifactValue and has a valid digest value, then we
/// want to convert it to a new FileArtifactValue type. Otherwise if they are of the two
/// forementioned types but do not have a digest, then we will create a FileArtifactValue using
/// its `Path`. Otherwise we will fingerprint the digest and return it as a new
/// `HasDigest.ByteStringDigest` object.
///
/// PORT: Java's parameter is any `HasDigest`; every caller on the source-directory path passes a
/// `FileStateValue` (the `FileArtifactValue` branch is for generated roots).
// Java: com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalFunction.java:472 RecursiveFilesystemTraversalFunction.withDigest
pub fn with_digest(fs_val: &FileStateValue, path: Option<&Path>) -> Result<HasDigest, IOException> {
    match fs_val {
        FileStateValue::RegularFileWithDigest { size, digest } => Ok(HasDigest::FileArtifactValue(
            FileArtifactValue::create_for_virtual_action_input(digest.clone(), *size),
        )),
        FileStateValue::RegularFileWithContentsProxy { size, .. } => {
            let path = path.unwrap_or_else(|| panic!("NullPointerException: path"));
            FileArtifactValue::create_for_normal_file_using_path(path, *size)
                .map(HasDigest::FileArtifactValue)
                .map_err(|e| IOException::from_io(&e))
        }
        // Java: com/google/devtools/build/lib/actions/FileStateValue.java:291 FileStateValue.getValueFingerprint
        _ => Ok(HasDigest::ByteStringDigest(fs_val.get_value_fingerprint())),
    }
}

/// Creates results for a file or for a symlink that points to one.
///
/// A symlink may be direct (points to a file) or transitive (points at a direct or transitive
/// symlink).
// Java: com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalFunction.java:624 RecursiveFilesystemTraversalFunction.resultForFileRoot
fn result_for_file_root(
    path: &RootedPath,
    info: &FileInfo,
) -> Result<RecursiveFilesystemTraversalValue, IOException> {
    if !info.type_.is_file() || !info.type_.exists() {
        return Err(IOException::inconsistent_filesystem(format!(
            "We were previously told {} was an existing file but it's actually {}",
            path, info
        )));
    }

    if info.type_.is_symlink() {
        Ok(RecursiveFilesystemTraversalValue::of_single(
            ResolvedFile::SymlinkToFile {
                target_path: info.real_path.clone().expect("real path"),
                link_name: path.clone(),
                unresolved_link_target: info
                    .unresolved_symlink_target
                    .clone()
                    .expect("unresolved symlink target"),
                metadata: info.metadata.clone(),
            },
        ))
    } else {
        Ok(RecursiveFilesystemTraversalValue::of_single(
            ResolvedFile::RegularFile {
                path: path.clone(),
                metadata: info.metadata.clone(),
            },
        ))
    }
}

// Java: com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalFunction.java:642 RecursiveFilesystemTraversalFunction.resultForDirectory
fn result_for_directory(
    traversal: &DirectoryArtifactTraversalRequest,
    root_info: &FileInfo,
    subdir_traversals: &[Arc<RecursiveFilesystemTraversalValue>],
) -> RecursiveFilesystemTraversalValue {
    // Collect transitive closure of files in subdirectories.
    let mut paths: NestedSetBuilder<ResolvedFile> = NestedSetBuilder::stable_order();
    for child in subdir_traversals {
        paths.add_transitive(child.get_transitive_files());
    }
    let root;
    if root_info.type_.is_symlink() {
        let children = paths.build();
        root = ResolvedFile::SymlinkToDirectory {
            target_path: root_info.real_path.clone().expect("real path"),
            link_name: traversal.root().as_rooted_path(),
            unresolved_link_target: root_info
                .unresolved_symlink_target
                .clone()
                .expect("unresolved symlink target"),
            metadata: hash_directory_symlink(&children, &root_info.metadata),
        };
        paths = NestedSetBuilder::stable_order();
        paths.add_transitive(&children).add(root.clone());
    } else {
        root = ResolvedFile::Directory {
            path: root_info.real_path.clone().expect("real path"),
        };
        if traversal.emit_empty_directory_nodes() && paths.is_empty() {
            paths.add(root.clone());
        }
    }
    RecursiveFilesystemTraversalValue::of(root, paths.build())
}

// Java: com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalFunction.java:670 RecursiveFilesystemTraversalFunction.hashDirectorySymlink
fn hash_directory_symlink(children: &NestedSet<ResolvedFile>, metadata: &HasDigest) -> HasDigest {
    // If the root is a directory symlink, the associated FileStateValue does not change when the
    // linked directory's contents change, so we can't use the FileStateValue as metadata like we
    // do with other ResolvedFile kinds. Instead we compute a metadata hash from the child
    // elements and return that as the ResolvedFile's metadata hash.
    let mut fp = Fingerprint::new();
    fp.add_bytes(metadata.get_digest().expect("digest"));
    for file in children.to_list().iter() {
        fp.add_string(file.get_name_in_symlink_tree().get_path_string());
        fp.add_bytes(file.get_metadata().get_digest().expect("digest"));
    }
    let result = fp.digest_and_reset();
    HasDigest::ByteStringDigest(result)
}

/// Requests Skyframe to compute the dependent values and returns them.
// Java: com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalFunction.java:688 RecursiveFilesystemTraversalFunction.traverseChildren
fn traverse_children(
    env: &mut Environment<'_>,
    traversal: &DirectoryArtifactTraversalRequest,
) -> Step<Vec<Arc<RecursiveFilesystemTraversalValue>>> {
    // PORT: `traverseGeneratedChildren` is unreachable (the root is never generated).
    traverse_source_children(env, traversal)
}

// Java: com/google/devtools/build/lib/skyframe/RecursiveFilesystemTraversalFunction.java:722 RecursiveFilesystemTraversalFunction.traverseSourceChildren
fn traverse_source_children(
    env: &mut Environment<'_>,
    traversal: &DirectoryArtifactTraversalRequest,
) -> Step<Vec<Arc<RecursiveFilesystemTraversalValue>>> {
    let dir_listing_value = env.get_value_or_throw::<DirectoryListingKey, IOException>(
        DirectoryListingKey::key(traversal.root().as_rooted_path()),
    )?;
    let dir_listing_value = match dir_listing_value {
        Ok(Some(v)) => v,
        Ok(None) => return Ok(Ok(None)),
        Err(e) => return Ok(Err(TraversalError::Io(e))),
    };
    let dirents = dir_listing_value.get_dirents();
    if dirents.iter().len() == 0 {
        return Ok(Ok(Some(Vec::new())));
    }

    enum ChildKey {
        File(FileKey),
        Traversal(DirectoryArtifactTraversalRequest),
    }
    let mut child_keys: Vec<ChildKey> = Vec::new();
    for dirent in dirents {
        let child_request = traversal.for_child_entry(dirent.get_name());
        // For source files, request the FileValue directly instead of another TraversalRequest.
        // This makes the base case of the recursive traversal a directory with no subdirectories
        // instead of a file, greatly reducing the number of skyframe nodes for directories with
        // many files.
        if dirent.get_type() == DirentType::File {
            child_keys.push(ChildKey::File(FileKey::key(
                child_request.root().as_rooted_path(),
            )));
        } else {
            child_keys.push(ChildKey::Traversal(child_request));
        }
    }

    let sky_keys: Vec<SkyKey> = child_keys
        .iter()
        .map(|k| match k {
            ChildKey::File(k) => SkyKey::new(k.clone()),
            ChildKey::Traversal(k) => SkyKey::new(k.clone()),
        })
        .collect();
    let result = env.get_values_and_exceptions(&sky_keys)?;
    let mut child_values: Vec<Arc<RecursiveFilesystemTraversalValue>> =
        Vec::with_capacity(child_keys.len());
    for key in &child_keys {
        match key {
            ChildKey::File(file_key) => {
                let Some(value) = result.get_typed(file_key) else {
                    continue;
                };
                let file_info = match to_file_info(
                    file_key.argument(),
                    &value,
                    env,
                    Some(&file_key.argument().as_path()),
                )? {
                    Ok(info) => info,
                    Err(e) => return Ok(Err(e)),
                };
                if let Some(file_info) = file_info {
                    match result_for_file_root(file_key.argument(), &file_info) {
                        Ok(v) => child_values.push(Arc::new(v)),
                        Err(e) => return Ok(Err(TraversalError::Io(e))),
                    }
                }
            }
            ChildKey::Traversal(request) => {
                let Some(value) = result.get_typed(request) else {
                    continue;
                };
                child_values.push(value);
            }
        }
    }
    Ok(Ok(if env.values_missing() {
        None
    } else {
        Some(child_values)
    }))
}
