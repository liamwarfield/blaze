//! Port of `lib/actions/ActionKeyContext.java`.

use std::hash::Hash;

use blaze_collect::NestedSet;
use blaze_collect::NestedSetFingerprintCache;
use blaze_collect::command_line_item::{CommandLineItem, ExceptionlessMapFn, MapFn, MapFnError};
use blaze_util::Fingerprint;

/// Contains state that aids in action key computation via `AbstractAction#computeKey`.
// Java: com/google/devtools/build/lib/actions/ActionKeyContext.java:22 ActionKeyContext
pub struct ActionKeyContext {
    nested_set_fingerprint_cache: NestedSetFingerprintCache,
}

impl Default for ActionKeyContext {
    fn default() -> Self {
        ActionKeyContext::new()
    }
}

impl ActionKeyContext {
    pub fn new() -> ActionKeyContext {
        ActionKeyContext {
            nested_set_fingerprint_cache: NestedSetFingerprintCache::new(),
        }
    }

    // Java: com/google/devtools/build/lib/actions/ActionKeyContext.java:27 ActionKeyContext.addNestedSetToFingerprint
    pub fn add_nested_set_to_fingerprint<T>(
        &self,
        fingerprint: &mut Fingerprint,
        nested_set: &NestedSet<T>,
    ) -> Result<(), MapFnError>
    where
        T: CommandLineItem + Clone + Eq + Hash + Send + Sync + 'static,
    {
        self.nested_set_fingerprint_cache
            .add_nested_set_to_fingerprint(fingerprint, nested_set)
    }

    // Java: com/google/devtools/build/lib/actions/ActionKeyContext.java:32 ActionKeyContext.addNestedSetToFingerprint
    pub fn add_nested_set_to_fingerprint_with_map_fn<T>(
        &self,
        map_fn: &'static dyn MapFn<T>,
        fingerprint: &mut Fingerprint,
        nested_set: &NestedSet<T>,
    ) -> Result<(), MapFnError>
    where
        T: Clone + Eq + Hash + Send + Sync + 'static,
    {
        self.nested_set_fingerprint_cache
            .add_nested_set_to_fingerprint_with_map_fn(map_fn, fingerprint, nested_set)
    }

    // Java: com/google/devtools/build/lib/actions/ActionKeyContext.java:38 ActionKeyContext.addNestedSetToFingerprint
    pub fn add_nested_set_to_fingerprint_exceptionless<T>(
        &self,
        map_fn: &'static dyn ExceptionlessMapFn<T>,
        fingerprint: &mut Fingerprint,
        nested_set: &NestedSet<T>,
    ) where
        T: Clone + Eq + Hash + Send + Sync + 'static,
    {
        self.nested_set_fingerprint_cache
            .add_nested_set_to_fingerprint_exceptionless(map_fn, fingerprint, nested_set)
    }

    // Java: com/google/devtools/build/lib/actions/ActionKeyContext.java:45 ActionKeyContext.describeNestedSetFingerprint
    pub fn describe_nested_set_fingerprint<T>(
        map_fn: &dyn ExceptionlessMapFn<T>,
        nested_set: &NestedSet<T>,
    ) -> String
    where
        T: Clone + Eq + Hash + 'static,
    {
        NestedSetFingerprintCache::described_nested_set_fingerprint(map_fn, nested_set)
    }

    // Java: com/google/devtools/build/lib/actions/ActionKeyContext.java:50 ActionKeyContext.clear
    pub fn clear(&self) {
        self.nested_set_fingerprint_cache.clear();
    }
}
