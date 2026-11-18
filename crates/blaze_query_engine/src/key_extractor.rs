//! Port of `query2/engine/KeyExtractor.java`.

use std::sync::Arc;

/// Helper for extracting a key of type `K` from an element of type `T`.
///
/// Depending on the choice of `K`, this enables potential memory optimizations.
// Java: com/google/devtools/build/lib/query2/engine/KeyExtractor.java:24 KeyExtractor
pub trait KeyExtractor<T, K>: Send + Sync {
    /// Extracts an unique key that can be used to dedupe the given `element`.
    // Java: com/google/devtools/build/lib/query2/engine/KeyExtractor.java:26 KeyExtractor.extractKey
    fn extract_key(&self, element: &T) -> K;
}

impl<T, K, F> KeyExtractor<T, K> for F
where
    F: Fn(&T) -> K + Send + Sync,
{
    fn extract_key(&self, element: &T) -> K {
        self(element)
    }
}

/// Java `KeyExtractor.compose(inner, outer)`.
// Java: com/google/devtools/build/lib/query2/engine/KeyExtractor.java:28 KeyExtractor.compose
pub fn compose<T1, T2, K>(
    inner: Arc<dyn KeyExtractor<T1, T2>>,
    outer: Arc<dyn KeyExtractor<T2, K>>,
) -> Arc<dyn KeyExtractor<T1, K>>
where
    T1: 'static,
    T2: 'static,
    K: 'static,
{
    Arc::new(move |t1: &T1| outer.extract_key(&inner.extract_key(t1)))
}
