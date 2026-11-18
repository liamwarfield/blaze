//! Port of `query2/engine/QueryUtil.java`: several query utilities to make easier to work with
//! query callbacks and uniquifiers.
//!
//! PORT: `LexicographicallySortedTargetAggregator` / `newLexicographicallySortedTargetAggregator`
//! are `Target`-specific (they sort by `Label`) and live in blaze_query.

use std::collections::{HashMap, HashSet};
use std::hash::Hash;
use std::sync::{Arc, Mutex};

use crate::callback::{
    Callback, IoExceptionSlot, OutputFormatterCallback, OutputFormatterError,
    ThreadSafeOutputFormatterCallback, output_formatter_callback_process,
};
use crate::java_concurrent_hash_map::{JavaConcurrentHashMap, JavaHashCode};
use crate::key_extractor::KeyExtractor;
use crate::min_depth_uniquifier::MinDepthUniquifier;
use crate::query_environment::{QueryEnvironment, ThreadSafeMutableSet};
use crate::query_exception::{QueryError, QueryException};
use crate::query_expression::QueryExpression;
use crate::query_expression_context::QueryExpressionContext;
use crate::query_task_future::{QueryEnvironmentFutures, QueryTaskFuture};
use crate::uniquifier::Uniquifier;

/// A `Callback` that can aggregate all the partial results into a single value.
// Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:46 QueryUtil.AggregateAllCallback
pub trait AggregateAllCallback<T, V>: Callback<T> {
    /// Returns a value representing a combination of all the partial results.
    fn get_result(&self) -> V;
}

/// A `OutputFormatterCallback` that is also a `AggregateAllCallback`.
// Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:52 QueryUtil.AggregateAllOutputFormatterCallback
pub trait AggregateAllOutputFormatterCallback<T, S>:
    ThreadSafeOutputFormatterCallback<T> + AggregateAllCallback<T, S>
{
}

// Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:56 QueryUtil.AggregateAllOutputFormatterCallbackImpl
struct AggregateAllOutputFormatterCallbackImpl<T> {
    result: Arc<dyn ThreadSafeMutableSet<T>>,
    io_exception: IoExceptionSlot,
}

impl<T> AggregateAllOutputFormatterCallbackImpl<T> {
    fn new(env: &(impl QueryEnvironment<T> + ?Sized)) -> Self {
        AggregateAllOutputFormatterCallbackImpl {
            result: env.create_thread_safe_mutable_set(),
            io_exception: IoExceptionSlot::new(),
        }
    }
}

impl<T: Clone> Callback<T> for AggregateAllOutputFormatterCallbackImpl<T> {
    fn process(&self, partial_result: &[T]) -> Result<(), QueryError> {
        output_formatter_callback_process(self, partial_result)
    }
}

impl<T: Clone> OutputFormatterCallback<T> for AggregateAllOutputFormatterCallbackImpl<T> {
    // Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:65 AggregateAllOutputFormatterCallbackImpl.processOutput
    fn process_output(&self, partial_result: &[T]) -> Result<(), OutputFormatterError> {
        self.result.add_all(partial_result);
        Ok(())
    }

    fn io_exception_slot(&self) -> &IoExceptionSlot {
        &self.io_exception
    }
}

impl<T: Clone> ThreadSafeOutputFormatterCallback<T> for AggregateAllOutputFormatterCallbackImpl<T> {}

impl<T: Clone> AggregateAllCallback<T, Arc<dyn ThreadSafeMutableSet<T>>>
    for AggregateAllOutputFormatterCallbackImpl<T>
{
    // Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:70 AggregateAllOutputFormatterCallbackImpl.getResult
    fn get_result(&self) -> Arc<dyn ThreadSafeMutableSet<T>> {
        self.result.clone()
    }
}

impl<T: Clone> AggregateAllOutputFormatterCallback<T, Arc<dyn ThreadSafeMutableSet<T>>>
    for AggregateAllOutputFormatterCallbackImpl<T>
{
}

/// PORT: Java's result is a `CompactHashSet` copy of the list, whose iteration order is the
/// insertion order; here it is that list (the elements are already unique by the environment's
/// set semantics).
// Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:75 QueryUtil.OrderedAggregateAllOutputFormatterCallbackImpl
struct OrderedAggregateAllOutputFormatterCallbackImpl<T> {
    result_set: Arc<dyn ThreadSafeMutableSet<T>>,
    result_list: Mutex<Vec<T>>,
    io_exception: IoExceptionSlot,
}

impl<T: Clone> Callback<T> for OrderedAggregateAllOutputFormatterCallbackImpl<T> {
    fn process(&self, partial_result: &[T]) -> Result<(), QueryError> {
        output_formatter_callback_process(self, partial_result)
    }
}

impl<T: Clone> OutputFormatterCallback<T> for OrderedAggregateAllOutputFormatterCallbackImpl<T> {
    // Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:86 OrderedAggregateAllOutputFormatterCallbackImpl.processOutput
    fn process_output(&self, partial_result: &[T]) -> Result<(), OutputFormatterError> {
        let mut result_list = self.result_list.lock().unwrap();
        for element in partial_result {
            if self.result_set.add(element.clone()) {
                result_list.push(element.clone());
            }
        }
        Ok(())
    }

    fn io_exception_slot(&self) -> &IoExceptionSlot {
        &self.io_exception
    }
}

impl<T: Clone> ThreadSafeOutputFormatterCallback<T>
    for OrderedAggregateAllOutputFormatterCallbackImpl<T>
{
}

impl<T: Clone> AggregateAllCallback<T, Vec<T>>
    for OrderedAggregateAllOutputFormatterCallbackImpl<T>
{
    // Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:95 OrderedAggregateAllOutputFormatterCallbackImpl.getResult
    fn get_result(&self) -> Vec<T> {
        // A CompactHashSet's iteration order is the same as its insertion order.
        self.result_list.lock().unwrap().clone()
    }
}

impl<T: Clone> AggregateAllOutputFormatterCallback<T, Vec<T>>
    for OrderedAggregateAllOutputFormatterCallbackImpl<T>
{
}

/// Returns a fresh `AggregateAllOutputFormatterCallback` instance whose `get_result` returns all
/// the elements of the result in the order they were processed.
// Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:131 QueryUtil.newOrderedAggregateAllOutputFormatterCallback
pub fn new_ordered_aggregate_all_output_formatter_callback<T: Clone + 'static>(
    env: &(impl QueryEnvironment<T> + ?Sized),
) -> Box<dyn AggregateAllOutputFormatterCallback<T, Vec<T>>> {
    Box::new(OrderedAggregateAllOutputFormatterCallbackImpl {
        result_set: env.create_thread_safe_mutable_set(),
        result_list: Mutex::new(Vec::new()),
        io_exception: IoExceptionSlot::new(),
    })
}

/// Returns a fresh `AggregateAllCallback` instance that aggregates all of the values into an
/// `ThreadSafeMutableSet`.
// Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:150 QueryUtil.newAggregateAllCallback
pub fn new_aggregate_all_callback<T: Clone + 'static>(
    env: &(impl QueryEnvironment<T> + ?Sized),
) -> Box<dyn AggregateAllCallback<T, Arc<dyn ThreadSafeMutableSet<T>>>> {
    Box::new(AggregateAllOutputFormatterCallbackImpl::new(env))
}

/// Returns a `QueryTaskFuture` representing the evaluation of `expr` as a mutable, thread safe
/// `Set` comprised of all the results.
///
/// Should only be used by QueryExpressions when it is the only way of achieving correctness.
// Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:161 QueryUtil.evalAll
pub fn eval_all<T: Clone + 'static>(
    env: &dyn QueryEnvironment<T>,
    context: &QueryExpressionContext<T>,
    expr: &QueryExpression,
) -> QueryTaskFuture<Arc<dyn ThreadSafeMutableSet<T>>> {
    let callback = new_aggregate_all_callback(env);
    env.when_succeeds_call(&env.eval(expr, context, &*callback), || {
        Ok(callback.get_result())
    })
}

/// A mutable thread safe `Set` that uses a `KeyExtractor` for determining equality of its
/// elements. This is useful e.g. when `T` isn't guaranteed to have a useful `equals` and
/// `hashCode` but `K` is.
///
/// PORT: java-hash-order. The backing `ConcurrentHashMap<K, T>` is modelled by
/// [`JavaConcurrentHashMap`], so iteration follows Java's order for `K`'s Java `hashCode`.
/// Java's `elementClass` (for `contains(Object)`/`remove(Object)` type checks) is not needed.
// Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:179 QueryUtil.ThreadSafeMutableKeyExtractorBackedSetImpl
pub struct ThreadSafeMutableKeyExtractorBackedSetImpl<T, K> {
    extractor: Arc<dyn KeyExtractor<T, K>>,
    map: Mutex<JavaConcurrentHashMap<K, T>>,
}

impl<T, K: JavaHashCode + Eq> ThreadSafeMutableKeyExtractorBackedSetImpl<T, K> {
    // Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:185 ThreadSafeMutableKeyExtractorBackedSetImpl.<init>
    pub fn new(extractor: Arc<dyn KeyExtractor<T, K>>) -> Self {
        Self::with_concurrency_level(extractor, /* concurrency_level= */ 1)
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:190 ThreadSafeMutableKeyExtractorBackedSetImpl.<init>
    pub fn with_concurrency_level(
        extractor: Arc<dyn KeyExtractor<T, K>>,
        concurrency_level: i32,
    ) -> Self {
        ThreadSafeMutableKeyExtractorBackedSetImpl {
            extractor,
            // new ConcurrentHashMap<>(/*initialCapacity=*/ concurrencyLevel, /*loadFactor=*/ 0.75f)
            map: Mutex::new(JavaConcurrentHashMap::with_capacity(
                concurrency_level,
                0.75,
                1,
            )),
        }
    }
}

impl<T: Clone, K: JavaHashCode + Eq> ThreadSafeMutableSet<T>
    for ThreadSafeMutableKeyExtractorBackedSetImpl<T, K>
{
    // Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:211 ThreadSafeMutableKeyExtractorBackedSetImpl.add
    fn add(&self, element: T) -> bool {
        let key = self.extractor.extract_key(&element);
        self.map.lock().unwrap().put_if_absent(key, element)
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:216 ThreadSafeMutableKeyExtractorBackedSetImpl.contains
    fn contains(&self, element: &T) -> bool {
        let key = self.extractor.extract_key(element);
        self.map.lock().unwrap().contains_key(&key)
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:225 ThreadSafeMutableKeyExtractorBackedSetImpl.remove
    fn remove(&self, element: &T) -> bool {
        let key = self.extractor.extract_key(element);
        self.map.lock().unwrap().remove(&key).is_some()
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:206 ThreadSafeMutableKeyExtractorBackedSetImpl.size
    fn size(&self) -> i32 {
        self.map.lock().unwrap().len() as i32
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:201 ThreadSafeMutableKeyExtractorBackedSetImpl.iterator
    fn to_vec(&self) -> Vec<T> {
        self.map.lock().unwrap().values().cloned().collect()
    }
}

/// A `Uniquifier` whose methods do not throw `QueryException`: every `unique`/`unique_all` call
/// returns `Ok`.
// Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:235 QueryUtil.NonExceptionalUniquifier
pub trait NonExceptionalUniquifier<T>: Uniquifier<T> {}

/// A `NonExceptionalUniquifier` that doesn't do anything and always says an element is unique.
// Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:247 QueryUtil.NullUniquifierImpl
pub struct NullUniquifierImpl;

impl NullUniquifierImpl {
    // Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:254 NullUniquifierImpl.instance
    pub fn instance() -> NullUniquifierImpl {
        NullUniquifierImpl
    }
}

impl<T: Clone> Uniquifier<T> for NullUniquifierImpl {
    // Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:259 NullUniquifierImpl.uniquePure
    fn unique_pure(&self, _new_element: &T) -> bool {
        true
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:264 NullUniquifierImpl.unique
    fn unique(&self, _new_element: &T) -> Result<bool, QueryException> {
        Ok(true)
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:269 NullUniquifierImpl.unique
    fn unique_all(&self, new_elements: &[T]) -> Result<Vec<T>, QueryException> {
        Ok(new_elements.to_vec())
    }
}

impl<T: Clone> NonExceptionalUniquifier<T> for NullUniquifierImpl {}

/// A trivial `Uniquifier` implementation.
///
/// PORT: Java's `ConcurrentHashMap`-backed set is a `HashSet`; its order is never observed.
// Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:275 QueryUtil.UniquifierImpl
pub struct UniquifierImpl<T, K> {
    extractor: Arc<dyn KeyExtractor<T, K>>,
    already_seen: Mutex<HashSet<K>>,
}

impl<T, K: Hash + Eq> UniquifierImpl<T, K> {
    // Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:279 UniquifierImpl.<init>
    pub fn new(extractor: Arc<dyn KeyExtractor<T, K>>) -> Self {
        Self::with_parallelism(extractor, /* query_evaluation_parallelism_level= */ 1)
    }

    /// The parallelism level only sizes Java's map.
    // Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:283 UniquifierImpl.<init>
    pub fn with_parallelism(
        extractor: Arc<dyn KeyExtractor<T, K>>,
        _query_evaluation_parallelism_level: i32,
    ) -> Self {
        UniquifierImpl {
            extractor,
            already_seen: Mutex::new(HashSet::new()),
        }
    }

    fn unique_infallible(&self, element: &T) -> bool {
        let key = self.extractor.extract_key(element);
        self.already_seen.lock().unwrap().insert(key)
    }
}

impl<T: Clone, K: Hash + Eq> Uniquifier<T> for UniquifierImpl<T, K> {
    // Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:296 UniquifierImpl.uniquePure
    fn unique_pure(&self, element: &T) -> bool {
        let key = self.extractor.extract_key(element);
        !self.already_seen.lock().unwrap().contains(&key)
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:301 UniquifierImpl.unique
    fn unique(&self, element: &T) -> Result<bool, QueryException> {
        Ok(self.unique_infallible(element))
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:306 UniquifierImpl.unique
    fn unique_all(&self, new_elements: &[T]) -> Result<Vec<T>, QueryException> {
        let mut result = Vec::new();
        for element in new_elements {
            if self.unique_infallible(element) {
                result.push(element.clone());
            }
        }
        Ok(result)
    }
}

impl<T: Clone, K: Hash + Eq> NonExceptionalUniquifier<T> for UniquifierImpl<T, K> {}

/// State tracked for each key tracked by the uniquifier.
// Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:378 MinDepthUniquifierImpl.KeyState
struct KeyState {
    depth: i32,
    has_been_output: bool,
}

/// A trivial `MinDepthUniquifier` implementation.
// Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:318 QueryUtil.MinDepthUniquifierImpl
pub struct MinDepthUniquifierImpl<T, K> {
    extractor: Arc<dyn KeyExtractor<T, K>>,
    already_seen_at_depth: Mutex<HashMap<K, KeyState>>,
}

impl<T, K: Hash + Eq> MinDepthUniquifierImpl<T, K> {
    /// The concurrency level only sizes Java's map.
    // Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:322 MinDepthUniquifierImpl.<init>
    pub fn new(extractor: Arc<dyn KeyExtractor<T, K>>, _concurrency_level: i32) -> Self {
        MinDepthUniquifierImpl {
            extractor,
            already_seen_at_depth: Mutex::new(HashMap::new()),
        }
    }

    fn unique_at_depth_infallible(&self, new_element: &T, depth: i32) -> bool {
        let key = self.extractor.extract_key(new_element);
        let mut map = self.already_seen_at_depth.lock().unwrap();
        match map.get_mut(&key) {
            None => {
                map.insert(
                    key,
                    KeyState {
                        depth,
                        has_been_output: false,
                    },
                );
                true
            }
            Some(previous_state) => {
                if depth < previous_state.depth {
                    // We've seen the element before, but never at a depth this shallow.
                    previous_state.depth = depth;
                    return true;
                }
                false
            }
        }
    }
}

impl<T: Clone, K: Hash + Eq> MinDepthUniquifier<T> for MinDepthUniquifierImpl<T, K> {
    // Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:329 MinDepthUniquifierImpl.uniqueAtDepthLessThanOrEqualTo
    fn unique_at_depth_less_than_or_equal_to_all(
        &self,
        new_elements: &[T],
        depth: i32,
    ) -> Result<Vec<T>, QueryException> {
        let mut result_builder = Vec::new();
        for new_element in new_elements {
            if self.unique_at_depth_infallible(new_element, depth) {
                result_builder.push(new_element.clone());
            }
        }
        Ok(result_builder)
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:341 MinDepthUniquifierImpl.uniqueAtDepthLessThanOrEqualTo
    fn unique_at_depth_less_than_or_equal_to(
        &self,
        new_element: &T,
        depth: i32,
    ) -> Result<bool, QueryException> {
        Ok(self.unique_at_depth_infallible(new_element, depth))
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:361 MinDepthUniquifierImpl.uniqueAtDepthLessThanOrEqualToPure
    fn unique_at_depth_less_than_or_equal_to_pure(&self, new_element: &T, depth: i32) -> bool {
        let key = self.extractor.extract_key(new_element);
        match self.already_seen_at_depth.lock().unwrap().get(&key) {
            None => true,
            Some(previous_state) => depth < previous_state.depth,
        }
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:367 MinDepthUniquifierImpl.uniqueForOutput
    fn unique_for_output(&self, element: &T) -> bool {
        let key = self.extractor.extract_key(element);
        let mut map = self.already_seen_at_depth.lock().unwrap();
        let key_state = map.get_mut(&key).unwrap_or_else(|| {
            panic!("NullPointerException: Must visit an element before outputting that element.")
        });
        let was_output = key_state.has_been_output;
        key_state.has_been_output = true;
        !was_output
    }

    // Java: com/google/devtools/build/lib/query2/engine/QueryUtil.java:374 MinDepthUniquifierImpl.uniqueElementsCount
    fn unique_elements_count(&self) -> i32 {
        self.already_seen_at_depth.lock().unwrap().len() as i32
    }
}
