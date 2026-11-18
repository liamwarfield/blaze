//! `com/google/devtools/build/lib/outputfilter/AutoOutputFilter.java`: the enum and its converter.
//!
//! PORT: the per-constant `getFilter` implementations need `OutputFilter` (not ported here).

use super::prelude::*;

// Java: com/google/devtools/build/lib/outputfilter/AutoOutputFilter.java:36 AutoOutputFilter
option_enum! {
    /// Implementation of various settings for the `--auto_output_filter` option.
    pub enum AutoOutputFilter { NONE, ALL, PACKAGES, SUBPACKAGES }
}

// Java: com/google/devtools/build/lib/outputfilter/AutoOutputFilter.java:134 AutoOutputFilter.Converter
enum_converter!(pub AutoOutputFilterConverter, AutoOutputFilter, "automatic output filter");
