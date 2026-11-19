//! Ports of `lib/util/io` pieces the execution layer needs.

pub mod file_out_err;
pub mod file_watcher;

pub use file_out_err::FileOutErr;
