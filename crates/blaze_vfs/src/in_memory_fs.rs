//! The test file system of `lib/vfs/inmemoryfs` (`InMemoryFileSystem` and its node classes).
//!
//! Not ported: Blaze's tests run on the Unix file system in a temporary directory, and a test
//! that needs an injected failure wraps the file system or the glob's `FilesystemOps`. A Bazel
//! server never creates an in-memory file system.

// PORT: skipped, a test file system (tests use the Unix file system in a temporary directory): FileInfo.isDirectory, FileInfo.isSymbolicLink, FileInfo.isFile, FileInfo.isSpecialFile
// PORT: skipped, a test file system (tests use the Unix file system in a temporary directory): FileInfo.createReadWriteByteChannel, FileInfo.getxattr
// PORT: skipped, a test file system (tests use the Unix file system in a temporary directory): InMemoryContentInfo.isDirectory, InMemoryContentInfo.isSymbolicLink, InMemoryContentInfo.isFile, InMemoryContentInfo.isSpecialFile
// PORT: skipped, a test file system (tests use the Unix file system in a temporary directory): InMemoryContentInfo.getLastModifiedTime, InMemoryContentInfo.getLastChangeTime, InMemoryContentInfo.getNodeId, InMemoryContentInfo.getPermissions
// PORT: skipped, a test file system (tests use the Unix file system in a temporary directory): InMemoryContentInfo.inodeOrThrow, InMemoryContentInfo.isReadable, InMemoryContentInfo.setReadable, InMemoryContentInfo.isWritable
// PORT: skipped, a test file system (tests use the Unix file system in a temporary directory): InMemoryContentInfo.setWritable, InMemoryContentInfo.isExecutable, InMemoryContentInfo.setExecutable, InMemoryContentInfo.checkPermissions
// PORT: skipped, a test file system (tests use the Unix file system in a temporary directory): InMemoryContentInfo.updatePermissions, InMemoryContentInfo.asDirectory
// PORT: skipped, a test file system (tests use the Unix file system in a temporary directory): InMemoryDirectoryInfo.getChild, InMemoryDirectoryInfo.getAllChildren, InMemoryDirectoryInfo.isDirectory, InMemoryDirectoryInfo.isSymbolicLink
// PORT: skipped, a test file system (tests use the Unix file system in a temporary directory): InMemoryDirectoryInfo.isFile, InMemoryDirectoryInfo.isSpecialFile, InMemoryDirectoryInfo.getSize, InMemoryDirectoryInfo.asDirectory
// PORT: skipped, a test file system (tests use the Unix file system in a temporary directory): InMemoryFileInfo.getSize, InMemoryFileInfo.getxattr, InMemoryFileInfo.getFastDigest, InMemoryFileInfo.getInputStream
// PORT: skipped, a test file system (tests use the Unix file system in a temporary directory): InMemoryFileInfo.getOutputStream, InMemoryFileInfo.createReadWriteByteChannel, InMemoryFileInfo.ensureOpen, InMemoryFileInfo.ensureReadable
// PORT: skipped, a test file system (tests use the Unix file system in a temporary directory): InMemoryFileInfo.ensureWritable, InMemoryFileInfo.checkSize, InMemoryFileInfo.maybeGrow, InMemoryFileInfo.isOpen
// PORT: skipped, a test file system (tests use the Unix file system in a temporary directory): InMemoryFileInfo.close, InMemoryFileInfo.read, InMemoryFileInfo.write, InMemoryFileInfo.position
// PORT: skipped, a test file system (tests use the Unix file system in a temporary directory): InMemoryFileInfo.size, InMemoryFileInfo.truncate
// PORT: skipped, a test file system (tests use the Unix file system in a temporary directory): InMemoryFileSystem.inodeOrThrow, InMemoryFileSystem.getFileSystemType, InMemoryFileSystem.newFile, InMemoryFileSystem.createDirectory
// PORT: skipped, a test file system (tests use the Unix file system in a temporary directory): InMemoryFileSystem.createReadWriteByteChannel, InMemoryFileSystem.getxattr, InMemoryFileSystem.getOutputStream, InMemoryFileSystem.baseNameOrWindowsDrive
// PORT: skipped, a test file system (tests use the Unix file system in a temporary directory): InMemoryLinkInfo.isDirectory, InMemoryLinkInfo.isSymbolicLink, InMemoryLinkInfo.isFile, InMemoryLinkInfo.isSpecialFile
// PORT: skipped, a test file system (tests use the Unix file system in a temporary directory): InMemoryLinkInfo.getLinkContent, InMemoryLinkInfo.getNormalizedLinkContent, InMemoryLinkInfo.toString

// Other classes of `lib/vfs` that a Bazel server does not use.
// PORT: skipped, unused at the pin (no other class refers to it): ReadonlyFileSystem.modificationException, ReadonlyFileSystem.getOutputStream, ReadonlyFileSystem.createReadWriteByteChannel
// PORT: skipped, unused at the pin (no other class refers to it): ReadonlyFileSystem.setReadable, ReadonlyFileSystem.setWritable, ReadonlyFileSystem.setExecutable
// PORT: skipped, unused at the pin (no other class refers to it): ReadonlyFileSystem.supportsModifications, ReadonlyFileSystem.supportsSymbolicLinksNatively
// PORT: skipped, unused at the pin (no other class refers to it): ReadonlyFileSystem.supportsHardLinksNatively, ReadonlyFileSystem.createDirectory, ReadonlyFileSystem.createDirectoryAndParents
// PORT: skipped, unused at the pin (no other class refers to it): ReadonlyFileSystem.createSymbolicLink, ReadonlyFileSystem.createFSDependentHardLink, ReadonlyFileSystem.renameTo
// PORT: skipped, unused at the pin (no other class refers to it): ReadonlyFileSystem.delete, ReadonlyFileSystem.setLastModifiedTime
// PORT: skipped, unused at the pin (no other class refers to it): SearchPath.parse, SearchPath.which
// PORT: skipped, used only by PathTransformingDelegateFileSystem, which nothing uses at the pin: DelegateFileSystem.toDelegatePath, DelegateFileSystem.fromDelegatePath
// PORT: skipped, serialization for remote analysis caching (not ported, ruling 2026-11-19 (1)): FileAccessException.getEncodedClass, FileAccessException.serialize, FileAccessException.deserialize
