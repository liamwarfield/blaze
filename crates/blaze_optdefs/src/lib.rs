//! Every Bazel `OptionsBase` subclass, transcribed with the `blaze_options` declaration macro.
//! One Rust module per Java class, grouped by owner. See `crates/blaze_options/TRANSCRIBE.md`.

/// `FragmentOptions` subclasses (they feed configuration checksums and output-path hashes).
pub mod fragment;
/// Command, runtime, startup, execution, query and module options.
pub mod command;
/// Repository, remote, auth, sandbox, worker and remaining options.
pub mod misc;
