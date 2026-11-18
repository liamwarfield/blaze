//! Port of `buildeventstream/TestFileNameConstants.java`.
//!
//! PORT: the Java class lives in `lib/buildeventstream`; it is kept next to its users here.
//!
//! The file names associated with a test are indexed in the build-event protocol by a string in
//! order to allow extensions of bazel to add their own files. This module provides constants for
//! the names of the standard files associated with a test.
// Java: com/google/devtools/build/lib/buildeventstream/TestFileNameConstants.java:24 TestFileNameConstants

pub const SPLIT_LOGS: &str = "test.splitlogs";
pub const TEST_INFRASTRUCTURE_FAILURE: &str = "test.infrastructure_failure";
pub const TEST_LOG: &str = "test.log";
pub const TEST_STDERR: &str = "test.stderr";
pub const TEST_WARNINGS: &str = "test.warnings";
pub const TEST_XML: &str = "test.xml";
pub const UNUSED_RUNFILES_LOG: &str = "test.unused_runfiles_log";

// Only present for the coverage command.
pub const TEST_COVERAGE: &str = "test.lcov";

// Present for both --zip_undeclared_outputs and --nozip_undeclared_outputs.
pub const UNDECLARED_OUTPUTS_ANNOTATIONS: &str = "test.outputs_manifest__ANNOTATIONS";
pub const UNDECLARED_OUTPUTS_ANNOTATIONS_PB: &str = "test.outputs_manifest__ANNOTATIONS.pb";
pub const UNDECLARED_OUTPUTS_MANIFEST: &str = "test.outputs_manifest__MANIFEST";

// Only present for --zip_undeclared_outputs.
pub const UNDECLARED_OUTPUTS_ZIP: &str = "test.outputs__outputs.zip";

// Only present for --nozip_undeclared_outputs.
// This is a prefix; each file in the undeclared outputs directory is reported individually, e.g.
// test.outputs/path/to/file.txt.
pub const UNDECLARED_OUTPUTS_DIR: &str = "test.outputs";
