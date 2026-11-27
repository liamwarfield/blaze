//! Port of Bazel's `com.google.devtools.build.lib.vfs` (the parts later crates need).
//!
//! - [`PathFragment`]: normalized absolute or relative paths, pure string operations.
//! - [`os_path_policy`]: the Unix path policy. Windows-only code is not ported.
//! - [`Root`], [`RootedPath`]: package-path roots and paths relative to them.
//! - [`FileSystem`] (trait), [`Path`], and [`UnixFileSystem`], the real file system via `std`.
//!
//! Not ported: serialization codecs, interning, `InMemoryFileSystem`, globbing, xattrs.

pub mod detailed_io_exception;
pub mod digest_hash_function;
pub mod dirent;
pub mod file_status;
pub mod file_system;
pub mod modified_file_set;
pub mod os_path_policy;
pub mod path;
pub mod path_fragment;
pub mod root;
pub mod rooted_path;
pub mod symlinks;
pub mod unix_file_system;

pub use detailed_io_exception::DetailedIOException;
pub use digest_hash_function::{DigestHashFunction, DigestHasher};
pub use dirent::{Dirent, DirentType};
pub use file_status::{FileStatus, StatMetadata};
pub use modified_file_set::ModifiedFileSet;
pub use file_system::{
    FileSymlinkLoopException, FileSystem, FileSystemExt, FileSystemRef, MAX_SYMLINKS,
    NotASymlinkException,
};
pub use path::{NOW_SENTINEL_TIME, Path};
pub use path_fragment::{InvalidBaseNameException, PathFragment, PathSegmentIterator};
pub use root::Root;
pub use rooted_path::RootedPath;
pub use symlinks::Symlinks;
pub use unix_file_system::UnixFileSystem;
