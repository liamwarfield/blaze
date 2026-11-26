//! Values of the file functions, from Java `lib/actions`.
//!
//! PORT: move to blaze_actions (Java `com.google.devtools.build.lib.actions`) when that crate
//! exists; FILE_STATE and FILE need them now.

pub mod file_state_value;
pub mod file_value;

pub use file_state_value::{
    ContentsComparisonResult, FileContentsProxy, FileStateType, FileStateValue,
};
pub use file_value::FileValue;
