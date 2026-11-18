//! `SkyframeMemoryDumper.DisplayMode` from `com/google/devtools/build/lib/buildtool/SkyframeMemoryDumper.java`.

use super::prelude::*;

// Java: com/google/devtools/build/lib/buildtool/SkyframeMemoryDumper.java:55 SkyframeMemoryDumper.DisplayMode
option_enum! {
    /// How to display Skyframe memory use.
    // SUMMARY: just a summary line; COUNT: object count by class; BYTES: bytes by class.
    pub enum DisplayMode { SUMMARY, COUNT, BYTES }
}
