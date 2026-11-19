//! Port of `analysis/InconsistentAspectOrderException.java`.

use blaze_cmdline::Label;
use blaze_starlark::Location;

use crate::aspect_collection::AspectCycleOnPathException;

/// Signals an inconsistency on an aspect path: an aspect occurs twice on the path and the second
/// occurrence sees a different set of aspects (see [`AspectCycleOnPathException`]).
// Java: com/google/devtools/build/lib/analysis/InconsistentAspectOrderException.java:25 InconsistentAspectOrderException
#[derive(Clone, Debug, thiserror::Error)]
#[error("{message}")]
pub struct InconsistentAspectOrderException {
    message: String,
    location: Location,
}

impl InconsistentAspectOrderException {
    // Java: com/google/devtools/build/lib/analysis/InconsistentAspectOrderException.java:28 InconsistentAspectOrderException.<init>
    pub fn new(
        target_label: &Label,
        location: Location,
        e: &AspectCycleOnPathException,
    ) -> InconsistentAspectOrderException {
        InconsistentAspectOrderException {
            message: format!("{} (when propagating to {})", e.get_message(), target_label),
            location,
        }
    }

    pub fn get_message(&self) -> &str {
        &self.message
    }

    // Java: com/google/devtools/build/lib/analysis/InconsistentAspectOrderException.java:34 InconsistentAspectOrderException.getLocation
    pub fn get_location(&self) -> &Location {
        &self.location
    }
}
