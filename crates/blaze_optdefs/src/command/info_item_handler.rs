//! `InfoItemHandler.InfoItemOutputType` from `com/google/devtools/build/lib/runtime/commands/info/InfoItemHandler.java`.

use super::prelude::*;

// Java: com/google/devtools/build/lib/runtime/commands/info/InfoItemHandler.java:32 InfoItemHandler.InfoItemOutputType
option_enum! {
    /// Where info output goes: printed to the console, or packed into response extensions.
    pub enum InfoItemOutputType { STDOUT, RESPONSE_PROTO }
}
