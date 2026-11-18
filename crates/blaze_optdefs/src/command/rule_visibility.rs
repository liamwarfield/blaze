//! The two constants of `com/google/devtools/build/lib/packages/RuleVisibility.java` that `--default_visibility` can
//! produce.

use super::prelude::*;

// Java: com/google/devtools/build/lib/packages/RuleVisibility.java:35 RuleVisibility
option_enum! {
    /// `RuleVisibility.PUBLIC` / `RuleVisibility.PRIVATE`; `toString()` is the label.
    // PORT: RuleVisibility is an abstract class with more subclasses (package groups); only the
    // two constants reachable from the option are ported here.
    pub enum RuleVisibility { PUBLIC = "//visibility:public", PRIVATE = "//visibility:private" }
}
