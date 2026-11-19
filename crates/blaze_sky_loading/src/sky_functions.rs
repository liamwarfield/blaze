//! Names of the SkyFunctions in this crate.
//!
//! Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java, plus
//! `vfs/FileStateKey.FILE_STATE` and the `lib/io` uniqueness function names. Hermeticity
//! (`createHermetic` / `createNonHermetic` / `createSemiHermetic`) only matters for incremental
//! builds and is not represented. PORT: skipped, incremental.

use blaze_skyframe::SkyFunctionName;

// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:22 SkyFunctions.PRECOMPUTED
pub const PRECOMPUTED: SkyFunctionName = SkyFunctionName("PRECOMPUTED");
// Java: com/google/devtools/build/lib/vfs/FileStateKey.java:26 FileStateKey.FILE_STATE
pub const FILE_STATE: SkyFunctionName = SkyFunctionName("FILE_STATE");
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:32 SkyFunctions.DIRECTORY_LISTING_STATE
pub const DIRECTORY_LISTING_STATE: SkyFunctionName = SkyFunctionName("DIRECTORY_LISTING_STATE");
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:34 SkyFunctions.DIRECTORY_LISTING
pub const DIRECTORY_LISTING: SkyFunctionName = SkyFunctionName("DIRECTORY_LISTING");
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:40 SkyFunctions.PACKAGE_LOOKUP
pub const PACKAGE_LOOKUP: SkyFunctionName = SkyFunctionName("PACKAGE_LOOKUP");
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:42 SkyFunctions.CONTAINING_PACKAGE_LOOKUP
pub const CONTAINING_PACKAGE_LOOKUP: SkyFunctionName = SkyFunctionName("CONTAINING_PACKAGE_LOOKUP");
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:47 SkyFunctions.BZL_COMPILE
pub const BZL_COMPILE: SkyFunctionName = SkyFunctionName("BZL_COMPILE");
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:48 SkyFunctions.STARLARK_BUILTINS
pub const STARLARK_BUILTINS: SkyFunctionName = SkyFunctionName("STARLARK_BUILTINS");
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:50 SkyFunctions.BZL_LOAD
pub const BZL_LOAD: SkyFunctionName = SkyFunctionName("BZL_LOAD");
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:53 SkyFunctions.FILE
pub const FILE: SkyFunctionName = SkyFunctionName("FILE");
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:54 SkyFunctions.GLOB
pub const GLOB: SkyFunctionName = SkyFunctionName("GLOB");
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:55 SkyFunctions.GLOBS
pub const GLOBS: SkyFunctionName = SkyFunctionName("GLOBS");
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:56 SkyFunctions.PACKAGE
pub const PACKAGE: SkyFunctionName = SkyFunctionName("PACKAGE");
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:68 SkyFunctions.TARGET_PATTERN
pub const TARGET_PATTERN: SkyFunctionName = SkyFunctionName("TARGET_PATTERN");
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:84 SkyFunctions.IGNORED_SUBDIRECTORIES
pub const IGNORED_SUBDIRECTORIES: SkyFunctionName = SkyFunctionName("IGNORED_SUBDIRECTORIES");
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:133 SkyFunctions.REPOSITORY_DIRECTORY
pub const REPOSITORY_DIRECTORY: SkyFunctionName = SkyFunctionName("REPOSITORY_DIRECTORY");
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:137 SkyFunctions.LOCAL_REPOSITORY_LOOKUP
pub const LOCAL_REPOSITORY_LOOKUP: SkyFunctionName = SkyFunctionName("LOCAL_REPOSITORY_LOOKUP");
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:147 SkyFunctions.REPOSITORY_MAPPING
pub const REPOSITORY_MAPPING: SkyFunctionName = SkyFunctionName("REPOSITORY_MAPPING");
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:151 SkyFunctions.REPO_PACKAGE_ARGS
pub const REPO_PACKAGE_ARGS: SkyFunctionName = SkyFunctionName("REPO_PACKAGE_ARGS");
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:153 SkyFunctions.REPO_FILE
pub const REPO_FILE: SkyFunctionName = SkyFunctionName("REPO_FILE");
// Java: com/google/devtools/build/lib/io/FileSymlinkCycleUniquenessFunction.java:31 FileSymlinkCycleUniquenessFunction.NAME
pub const FILE_SYMLINK_CYCLE_UNIQUENESS: SkyFunctionName =
    SkyFunctionName("FILE_SYMLINK_CYCLE_UNIQUENESS");
// Java: com/google/devtools/build/lib/io/FileSymlinkInfiniteExpansionUniquenessFunction.java:32 FileSymlinkInfiniteExpansionUniquenessFunction.NAME
pub const FILE_SYMLINK_INFINITE_EXPANSION_UNIQUENESS: SkyFunctionName =
    SkyFunctionName("FILE_SYMLINK_INFINITE_EXPANSION_UNIQUENESS");
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:57 SkyFunctions.PACKAGE_DECLARATIONS
pub const PACKAGE_DECLARATIONS: SkyFunctionName = SkyFunctionName("PACKAGE_DECLARATIONS");
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:62 SkyFunctions.EVAL_MACRO
pub const EVAL_MACRO: SkyFunctionName = SkyFunctionName("EVAL_MACRO");
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:63 SkyFunctions.MACRO_INSTANCE
pub const MACRO_INSTANCE: SkyFunctionName = SkyFunctionName("MACRO_INSTANCE");
// Java: com/google/devtools/build/lib/skyframe/SkyFunctions.java:65 SkyFunctions.NON_FINALIZER_PACKAGE_PIECES
pub const NON_FINALIZER_PACKAGE_PIECES: SkyFunctionName =
    SkyFunctionName("NON_FINALIZER_PACKAGE_PIECES");
