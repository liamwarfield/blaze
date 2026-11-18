//! Port of `query2/engine/MinDepthUniquifier.java`.

use crate::query_exception::QueryException;

/// A helper for deduping values that have already been seen at certain "depths".
///
/// This is similar to `Uniquifier`.
// Java: com/google/devtools/build/lib/query2/engine/MinDepthUniquifier.java:27 MinDepthUniquifier
pub trait MinDepthUniquifier<T> {
    /// Returns whether `new_element` hasn't been seen before at depth less than or equal to
    /// `depth` by `unique_at_depth_less_than_or_equal_to` or
    /// `unique_at_depth_less_than_or_equal_to_all`.
    ///
    /// This method is inherently racy wrt the non-pure methods. Only use it if you know what you
    /// are doing.
    // Java: com/google/devtools/build/lib/query2/engine/MinDepthUniquifier.java:41 MinDepthUniquifier.uniqueAtDepthLessThanOrEqualToPure
    fn unique_at_depth_less_than_or_equal_to_pure(&self, new_element: &T, depth: i32) -> bool;

    /// Returns whether `new_element` hasn't been seen before at depth less than or equal to
    /// `depth`.
    ///
    /// There's a natural benign check-then-act race in all concurrent uses of this interface.
    // Java: com/google/devtools/build/lib/query2/engine/MinDepthUniquifier.java:54 MinDepthUniquifier.uniqueAtDepthLessThanOrEqualTo
    fn unique_at_depth_less_than_or_equal_to(
        &self,
        new_element: &T,
        depth: i32,
    ) -> Result<bool, QueryException>;

    /// Returns whether `element` hasn't been output before by the depth-bounded visitation.
    ///
    /// The `unique_at_depth` methods are used to determine whether to *visit* an element again
    /// given the reported depth, and may return `true` multiple times for the same element when
    /// visited at lower depths. These methods necessarily cannot be used to uniquely determine
    /// whether a target has already been produced as query output.
    // Java: com/google/devtools/build/lib/query2/engine/MinDepthUniquifier.java:64 MinDepthUniquifier.uniqueForOutput
    fn unique_for_output(&self, element: &T) -> bool;

    /// Batch version of `unique_at_depth_less_than_or_equal_to` (Java's `Iterable` overload).
    ///
    /// The same benign check-then-act race applies here too.
    // Java: com/google/devtools/build/lib/query2/engine/MinDepthUniquifier.java:71 MinDepthUniquifier.uniqueAtDepthLessThanOrEqualTo
    fn unique_at_depth_less_than_or_equal_to_all(
        &self,
        new_elements: &[T],
        depth: i32,
    ) -> Result<Vec<T>, QueryException>;

    /// Returns the number of unique elements seen by the uniquifier.
    // Java: com/google/devtools/build/lib/query2/engine/MinDepthUniquifier.java:75 MinDepthUniquifier.uniqueElementsCount
    fn unique_elements_count(&self) -> i32;
}
