//! Port of `query2/engine/Uniquifier.java`.

use crate::query_exception::QueryException;

/// A helper for deduping values.
// Java: com/google/devtools/build/lib/query2/engine/Uniquifier.java:22 Uniquifier
pub trait Uniquifier<T> {
    /// Returns whether `new_element` has been seen before by `unique` or `unique_all`.
    ///
    /// Please note the difference between this method and `unique`!
    ///
    /// This method is inherently racy wrt `unique` and `unique_all`. Only use it if you know what
    /// you are doing.
    // Java: com/google/devtools/build/lib/query2/engine/Uniquifier.java:32 Uniquifier.uniquePure
    fn unique_pure(&self, new_element: &T) -> bool;

    /// Returns whether `new_element` has been seen before by `unique` or `unique_all`.
    // Java: com/google/devtools/build/lib/query2/engine/Uniquifier.java:38 Uniquifier.unique
    fn unique(&self, new_element: &T) -> Result<bool, QueryException>;

    /// Returns the subset of `new_elements` that haven't been seen before by `unique` or
    /// `unique_all` (Java's `unique(Iterable<T>)` overload).
    // Java: com/google/devtools/build/lib/query2/engine/Uniquifier.java:44 Uniquifier.unique
    fn unique_all(&self, new_elements: &[T]) -> Result<Vec<T>, QueryException>;
}
