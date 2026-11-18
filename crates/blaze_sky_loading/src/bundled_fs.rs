//! The bundled `@_builtins` .bzl files and the read-only in-memory file system Bazel unpacks them
//! into (`/virtual_builtins_bzl`).
//!
//! The files under `crates/blaze_sky_loading/builtins_bzl/` are the `**/*.bzl` files of Bazel
//! 9.3.2's `src/main/starlark/builtins_bzl` (the contents of Java's `builtins_bzl.zip` resource),
//! copied unmodified from the pinned source tree.
//!
//! Java: com/google/devtools/build/lib/analysis/ConfiguredRuleClassProvider.java
//! (`BundledFileSystem`, `unpackBuiltinsBzlZipResource`, `getBundledBuiltinsRoot`).

use std::collections::BTreeMap;
use std::fmt;
use std::io::{self, Read, Write};
use std::sync::{Arc, OnceLock};

use blaze_vfs::{
    DigestHashFunction, FileStatus, FileSystem, FileSystemRef, Path, PathFragment, Root,
};

/// The bundled builtins: path relative to the builtins root, contents.
pub const BUILTINS_BZL_FILES: &[(&str, &str)] = &[
    (
        "bazel/exports.bzl",
        include_str!("../builtins_bzl/bazel/exports.bzl"),
    ),
    (
        "common/builtin_exec_platforms.bzl",
        include_str!("../builtins_bzl/common/builtin_exec_platforms.bzl"),
    ),
    (
        "common/cc/attrs.bzl",
        include_str!("../builtins_bzl/common/cc/attrs.bzl"),
    ),
    (
        "common/cc/cc_binary.bzl",
        include_str!("../builtins_bzl/common/cc/cc_binary.bzl"),
    ),
    (
        "common/cc/cc_common_bazel.bzl",
        include_str!("../builtins_bzl/common/cc/cc_common_bazel.bzl"),
    ),
    (
        "common/cc/cc_common.bzl",
        include_str!("../builtins_bzl/common/cc/cc_common.bzl"),
    ),
    (
        "common/cc/cc_helper.bzl",
        include_str!("../builtins_bzl/common/cc/cc_helper.bzl"),
    ),
    (
        "common/cc/cc_helper_internal.bzl",
        include_str!("../builtins_bzl/common/cc/cc_helper_internal.bzl"),
    ),
    (
        "common/cc/cc_import.bzl",
        include_str!("../builtins_bzl/common/cc/cc_import.bzl"),
    ),
    (
        "common/cc/cc_library.bzl",
        include_str!("../builtins_bzl/common/cc/cc_library.bzl"),
    ),
    (
        "common/cc/cc_shared_library.bzl",
        include_str!("../builtins_bzl/common/cc/cc_shared_library.bzl"),
    ),
    (
        "common/cc/compile/compile_build_variables.bzl",
        include_str!("../builtins_bzl/common/cc/compile/compile_build_variables.bzl"),
    ),
    (
        "common/cc/compile/compile.bzl",
        include_str!("../builtins_bzl/common/cc/compile/compile.bzl"),
    ),
    (
        "common/cc/link/create_extra_link_time_library.bzl",
        include_str!("../builtins_bzl/common/cc/link/create_extra_link_time_library.bzl"),
    ),
    (
        "common/cc/link/lto_backends.bzl",
        include_str!("../builtins_bzl/common/cc/link/lto_backends.bzl"),
    ),
    (
        "common/cc/semantics.bzl",
        include_str!("../builtins_bzl/common/cc/semantics.bzl"),
    ),
    (
        "common/exports.bzl",
        include_str!("../builtins_bzl/common/exports.bzl"),
    ),
    (
        "common/java/java_common.bzl",
        include_str!("../builtins_bzl/common/java/java_common.bzl"),
    ),
    (
        "common/objc/apple_common.bzl",
        include_str!("../builtins_bzl/common/objc/apple_common.bzl"),
    ),
    (
        "common/objc/apple_env.bzl",
        include_str!("../builtins_bzl/common/objc/apple_env.bzl"),
    ),
    (
        "common/objc/apple_platform.bzl",
        include_str!("../builtins_bzl/common/objc/apple_platform.bzl"),
    ),
    (
        "common/objc/apple_toolchain.bzl",
        include_str!("../builtins_bzl/common/objc/apple_toolchain.bzl"),
    ),
    (
        "common/objc/objc_info.bzl",
        include_str!("../builtins_bzl/common/objc/objc_info.bzl"),
    ),
    (
        "common/paths.bzl",
        include_str!("../builtins_bzl/common/paths.bzl"),
    ),
    (
        "common/python/py_internal.bzl",
        include_str!("../builtins_bzl/common/python/py_internal.bzl"),
    ),
    (
        "common/util.bzl",
        include_str!("../builtins_bzl/common/util.bzl"),
    ),
    (
        "common/xcode/providers.bzl",
        include_str!("../builtins_bzl/common/xcode/providers.bzl"),
    ),
    ("exports.bzl", include_str!("../builtins_bzl/exports.bzl")),
];

/// Where Bazel unpacks the bundled builtins.
// Java: com/google/devtools/build/lib/analysis/ConfiguredRuleClassProvider.java:539 ConfiguredRuleClassProvider.Builder.build
pub const BUILTINS_PATH: &str = "/virtual_builtins_bzl";

const S_IFREG: u32 = 0o100000;
const S_IFDIR: u32 = 0o040000;

#[derive(Debug)]
enum Node {
    Directory(Vec<String>),
    File(&'static str),
}

/// An in-memory, read-only file system for bundled builtins .bzl files.
///
/// Java pretends the digest of a bundled file is uniquely determined by its name, not its
/// contents: the contents are guaranteed to not change throughout the lifetime of the server, and
/// different bundled files must have different digests (bzl environment hashes of rule classes
/// defined in two bundled files must differ). So the fast digest of a file is the hash of its
/// path string.
// Java: com/google/devtools/build/lib/analysis/ConfiguredRuleClassProvider.java:105 ConfiguredRuleClassProvider.BundledFileSystem
pub struct BundledFileSystem {
    nodes: BTreeMap<String, Node>,
}

impl fmt::Debug for BundledFileSystem {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "BundledFileSystem({} entries)", self.nodes.len())
    }
}

fn read_only(path: &PathFragment) -> io::Error {
    io::Error::new(
        io::ErrorKind::PermissionDenied,
        format!("{} (Read-only file system)", path),
    )
}

fn not_found(path: &PathFragment) -> io::Error {
    io::Error::new(
        io::ErrorKind::NotFound,
        format!("{} (No such file or directory)", path),
    )
}

impl BundledFileSystem {
    /// Unpacks `files` (root-relative path, contents) under `root`.
    // Java: com/google/devtools/build/lib/analysis/ConfiguredRuleClassProvider.java:575 ConfiguredRuleClassProvider.Builder.unpackBuiltinsBzlZipResource
    fn new(root: &str, files: &[(&'static str, &'static str)]) -> BundledFileSystem {
        let mut nodes: BTreeMap<String, Node> = BTreeMap::new();
        nodes.insert("/".to_owned(), Node::Directory(Vec::new()));
        let add_dir = |nodes: &mut BTreeMap<String, Node>, dir: &PathFragment| {
            let mut cur = dir.clone();
            let mut chain = Vec::new();
            while !nodes.contains_key(cur.get_path_string()) {
                chain.push(cur.clone());
                match cur.get_parent_directory() {
                    Some(p) => cur = p,
                    None => break,
                }
            }
            for d in chain.into_iter().rev() {
                let parent = d.get_parent_directory().expect("not the root");
                if let Some(Node::Directory(children)) = nodes.get_mut(parent.get_path_string()) {
                    children.push(d.get_base_name().to_owned());
                }
                nodes.insert(d.get_path_string().to_owned(), Node::Directory(Vec::new()));
            }
        };
        let root = PathFragment::create(root);
        add_dir(&mut nodes, &root);
        for (rel, contents) in files {
            let path = root.get_relative_str(rel);
            let parent = path.get_parent_directory().expect("has a parent");
            add_dir(&mut nodes, &parent);
            if let Some(Node::Directory(children)) = nodes.get_mut(parent.get_path_string()) {
                children.push(path.get_base_name().to_owned());
            }
            nodes.insert(path.get_path_string().to_owned(), Node::File(contents));
        }
        BundledFileSystem { nodes }
    }

    fn node(&self, path: &PathFragment) -> io::Result<&Node> {
        self.nodes
            .get(path.get_path_string())
            .ok_or_else(|| not_found(path))
    }
}

impl FileSystem for BundledFileSystem {
    fn get_digest_function(&self) -> DigestHashFunction {
        DigestHashFunction::Sha256
    }

    fn supports_modifications(&self, _path: &PathFragment) -> bool {
        false
    }

    fn supports_symbolic_links_natively(&self, _path: &PathFragment) -> bool {
        false
    }

    fn supports_hard_links_natively(&self, _path: &PathFragment) -> bool {
        false
    }

    fn may_be_case_or_normalization_insensitive(&self) -> bool {
        false
    }

    fn create_directory(&self, path: &PathFragment) -> io::Result<bool> {
        Err(read_only(path))
    }

    fn create_directory_and_parents(&self, path: &PathFragment) -> io::Result<()> {
        Err(read_only(path))
    }

    fn get_file_size(&self, path: &PathFragment, _follow_symlinks: bool) -> io::Result<i64> {
        match self.node(path)? {
            Node::File(c) => Ok(c.len() as i64),
            Node::Directory(_) => Ok(0),
        }
    }

    fn delete(&self, path: &PathFragment) -> io::Result<bool> {
        Err(read_only(path))
    }

    fn get_last_modified_time(
        &self,
        path: &PathFragment,
        _follow_symlinks: bool,
    ) -> io::Result<i64> {
        self.node(path).map(|_| 0)
    }

    fn set_last_modified_time(&self, path: &PathFragment, _new_time: i64) -> io::Result<()> {
        Err(read_only(path))
    }

    // Java: com/google/devtools/build/lib/analysis/ConfiguredRuleClassProvider.java:126 ConfiguredRuleClassProvider.BundledFileSystem.getFastDigest
    fn get_fast_digest(&self, path: &PathFragment) -> io::Result<Option<Vec<u8>>> {
        self.get_digest(path).map(Some)
    }

    // Java: com/google/devtools/build/lib/analysis/ConfiguredRuleClassProvider.java:131 ConfiguredRuleClassProvider.BundledFileSystem.getDigest
    fn get_digest(&self, path: &PathFragment) -> io::Result<Vec<u8>> {
        // StringUnsafe.getInternalStringBytes: the path's internal (UTF-8) bytes.
        self.get_digest_function()
            .hash_reader(&mut path.get_path_string().as_bytes())
    }

    fn stat(&self, path: &PathFragment, _follow_symlinks: bool) -> io::Result<FileStatus> {
        // PORT: Java's InMemoryFileSystem records creation times and node ids; bundled files have
        // fast digests, so FileStateValue never uses them.
        match self.node(path)? {
            Node::File(c) => Ok(FileStatus::new(S_IFREG | 0o555, 0, 0, c.len() as i64, 0)),
            Node::Directory(_) => Ok(FileStatus::new(S_IFDIR | 0o555, 0, 0, 0, 0)),
        }
    }

    fn create_symbolic_link(
        &self,
        link_path: &PathFragment,
        _target_fragment: &PathFragment,
    ) -> io::Result<()> {
        Err(read_only(link_path))
    }

    fn read_symbolic_link(&self, path: &PathFragment) -> io::Result<PathFragment> {
        self.node(path)?;
        Err(blaze_vfs::NotASymlinkException::new_error(path))
    }

    fn exists(&self, path: &PathFragment, _follow_symlinks: bool) -> bool {
        self.nodes.contains_key(path.get_path_string())
    }

    fn get_directory_entries(&self, path: &PathFragment) -> io::Result<Vec<String>> {
        match self.node(path)? {
            Node::Directory(children) => Ok(children.clone()),
            Node::File(_) => Err(io::Error::other(format!("{} (Not a directory)", path))),
        }
    }

    fn is_readable(&self, path: &PathFragment) -> io::Result<bool> {
        self.node(path).map(|_| true)
    }

    fn set_readable(&self, path: &PathFragment, _readable: bool) -> io::Result<()> {
        Err(read_only(path))
    }

    fn is_writable(&self, path: &PathFragment) -> io::Result<bool> {
        self.node(path).map(|_| false)
    }

    fn set_writable(&self, path: &PathFragment, _writable: bool) -> io::Result<()> {
        Err(read_only(path))
    }

    fn is_executable(&self, path: &PathFragment) -> io::Result<bool> {
        self.node(path).map(|_| true)
    }

    fn set_executable(&self, path: &PathFragment, _executable: bool) -> io::Result<()> {
        Err(read_only(path))
    }

    fn get_input_stream(&self, path: &PathFragment) -> io::Result<Box<dyn Read + Send>> {
        match self.node(path)? {
            Node::File(c) => Ok(Box::new(io::Cursor::new(c.as_bytes()))),
            Node::Directory(_) => Err(io::Error::other(format!("{} (Is a directory)", path))),
        }
    }

    fn get_output_stream(
        &self,
        path: &PathFragment,
        _append: bool,
        _internal: bool,
    ) -> io::Result<Box<dyn Write + Send>> {
        Err(read_only(path))
    }

    fn rename_to(&self, source_path: &PathFragment, _target_path: &PathFragment) -> io::Result<()> {
        Err(read_only(source_path))
    }

    fn create_fs_dependent_hard_link(
        &self,
        link_path: &PathFragment,
        _original_path: &PathFragment,
    ) -> io::Result<()> {
        Err(read_only(link_path))
    }
}

static BUNDLED_FS: OnceLock<FileSystemRef> = OnceLock::new();

/// The process-wide bundled file system holding [`BUILTINS_BZL_FILES`] under
/// [`BUILTINS_PATH`].
pub fn bundled_file_system() -> &'static FileSystemRef {
    BUNDLED_FS.get_or_init(|| Arc::new(BundledFileSystem::new(BUILTINS_PATH, BUILTINS_BZL_FILES)))
}

/// Java `rootedPath.getRoot().getFileSystem() instanceof BundledFileSystem`.
pub fn is_bundled_file_system(fs: &FileSystemRef) -> bool {
    BUNDLED_FS.get().is_some_and(|b| Arc::ptr_eq(b, fs))
}

/// Java `ConfiguredRuleClassProvider.getBundledBuiltinsRoot()`.
// Java: com/google/devtools/build/lib/analysis/ConfiguredRuleClassProvider.java:751 ConfiguredRuleClassProvider.getBundledBuiltinsRoot
pub fn bundled_builtins_root() -> Root {
    Root::from_path(Path::create_from_str(BUILTINS_PATH, bundled_file_system()))
}
