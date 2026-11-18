//! The option-relevant part of `buildeventstream/BuildEventContext.java`.

use blaze_options::option_enum;

// Java: com/google/devtools/build/lib/buildeventstream/BuildEventContext.java:41 BuildEventContext.OutputGroupFileMode
option_enum! {
    /// Mode in which files are reported in an output group: recursive filesets, inline, or both.
    pub enum OutputGroupFileMode { NAMED_SET_OF_FILES_ONLY, INLINE_ONLY, BOTH }
}
