//! Port of the string constants of `lib/actions/ExecutionRequirements.java`.
//!
//! PORT: `ParseableRequirement` (CPU, RESOURCES) and the worker maps are not ported yet.

// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:127 ExecutionRequirements.TIMEOUT
pub const TIMEOUT: &str = "timeout";
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:130 ExecutionRequirements.REQUIRES_DARWIN
pub const REQUIRES_DARWIN: &str = "requires-darwin";
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:133 ExecutionRequirements.DISABLE_LOCAL_PREFETCH
pub const DISABLE_LOCAL_PREFETCH: &str = "disable-local-prefetch";
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:274 ExecutionRequirements.SUPPORTS_WORKERS
pub const SUPPORTS_WORKERS: &str = "supports-workers";
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:276 ExecutionRequirements.SUPPORTS_MULTIPLEX_WORKERS
pub const SUPPORTS_MULTIPLEX_WORKERS: &str = "supports-multiplex-workers";
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:279 ExecutionRequirements.REQUIRES_WORKER_PROTOCOL
pub const REQUIRES_WORKER_PROTOCOL: &str = "requires-worker-protocol";
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:281 ExecutionRequirements.SUPPORTS_WORKER_CANCELLATION
pub const SUPPORTS_WORKER_CANCELLATION: &str = "supports-worker-cancellation";
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:283 ExecutionRequirements.SUPPORTS_MULTIPLEX_SANDBOXING
pub const SUPPORTS_MULTIPLEX_SANDBOXING: &str = "supports-multiplex-sandboxing";
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:292 ExecutionRequirements.WORKER_KEY_MNEMONIC
pub const WORKER_KEY_MNEMONIC: &str = "worker-key-mnemonic";
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:305 ExecutionRequirements.LOCAL
pub const LOCAL: &str = "local";
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:315 ExecutionRequirements.NO_CACHE
pub const NO_CACHE: &str = "no-cache";
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:318 ExecutionRequirements.NO_REMOTE_CACHE
pub const NO_REMOTE_CACHE: &str = "no-remote-cache";
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:321 ExecutionRequirements.NO_REMOTE_CACHE_UPLOAD
pub const NO_REMOTE_CACHE_UPLOAD: &str = "no-remote-cache-upload";
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:324 ExecutionRequirements.NO_REMOTE_EXEC
pub const NO_REMOTE_EXEC: &str = "no-remote-exec";
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:327 ExecutionRequirements.NO_TESTLOASD
pub const NO_TESTLOASD: &str = "no-testloasd";
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:333 ExecutionRequirements.NO_REMOTE
pub const NO_REMOTE: &str = "no-remote";
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:336 ExecutionRequirements.NO_LOCAL
pub const NO_LOCAL: &str = "no-local";
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:339 ExecutionRequirements.LEGACY_NOSANDBOX
pub const LEGACY_NOSANDBOX: &str = "nosandbox";
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:342 ExecutionRequirements.NO_SANDBOX
pub const NO_SANDBOX: &str = "no-sandbox";
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:349 ExecutionRequirements.REQUIREMENTS_SET
pub const REQUIREMENTS_SET: &str = "supports-xcode-requirements-set";
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:355 ExecutionRequirements.REQUIRES_NETWORK
pub const REQUIRES_NETWORK: &str = "requires-network";
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:361 ExecutionRequirements.BLOCK_NETWORK
pub const BLOCK_NETWORK: &str = "block-network";
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:367 ExecutionRequirements.REQUIRES_FAKEROOT
pub const REQUIRES_FAKEROOT: &str = "requires-fakeroot";
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:370 ExecutionRequirements.DO_NOT_REPORT
pub const DO_NOT_REPORT: &str = "internal-do-not-report";
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:373 ExecutionRequirements.REMOTE_EXECUTION_INLINE_OUTPUTS
pub const REMOTE_EXECUTION_INLINE_OUTPUTS: &str = "internal-inline-outputs";
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:376 ExecutionRequirements.MEMORY_ESTIMATE
pub const MEMORY_ESTIMATE: &str = "internal-memory-estimate";
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:382 ExecutionRequirements.GRACEFUL_TERMINATION
pub const GRACEFUL_TERMINATION: &str = "supports-graceful-termination";
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:385 ExecutionRequirements.REQUIRES_XCODE
pub const REQUIRES_XCODE: &str = "requires-xcode";
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:393 ExecutionRequirements.REQUIRES_XCODE_LABEL
pub const REQUIRES_XCODE_LABEL: &str = "requires-xcode-label";
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:396 ExecutionRequirements.DIFFERENTIATE_WORKSPACE_CACHE
pub const DIFFERENTIATE_WORKSPACE_CACHE: &str = "internal-differentiate-workspace-cache";
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:403 ExecutionRequirements.SUPPORTS_PATH_MAPPING
pub const SUPPORTS_PATH_MAPPING: &str = "supports-path-mapping";
