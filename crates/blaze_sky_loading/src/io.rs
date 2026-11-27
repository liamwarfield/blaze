//! `java.io.IOException` and the `lib/io` exceptions that the file functions throw through
//! Skyframe.
//!
//! Java catches these by class (`getValueOrThrow(key, IOException.class)` and then `catch
//! (FileSymlinkException e)` before `catch (IOException e)`). Skyframe here matches a dependency
//! error by its exact Rust type, so every IOException subclass is one type, [`IOException`], and
//! [`IOExceptionKind`] says which Java class it is. Callers `match` on the kind in the order of
//! Java's catch clauses.
//!
//! Java: java/io/IOException, com/google/devtools/build/lib/io/{FileSymlinkException,
//! FileSymlinkCycleException, FileSymlinkInfiniteExpansionException,
//! InconsistentFilesystemException}.java.

use std::fmt;

use blaze_vfs::{DetailedIOException, RootedPath};

/// Which Java `IOException` class an [`IOException`] stands for.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum IOExceptionKind {
    /// `java.io.IOException` itself (and OS errors).
    Plain,
    /// `InconsistentFilesystemException`.
    InconsistentFilesystem,
    // Java: com/google/devtools/build/lib/io/FileSymlinkException.java:20 FileSymlinkException.<init>
    // Java: com/google/devtools/build/lib/io/FileSymlinkCycleException.java:42 FileSymlinkCycleException.getPathToCycle
    // Java: com/google/devtools/build/lib/io/FileSymlinkCycleException.java:50 FileSymlinkCycleException.getCycle
    /// `FileSymlinkCycleException` (a `FileSymlinkException`).
    FileSymlinkCycle {
        path_to_cycle: Vec<RootedPath>,
        cycle: Vec<RootedPath>,
    },
    // Java: com/google/devtools/build/lib/io/FileSymlinkInfiniteExpansionException.java:42 FileSymlinkInfiniteExpansionException.getPathToChain
    // Java: com/google/devtools/build/lib/io/FileSymlinkInfiniteExpansionException.java:51 FileSymlinkInfiniteExpansionException.getChain
    /// `FileSymlinkInfiniteExpansionException` (a `FileSymlinkException`).
    FileSymlinkInfiniteExpansion {
        path_to_chain: Vec<RootedPath>,
        chain: Vec<RootedPath>,
    },
    /// `DetailedIOException` (vfs), thrown by a file system with its own failure detail.
    Detailed(DetailedIOException),
}

/// A Java `IOException` (or subclass, see [`IOExceptionKind`]). `Display` is `getMessage()`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct IOException {
    kind: IOExceptionKind,
    message: String,
}

impl IOException {
    /// `new IOException(message)`.
    pub fn new(message: impl Into<String>) -> IOException {
        IOException {
            kind: IOExceptionKind::Plain,
            message: message.into(),
        }
    }

    /// An `IOException` thrown by the file system layer: a `DetailedIOException` keeps its class.
    pub fn from_io(e: &std::io::Error) -> IOException {
        match DetailedIOException::find(e) {
            Some(detailed) => IOException {
                kind: IOExceptionKind::Detailed(detailed.clone()),
                message: detailed.get_message().to_owned(),
            },
            None => IOException::new(e.to_string()),
        }
    }

    /// `e instanceof DetailedIOException ? (DetailedIOException) e : null`.
    pub fn as_detailed(&self) -> Option<&DetailedIOException> {
        match &self.kind {
            IOExceptionKind::Detailed(detailed) => Some(detailed),
            _ => None,
        }
    }

    /// Used to indicate a filesystem inconsistency, e.g. file 'a/b' exists but directory 'a'
    /// doesn't exist. This generally means the result of the build is undefined but we shouldn't
    /// crash hard.
    // Java: com/google/devtools/build/lib/io/InconsistentFilesystemException.java:24 InconsistentFilesystemException.<init>
    pub fn inconsistent_filesystem(inconsistency_message: impl fmt::Display) -> IOException {
        IOException {
            kind: IOExceptionKind::InconsistentFilesystem,
            message: format!(
                "Inconsistent filesystem operations. {} The results of the build are not \
                 guaranteed to be correct. You should probably run 'bazel clean' and investigate \
                 the filesystem inconsistency (likely due to filesystem updates concurrent with \
                 the build)",
                inconsistency_message
            ),
        }
    }

    /// Exception indicating that a cycle was found in the filesystem.
    // Java: com/google/devtools/build/lib/io/FileSymlinkCycleException.java:30 FileSymlinkCycleException.<init>
    pub fn file_symlink_cycle(path_to_cycle: Vec<RootedPath>, cycle: Vec<RootedPath>) -> Self {
        // The cycle itself has already been reported by FileSymlinkCycleUniquenessValue, but we
        // still want to have a readable #getMessage.
        IOException {
            kind: IOExceptionKind::FileSymlinkCycle {
                path_to_cycle,
                cycle,
            },
            message: "Symlink cycle".to_owned(),
        }
    }

    /// Exception indicating that a symlink has an unbounded expansion on resolution.
    // Java: com/google/devtools/build/lib/io/FileSymlinkInfiniteExpansionException.java:30 FileSymlinkInfiniteExpansionException.<init>
    pub fn file_symlink_infinite_expansion(
        path_to_chain: Vec<RootedPath>,
        chain: Vec<RootedPath>,
    ) -> Self {
        // The infinite expansion has already been reported by
        // FileSymlinkInfiniteExpansionUniquenessValue, but we still want to have a readable
        // #getMessage.
        IOException {
            kind: IOExceptionKind::FileSymlinkInfiniteExpansion {
                path_to_chain,
                chain,
            },
            message: "Infinite symlink expansion".to_owned(),
        }
    }

    pub fn kind(&self) -> &IOExceptionKind {
        &self.kind
    }

    /// Java `Throwable.getMessage()`.
    pub fn get_message(&self) -> &str {
        &self.message
    }

    /// `e instanceof InconsistentFilesystemException`.
    pub fn is_inconsistent_filesystem(&self) -> bool {
        self.kind == IOExceptionKind::InconsistentFilesystem
    }

    /// `e instanceof FileSymlinkException`.
    pub fn is_file_symlink_exception(&self) -> bool {
        matches!(
            self.kind,
            IOExceptionKind::FileSymlinkCycle { .. }
                | IOExceptionKind::FileSymlinkInfiniteExpansion { .. }
        )
    }

    /// `FileSymlinkException.getUserFriendlyMessage()`: a description of the problem that is
    /// suitable for printing to users. `None` for other kinds.
    // Java: com/google/devtools/build/lib/io/FileSymlinkCycleException.java:59 FileSymlinkCycleException.getUserFriendlyMessage
    // Java: com/google/devtools/build/lib/io/FileSymlinkInfiniteExpansionException.java:59 FileSymlinkInfiniteExpansionException.getUserFriendlyMessage
    pub fn get_user_friendly_message(&self) -> Option<String> {
        let (prefix, chain) = match &self.kind {
            IOExceptionKind::FileSymlinkCycle { cycle, .. } => ("Symlink cycle: ", cycle),
            IOExceptionKind::FileSymlinkInfiniteExpansion { chain, .. } => {
                ("Infinite symlink expansion: ", chain)
            }
            _ => return None,
        };
        let joined: Vec<String> = chain.iter().map(|p| p.as_path().to_string()).collect();
        Some(format!("{}{}", prefix, joined.join("- > ")))
    }
}

impl fmt::Display for IOException {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for IOException {}
