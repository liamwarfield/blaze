//! Port of the Linux parts of Bazel's C++ client (`src/main/cpp`).
//!
//! Entry point: [`main`] (`blaze::Main` of `src/main/cpp/blaze.cc`), which computes everything
//! Bazel's client computes (startup options, workspace, output/install bases, locks, install
//! base extraction, the server command line) and then hands the command to an
//! [`server::InProcessServer`] instead of a server JVM.

// field_reassign_with_default: proto requests are built field by field as the C++ does.
#![allow(
    clippy::too_many_arguments,
    clippy::new_without_default,
    clippy::field_reassign_with_default
)]

pub mod archive_utils;
pub mod binary_identity;
pub mod blaze;
pub mod command_server;
pub mod cpp_unordered;
pub mod option_processor;
pub mod platform;
pub mod server;
pub mod signal_handler;
pub mod startup_options;
pub mod util;
pub mod workspace_layout;

pub use archive_utils::EmbeddedArchive;
pub use blaze::{argv_from_os, main};
