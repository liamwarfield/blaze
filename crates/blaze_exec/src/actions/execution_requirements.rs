//! Port of the string constants of `lib/actions/ExecutionRequirements.java`.

// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:31 ExecutionRequirements
pub const TIMEOUT: &str = "timeout";
pub const REQUIRES_DARWIN: &str = "requires-darwin";
pub const DISABLE_LOCAL_PREFETCH: &str = "disable-local-prefetch";
pub const SUPPORTS_WORKERS: &str = "supports-workers";
pub const SUPPORTS_MULTIPLEX_WORKERS: &str = "supports-multiplex-workers";
pub const REQUIRES_WORKER_PROTOCOL: &str = "requires-worker-protocol";
pub const SUPPORTS_WORKER_CANCELLATION: &str = "supports-worker-cancellation";
pub const SUPPORTS_MULTIPLEX_SANDBOXING: &str = "supports-multiplex-sandboxing";
pub const WORKER_KEY_MNEMONIC: &str = "worker-key-mnemonic";
pub const LOCAL: &str = "local";
pub const NO_CACHE: &str = "no-cache";
pub const NO_REMOTE_CACHE: &str = "no-remote-cache";
pub const NO_REMOTE_CACHE_UPLOAD: &str = "no-remote-cache-upload";
pub const NO_REMOTE_EXEC: &str = "no-remote-exec";
pub const NO_TESTLOASD: &str = "no-testloasd";
pub const NO_REMOTE: &str = "no-remote";
pub const NO_LOCAL: &str = "no-local";
pub const LEGACY_NOSANDBOX: &str = "nosandbox";
pub const NO_SANDBOX: &str = "no-sandbox";
pub const REQUIREMENTS_SET: &str = "supports-xcode-requirements-set";
pub const REQUIRES_NETWORK: &str = "requires-network";
pub const BLOCK_NETWORK: &str = "block-network";
pub const REQUIRES_FAKEROOT: &str = "requires-fakeroot";
pub const DO_NOT_REPORT: &str = "internal-do-not-report";
pub const REMOTE_EXECUTION_INLINE_OUTPUTS: &str = "internal-inline-outputs";
pub const MEMORY_ESTIMATE: &str = "internal-memory-estimate";
pub const GRACEFUL_TERMINATION: &str = "supports-graceful-termination";
pub const REQUIRES_XCODE: &str = "requires-xcode";
pub const REQUIRES_XCODE_LABEL: &str = "requires-xcode-label";
pub const SUPPORTS_PATH_MAPPING: &str = "supports-path-mapping";
pub const DIFFERENTIATE_WORKSPACE_CACHE: &str = "internal-differentiate-workspace-cache";
