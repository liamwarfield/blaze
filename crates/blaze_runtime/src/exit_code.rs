//! `lib/util/ExitCode` and `lib/util/DetailedExitCode`: the shared port in `blaze_util`
//! (`blaze_util::exit_code`), re-exported so that `crate::exit_code` paths keep working.
//!
//! PORT: this crate had its own copy (with the same table generated from
//! `failure_details.proto`); there is one set of types now, so that `blaze_server`'s
//! `BlazeCommandResult` and this crate's are the same. The tests of the copy
//! (`numeric_exit_code_from_proto_metadata`, `choose_more_important_with_first_if_tie`) are in
//! `blaze_util`.

pub use blaze_util::exit_code::{DetailedExitCode, ExitCode};
