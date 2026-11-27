//! A file system for tests that delegates to another one and lets a test replace single
//! operations, the role of the `FileSystem` subclasses Bazel's tests write over an
//! `InMemoryFileSystem` (overridden `statIfFound`, `readdir`, `getInputStream`, `getDigest`,
//! `readSymbolicLink`, ...).
//!
//! A hook returns `Some(result)` to answer the operation itself, or `None` to delegate. Every
//! other operation, the defaulted ones included, is delegated, so a default method of
//! [`FileSystem`] never bypasses a hook through the wrapper's own implementation.

use std::fmt;
use std::io::{self, Read, Write};
use std::sync::Arc;

use crate::digest_hash_function::DigestHashFunction;
use crate::dirent::Dirent;
use crate::file_status::FileStatus;
use crate::file_system::{FileSystem, FileSystemRef};
use crate::path_fragment::PathFragment;

/// A hook of an operation on a path (its path string): `Some` answers it, `None` delegates.
pub type PathHook<R> = Box<dyn for<'a> Fn(&'a str) -> Option<R> + Send + Sync>;

/// A hook of an operation on a path that may follow symbolic links.
pub type PathFollowHook<R> = Box<dyn for<'a> Fn(&'a str, bool) -> Option<R> + Send + Sync>;

/// The operations a test can replace.
#[derive(Default)]
pub struct Hooks {
    /// `stat(path, follow_symlinks)`, also asked by `stat_if_found`, `stat_nullable`,
    /// `exists`, `is_file`, `is_directory` and `is_symbolic_link`.
    pub stat: Option<PathFollowHook<io::Result<FileStatus>>>,
    /// `readdir(path, follow_symlinks)`, also asked by `get_directory_entries`.
    pub readdir: Option<PathFollowHook<io::Result<Vec<Dirent>>>>,
    /// `get_input_stream(path)`.
    pub get_input_stream: Option<PathHook<io::Result<Box<dyn Read + Send>>>>,
    /// `get_digest(path)` (not the fast digest, which Java's tests do not override).
    pub get_digest: Option<PathHook<io::Result<Vec<u8>>>>,
    /// `read_symbolic_link(path)`, also asked by `read_symbolic_link_unchecked` and
    /// `resolve_one_link`.
    pub read_symbolic_link: Option<PathHook<io::Result<PathFragment>>>,
}

/// A [`FileSystem`] that delegates to `delegate` except where a [`Hooks`] entry answers.
pub struct FaultInjectingFileSystem {
    delegate: FileSystemRef,
    hooks: Hooks,
}

impl FaultInjectingFileSystem {
    pub fn new(delegate: FileSystemRef, hooks: Hooks) -> Arc<FaultInjectingFileSystem> {
        Arc::new(FaultInjectingFileSystem { delegate, hooks })
    }

    fn hooked_stat(&self, path: &PathFragment, follow: bool) -> Option<io::Result<FileStatus>> {
        self.hooks
            .stat
            .as_ref()
            .and_then(|h| h(path.get_path_string(), follow))
    }
}

impl fmt::Debug for FaultInjectingFileSystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FaultInjectingFileSystem")
            .field("delegate", &self.delegate)
            .finish_non_exhaustive()
    }
}

impl FileSystem for FaultInjectingFileSystem {
    fn get_digest_function(&self) -> DigestHashFunction {
        self.delegate.get_digest_function()
    }

    fn get_host_file_system(&self) -> Option<FileSystemRef> {
        self.delegate.get_host_file_system()
    }

    fn supports_modifications(&self, path: &PathFragment) -> bool {
        self.delegate.supports_modifications(path)
    }

    fn supports_symbolic_links_natively(&self, path: &PathFragment) -> bool {
        self.delegate.supports_symbolic_links_natively(path)
    }

    fn supports_hard_links_natively(&self, path: &PathFragment) -> bool {
        self.delegate.supports_hard_links_natively(path)
    }

    fn may_be_case_or_normalization_insensitive(&self) -> bool {
        self.delegate.may_be_case_or_normalization_insensitive()
    }

    fn create_directory(&self, path: &PathFragment) -> io::Result<bool> {
        self.delegate.create_directory(path)
    }

    fn create_directory_and_parents(&self, path: &PathFragment) -> io::Result<()> {
        self.delegate.create_directory_and_parents(path)
    }

    fn get_file_size(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<i64> {
        self.delegate.get_file_size(path, follow_symlinks)
    }

    fn delete(&self, path: &PathFragment) -> io::Result<bool> {
        self.delegate.delete(path)
    }

    fn delete_tree(&self, path: &PathFragment) -> io::Result<()> {
        self.delegate.delete_tree(path)
    }

    fn delete_trees_below(&self, dir: &PathFragment) -> io::Result<()> {
        self.delegate.delete_trees_below(dir)
    }

    fn get_last_modified_time(
        &self,
        path: &PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<i64> {
        self.delegate.get_last_modified_time(path, follow_symlinks)
    }

    fn set_last_modified_time(&self, path: &PathFragment, new_time: i64) -> io::Result<()> {
        self.delegate.set_last_modified_time(path, new_time)
    }

    fn get_fast_digest(&self, path: &PathFragment) -> io::Result<Option<Vec<u8>>> {
        self.delegate.get_fast_digest(path)
    }

    fn get_digest(&self, path: &PathFragment) -> io::Result<Vec<u8>> {
        if let Some(r) = self
            .hooks
            .get_digest
            .as_ref()
            .and_then(|h| h(path.get_path_string()))
        {
            return r;
        }
        self.delegate.get_digest(path)
    }

    fn resolve_one_link(&self, path: &PathFragment) -> io::Result<Option<PathFragment>> {
        if let Some(r) = self
            .hooks
            .read_symbolic_link
            .as_ref()
            .and_then(|h| h(path.get_path_string()))
        {
            return r.map(Some);
        }
        self.delegate.resolve_one_link(path)
    }

    fn resolve_symbolic_links(&self, path: &PathFragment) -> io::Result<PathFragment> {
        self.delegate.resolve_symbolic_links(path)
    }

    fn stat(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<FileStatus> {
        if let Some(r) = self.hooked_stat(path, follow_symlinks) {
            return r;
        }
        self.delegate.stat(path, follow_symlinks)
    }

    fn stat_metadata(
        &self,
        path: &PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<Option<std::sync::Arc<dyn crate::file_status::StatMetadata>>> {
        self.delegate.stat_metadata(path, follow_symlinks)
    }

    fn stat_nullable(&self, path: &PathFragment, follow_symlinks: bool) -> Option<FileStatus> {
        match self.hooked_stat(path, follow_symlinks) {
            Some(r) => r.ok(),
            None => self.delegate.stat_nullable(path, follow_symlinks),
        }
    }

    fn stat_if_found(
        &self,
        path: &PathFragment,
        follow_symlinks: bool,
    ) -> io::Result<Option<FileStatus>> {
        match self.hooked_stat(path, follow_symlinks) {
            Some(Ok(status)) => Ok(Some(status)),
            Some(Err(e)) if e.kind() == io::ErrorKind::NotFound => Ok(None),
            Some(Err(e)) => Err(e),
            None => self.delegate.stat_if_found(path, follow_symlinks),
        }
    }

    fn is_file(&self, path: &PathFragment, follow_symlinks: bool) -> bool {
        match self.hooked_stat(path, follow_symlinks) {
            Some(r) => r.is_ok_and(|s| s.is_file()),
            None => self.delegate.is_file(path, follow_symlinks),
        }
    }

    fn is_special_file(&self, path: &PathFragment, follow_symlinks: bool) -> bool {
        self.delegate.is_special_file(path, follow_symlinks)
    }

    fn is_symbolic_link(&self, path: &PathFragment) -> bool {
        match self.hooked_stat(path, false) {
            Some(r) => r.is_ok_and(|s| s.is_symbolic_link()),
            None => self.delegate.is_symbolic_link(path),
        }
    }

    fn is_directory(&self, path: &PathFragment, follow_symlinks: bool) -> bool {
        match self.hooked_stat(path, follow_symlinks) {
            Some(r) => r.is_ok_and(|s| s.is_directory()),
            None => self.delegate.is_directory(path, follow_symlinks),
        }
    }

    fn create_symbolic_link(
        &self,
        link_path: &PathFragment,
        target_fragment: &PathFragment,
    ) -> io::Result<()> {
        self.delegate
            .create_symbolic_link(link_path, target_fragment)
    }

    fn read_symbolic_link(&self, path: &PathFragment) -> io::Result<PathFragment> {
        if let Some(r) = self
            .hooks
            .read_symbolic_link
            .as_ref()
            .and_then(|h| h(path.get_path_string()))
        {
            return r;
        }
        self.delegate.read_symbolic_link(path)
    }

    fn read_symbolic_link_unchecked(&self, path: &PathFragment) -> io::Result<PathFragment> {
        if let Some(r) = self
            .hooks
            .read_symbolic_link
            .as_ref()
            .and_then(|h| h(path.get_path_string()))
        {
            return r;
        }
        self.delegate.read_symbolic_link_unchecked(path)
    }

    fn exists(&self, path: &PathFragment, follow_symlinks: bool) -> bool {
        match self.hooked_stat(path, follow_symlinks) {
            Some(r) => r.is_ok(),
            None => self.delegate.exists(path, follow_symlinks),
        }
    }

    fn get_directory_entries(&self, path: &PathFragment) -> io::Result<Vec<String>> {
        if let Some(r) = self
            .hooks
            .readdir
            .as_ref()
            .and_then(|h| h(path.get_path_string(), false))
        {
            return r.map(|dirents| dirents.iter().map(|d| d.get_name().to_owned()).collect());
        }
        self.delegate.get_directory_entries(path)
    }

    fn readdir(&self, path: &PathFragment, follow_symlinks: bool) -> io::Result<Vec<Dirent>> {
        if let Some(r) = self
            .hooks
            .readdir
            .as_ref()
            .and_then(|h| h(path.get_path_string(), follow_symlinks))
        {
            return r;
        }
        self.delegate.readdir(path, follow_symlinks)
    }

    fn is_readable(&self, path: &PathFragment) -> io::Result<bool> {
        self.delegate.is_readable(path)
    }

    fn set_readable(&self, path: &PathFragment, readable: bool) -> io::Result<()> {
        self.delegate.set_readable(path, readable)
    }

    fn is_writable(&self, path: &PathFragment) -> io::Result<bool> {
        self.delegate.is_writable(path)
    }

    fn set_writable(&self, path: &PathFragment, writable: bool) -> io::Result<()> {
        self.delegate.set_writable(path, writable)
    }

    fn is_executable(&self, path: &PathFragment) -> io::Result<bool> {
        self.delegate.is_executable(path)
    }

    fn set_executable(&self, path: &PathFragment, executable: bool) -> io::Result<()> {
        self.delegate.set_executable(path, executable)
    }

    fn chmod(&self, path: &PathFragment, mode: u32) -> io::Result<()> {
        self.delegate.chmod(path, mode)
    }

    fn get_input_stream(&self, path: &PathFragment) -> io::Result<Box<dyn Read + Send>> {
        if let Some(r) = self
            .hooks
            .get_input_stream
            .as_ref()
            .and_then(|h| h(path.get_path_string()))
        {
            return r;
        }
        self.delegate.get_input_stream(path)
    }

    fn get_output_stream(
        &self,
        path: &PathFragment,
        append: bool,
        internal: bool,
    ) -> io::Result<Box<dyn Write + Send>> {
        self.delegate.get_output_stream(path, append, internal)
    }

    fn rename_to(&self, source_path: &PathFragment, target_path: &PathFragment) -> io::Result<()> {
        self.delegate.rename_to(source_path, target_path)
    }

    fn create_hard_link(
        &self,
        link_path: &PathFragment,
        original_path: &PathFragment,
    ) -> io::Result<()> {
        self.delegate.create_hard_link(link_path, original_path)
    }

    fn create_fs_dependent_hard_link(
        &self,
        link_path: &PathFragment,
        original_path: &PathFragment,
    ) -> io::Result<()> {
        self.delegate
            .create_fs_dependent_hard_link(link_path, original_path)
    }

    fn create_temp_directory(
        &self,
        parent: &PathFragment,
        prefix: &str,
    ) -> io::Result<PathFragment> {
        self.delegate.create_temp_directory(parent, prefix)
    }
}
