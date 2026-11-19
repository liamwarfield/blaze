//! Port of `EventFilter`.

use crate::SkyKey;

/// Filters out events which should not be stored during evaluation in `ParallelEvaluator`.
// Java: com/google/devtools/build/skyframe/EventFilter.java:17 EventFilter
pub trait EventFilter: Send + Sync {
    /// Returns true if any events (`Reportable`) should be stored in skyframe nodes. Otherwise,
    /// optimizations may be made to avoid doing unnecessary work when evaluating node entries.
    // Java: com/google/devtools/build/skyframe/EventFilter.java:24 EventFilter.storeEvents
    fn store_events(&self) -> bool;

    /// Determines whether stored events (`Reportable`) should propagate from `dep_key` to
    /// `primary_key`.
    ///
    /// Only relevant if [`EventFilter::store_events`] returns `true`.
    // Java: com/google/devtools/build/skyframe/EventFilter.java:32 EventFilter.shouldPropagate
    fn should_propagate(&self, dep_key: &SkyKey, primary_key: &SkyKey) -> bool;
}

impl dyn EventFilter {
    // Java: com/google/devtools/build/skyframe/EventFilter.java:34 EventFilter.FULL_STORAGE
    pub const FULL_STORAGE: &'static (dyn EventFilter + 'static) = &FullStorage;

    // Java: com/google/devtools/build/skyframe/EventFilter.java:47 EventFilter.NO_STORAGE
    pub const NO_STORAGE: &'static (dyn EventFilter + 'static) = &NoStorage;
}

// Java: com/google/devtools/build/skyframe/EventFilter.java:35 EventFilter.FULL_STORAGE
struct FullStorage;

impl EventFilter for FullStorage {
    // Java: com/google/devtools/build/skyframe/EventFilter.java:37 EventFilter.FULL_STORAGE.storeEvents
    fn store_events(&self) -> bool {
        true
    }

    // Java: com/google/devtools/build/skyframe/EventFilter.java:42 EventFilter.FULL_STORAGE.shouldPropagate
    fn should_propagate(&self, _dep_key: &SkyKey, _primary_key: &SkyKey) -> bool {
        true
    }
}

// Java: com/google/devtools/build/skyframe/EventFilter.java:48 EventFilter.NO_STORAGE
struct NoStorage;

impl EventFilter for NoStorage {
    // Java: com/google/devtools/build/skyframe/EventFilter.java:50 EventFilter.NO_STORAGE.storeEvents
    fn store_events(&self) -> bool {
        false
    }

    // Java: com/google/devtools/build/skyframe/EventFilter.java:55 EventFilter.NO_STORAGE.shouldPropagate
    fn should_propagate(&self, _dep_key: &SkyKey, _primary_key: &SkyKey) -> bool {
        // Java: throw new UnsupportedOperationException() (no message).
        panic!("java.lang.UnsupportedOperationException")
    }
}

/// A static filter (`EventFilter::FULL_STORAGE`, `EventFilter::NO_STORAGE`) as an owned one, for
/// `EvaluatorBuilder::event_filter(Arc::new(<dyn EventFilter>::FULL_STORAGE))`.
impl EventFilter for &'static (dyn EventFilter + 'static) {
    fn store_events(&self) -> bool {
        (**self).store_events()
    }

    fn should_propagate(&self, dep_key: &SkyKey, primary_key: &SkyKey) -> bool {
        (**self).should_propagate(dep_key, primary_key)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluator::ERROR_TRANSIENCE_KEY;

    #[test]
    fn full_storage_stores_and_propagates() {
        let f = <dyn EventFilter>::FULL_STORAGE;
        assert!(f.store_events());
        assert!(f.should_propagate(&ERROR_TRANSIENCE_KEY, &ERROR_TRANSIENCE_KEY));
    }

    #[test]
    fn no_storage_stores_nothing() {
        assert!(!<dyn EventFilter>::NO_STORAGE.store_events());
    }

    #[test]
    #[should_panic(expected = "java.lang.UnsupportedOperationException")]
    fn no_storage_should_propagate_throws() {
        <dyn EventFilter>::NO_STORAGE
            .should_propagate(&ERROR_TRANSIENCE_KEY, &ERROR_TRANSIENCE_KEY);
    }
}
