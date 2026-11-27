//! Port of `com.google.devtools.build.lib.collect.Extrema`.

use std::cmp::Ordering;
use std::sync::Arc;

/// `Comparator<T>`.
pub type Comparator<T> = Arc<dyn Fn(&T, &T) -> Ordering + Send + Sync>;

/// A streaming aggregator that, given a `k`, allows a streaming aggregation of `n` elements into
/// the `min(k, n)` most extreme, in `O(min(k, n))` memory and `O(n * log(min(k, n)))` time.
// Java: com/google/devtools/build/lib/collect/Extrema.java:30 Extrema
pub struct Extrema<T> {
    /// `null` for `EmptyExtrema` (`k == 0`).
    regular: Option<RegularExtrema<T>>,
}

impl<T: Clone + 'static> Extrema<T> {
    /// Creates an `Extrema` that can aggregate `n` elements into the `min(k, n)` smallest.
    // Java: com/google/devtools/build/lib/collect/Extrema.java:37 Extrema.min
    pub fn min(k: i32) -> Extrema<T>
    where
        T: Ord + 'static,
    {
        Self::min_with(k, Arc::new(|a: &T, b: &T| a.cmp(b)))
    }

    /// Creates an `Extrema` that can aggregate `n` elements into the `min(k, n)` smallest, per the
    /// given `comparator`.
    // Java: com/google/devtools/build/lib/collect/Extrema.java:45 Extrema.min
    pub fn min_with(k: i32, comparator: Comparator<T>) -> Extrema<T> {
        Self::create(k, comparator)
    }

    /// Creates an `Extrema` that can aggregate `n` elements into the `min(k, n)` largest.
    // Java: com/google/devtools/build/lib/collect/Extrema.java:53 Extrema.max
    pub fn max(k: i32) -> Extrema<T>
    where
        T: Ord + 'static,
    {
        Self::max_with(k, Arc::new(|a: &T, b: &T| a.cmp(b)))
    }

    /// Creates an `Extrema` that can aggregate `n` elements into the `min(k, n)` largest, per the
    /// given `comparator`.
    // Java: com/google/devtools/build/lib/collect/Extrema.java:61 Extrema.max
    pub fn max_with(k: i32, comparator: Comparator<T>) -> Extrema<T>
    where
        T: 'static,
    {
        // comparator.reversed()
        Self::create(k, Arc::new(move |a: &T, b: &T| comparator(b, a)))
    }

    // Java: com/google/devtools/build/lib/collect/Extrema.java:92 Extrema.create
    fn create(k: i32, comparator: Comparator<T>) -> Extrema<T> {
        blaze_util::check_argument!(k >= 0, "invalid k ({}), must be >=0", k);
        Extrema {
            regular: if k == 0 {
                None
            } else {
                Some(RegularExtrema::new(k as usize, comparator))
            },
        }
    }

    /// Aggregates the given element.
    // Java: com/google/devtools/build/lib/collect/Extrema.java:70 Extrema.aggregate
    // Java: com/google/devtools/build/lib/collect/Extrema.java:98 Extrema.EmptyExtrema.aggregate
    // Java: com/google/devtools/build/lib/collect/Extrema.java:140 Extrema.RegularExtrema.aggregate
    pub fn aggregate(&mut self, element: T) {
        if let Some(regular) = &mut self.regular {
            regular.aggregate(element);
        }
    }

    /// For an `Extrema` created with `k` and with `n` calls to `aggregate` since the most recent
    /// call to `clear`, returns the `min(k, n)` most extreme of those elements, sorted from most
    /// extreme to least extreme.
    // Java: com/google/devtools/build/lib/collect/Extrema.java:77 Extrema.getExtremeElements
    // Java: com/google/devtools/build/lib/collect/Extrema.java:103 Extrema.EmptyExtrema.getExtremeElements
    // Java: com/google/devtools/build/lib/collect/Extrema.java:156 Extrema.RegularExtrema.getExtremeElements
    pub fn get_extreme_elements(&self) -> Vec<T> {
        match &self.regular {
            None => Vec::new(),
            Some(regular) => regular.get_extreme_elements(),
        }
    }

    /// Returns true iff `get_extreme_elements` would return an empty result.
    // Java: com/google/devtools/build/lib/collect/Extrema.java:80 Extrema.isEmpty
    // Java: com/google/devtools/build/lib/collect/Extrema.java:113 Extrema.EmptyExtrema.isEmpty
    // Java: com/google/devtools/build/lib/collect/Extrema.java:166 Extrema.RegularExtrema.isEmpty
    pub fn is_empty(&self) -> bool {
        match &self.regular {
            None => true,
            Some(regular) => regular.priority_queue.is_empty(),
        }
    }

    /// Disregards all the elements aggregated already.
    // Java: com/google/devtools/build/lib/collect/Extrema.java:87 Extrema.clear
    // Java: com/google/devtools/build/lib/collect/Extrema.java:108 Extrema.EmptyExtrema.clear
    // Java: com/google/devtools/build/lib/collect/Extrema.java:161 Extrema.RegularExtrema.clear
    pub fn clear(&mut self) {
        if let Some(regular) = &mut self.regular {
            regular.priority_queue.clear();
        }
    }
}

// Java: com/google/devtools/build/lib/collect/Extrema.java:119 Extrema.RegularExtrema
struct RegularExtrema<T> {
    k: usize,
    /// `extremaComparator(a, b) < 0` iff `a` is more extreme than `b`.
    extrema_comparator: Comparator<T>,
    /// A priority queue of the k most extreme elements encountered, ordered backwards
    /// (`extremaComparator.reversed()`), so the least extreme of them is at its head.
    priority_queue: JavaPriorityQueue<T>,
}

impl<T: Clone + 'static> RegularExtrema<T> {
    // Java: com/google/devtools/build/lib/collect/Extrema.java:129 Extrema.RegularExtrema.<init>
    fn new(k: usize, extrema_comparator: Comparator<T>) -> RegularExtrema<T> {
        let reversed = extrema_comparator.clone();
        RegularExtrema {
            k,
            extrema_comparator,
            priority_queue: JavaPriorityQueue::new(Arc::new(move |a: &T, b: &T| reversed(b, a))),
        }
    }

    // Java: com/google/devtools/build/lib/collect/Extrema.java:140 Extrema.RegularExtrema.aggregate
    fn aggregate(&mut self, element: T) {
        if self.priority_queue.size() < self.k {
            self.priority_queue.add(element);
        } else if (self.extrema_comparator)(&element, self.priority_queue.peek()) == Ordering::Less
        {
            // Suppose the least extreme of the current k most extreme elements is e. If the new
            // element is more extreme than e, then (i) it must be among the new k most extreme
            // among the (2) e must not be.
            self.priority_queue.remove();
            self.priority_queue.add(element);
        }
    }

    // Java: com/google/devtools/build/lib/collect/Extrema.java:156 Extrema.RegularExtrema.getExtremeElements
    fn get_extreme_elements(&self) -> Vec<T> {
        // ImmutableList.sortedCopyOf(extremaComparator, priorityQueue): a stable sort of the
        // queue's array order (`PriorityQueue.toArray`), so ties keep the heap's order.
        let mut elements = self.priority_queue.queue.clone();
        let comparator = &self.extrema_comparator;
        elements.sort_by(|a, b| comparator(a, b));
        elements
    }
}

/// The part of `java.util.PriorityQueue` that `Extrema` uses, with the same array layout (its
/// order shows through `toArray`, which `getExtremeElements` sorts stably).
// Java: java/util/PriorityQueue.java PriorityQueue
struct JavaPriorityQueue<T> {
    queue: Vec<T>,
    comparator: Comparator<T>,
}

impl<T> JavaPriorityQueue<T> {
    fn new(comparator: Comparator<T>) -> JavaPriorityQueue<T> {
        JavaPriorityQueue {
            queue: Vec::new(),
            comparator,
        }
    }

    fn size(&self) -> usize {
        self.queue.len()
    }

    fn is_empty(&self) -> bool {
        self.queue.is_empty()
    }

    fn clear(&mut self) {
        self.queue.clear();
    }

    // Java: java/util/PriorityQueue.java PriorityQueue.peek
    fn peek(&self) -> &T {
        // Only called on a full queue (`size() == k > 0`).
        &self.queue[0]
    }

    // Java: java/util/PriorityQueue.java PriorityQueue.offer
    fn add(&mut self, element: T) {
        // siftUpUsingComparator(size, e)
        let mut k = self.queue.len();
        self.queue.push(element);
        while k > 0 {
            let parent = (k - 1) >> 1;
            if (self.comparator)(&self.queue[k], &self.queue[parent]) != Ordering::Less {
                break;
            }
            self.queue.swap(k, parent);
            k = parent;
        }
    }

    /// `remove()` (`poll()`, the queue is never empty here).
    // Java: java/util/PriorityQueue.java PriorityQueue.poll
    fn remove(&mut self) -> T {
        let n = self.queue.len() - 1;
        // x = es[n]; es[n] = null; siftDown(0, x) over the n remaining slots.
        let result = self.queue.swap_remove(0);
        if n > 0 {
            // siftDownUsingComparator(0, x, es, n): the element now at 0 is x.
            let half = n >> 1;
            let mut k = 0;
            while k < half {
                let mut child = (k << 1) + 1;
                let right = child + 1;
                if right < n
                    && (self.comparator)(&self.queue[child], &self.queue[right])
                        == Ordering::Greater
                {
                    child = right;
                }
                if (self.comparator)(&self.queue[k], &self.queue[child]) != Ordering::Greater {
                    break;
                }
                self.queue.swap(k, child);
                k = child;
            }
        }
        result
    }
}

#[cfg(test)]
mod tests {
    //! Port of `ExtremaTest`.
    // Java: src/test/java/com/google/devtools/build/lib/collect/ExtremaTest.java

    use super::*;

    #[test]
    fn handles_dupes() {
        let mut extrema = Extrema::<i32>::min(3);
        for i in [4, 3, 1, 2, 1, 3, 1] {
            extrema.aggregate(i);
        }
        assert_eq!(extrema.get_extreme_elements(), vec![1, 1, 1]);
    }

    #[test]
    fn test_is_empty() {
        let mut extrema = Extrema::<i32>::max(2);
        assert!(extrema.is_empty());
        extrema.aggregate(1);
        assert!(!extrema.is_empty());
    }

    #[test]
    fn test_clear() {
        let mut extrema = Extrema::<i32>::max(2);
        extrema.aggregate(1);
        extrema.clear();
        assert!(extrema.is_empty());
    }

    #[test]
    fn custom_comparator() {
        #[derive(Clone)]
        struct BoxedInt {
            i: i32,
        }
        let mut extrema = Extrema::max_with(
            2,
            Arc::new(|bi1: &BoxedInt, bi2: &BoxedInt| bi1.i.cmp(&bi2.i)),
        );
        for i in [4, 1, 2, 3, 5] {
            extrema.aggregate(BoxedInt { i });
        }
        let extreme_elements: Vec<i32> = extrema
            .get_extreme_elements()
            .into_iter()
            .map(|bi| bi.i)
            .collect();
        assert_eq!(extreme_elements, vec![5, 4]);
    }

    #[test]
    fn min_extrema_small_k() {
        run_range_test(Extrema::min(5), 1, 100, &[1, 2, 3, 4, 5]);
    }

    #[test]
    fn min_extrema_large_k() {
        run_range_test(Extrema::min(10), 1, 5, &[1, 2, 3, 4, 5]);
    }

    #[test]
    fn max_extrema_small_k() {
        run_range_test(Extrema::max(5), 1, 100, &[100, 99, 98, 97, 96]);
    }

    #[test]
    fn max_extrema_large_k() {
        run_range_test(Extrema::max(10), 1, 5, &[5, 4, 3, 2, 1]);
    }

    #[test]
    fn test_empty_extrema() {
        let mut extrema = Extrema::<i32>::max(0);
        extrema.aggregate(1);
        assert!(extrema.is_empty());
        assert!(extrema.get_extreme_elements().is_empty());
        extrema.clear();
        assert!(extrema.is_empty());
        assert!(extrema.get_extreme_elements().is_empty());
    }

    #[test]
    fn test_negative_extrema_disallowed() {
        let thrown = std::panic::catch_unwind(|| Extrema::<i32>::min(-1))
            .err()
            .expect("IllegalArgumentException");
        let message = thrown.downcast_ref::<String>().cloned().unwrap_or_default();
        assert_eq!(
            message,
            "IllegalArgumentException: invalid k (-1), must be >=0"
        );
    }

    fn run_range_test(
        mut extrema: Extrema<i32>,
        left_endpoint_inclusive: i32,
        right_endpoint_inclusive: i32,
        expected: &[i32],
    ) {
        assert!(extrema.get_extreme_elements().is_empty());
        for i in closed_range_shuffled(left_endpoint_inclusive, right_endpoint_inclusive) {
            extrema.aggregate(i);
        }
        assert_eq!(extrema.get_extreme_elements(), expected);
        extrema.clear();
        assert!(extrema.get_extreme_elements().is_empty());
    }

    /// `Collections.shuffle`: a fixed permutation per call (an odd multiplier modulo a power of
    /// two above the range's length visits every index once), varied by the range so the four
    /// range tests see different orders.
    fn closed_range_shuffled(
        left_endpoint_inclusive: i32,
        right_endpoint_inclusive: i32,
    ) -> Vec<i32> {
        let list: Vec<i32> = (left_endpoint_inclusive..=right_endpoint_inclusive).collect();
        let modulus = list.len().next_power_of_two();
        let multiplier = 2 * (list.len() / 3) + 1;
        (0..modulus)
            .map(|i| (i * multiplier + 7) % modulus)
            .filter(|&j| j < list.len())
            .map(|j| list[j])
            .collect()
    }

    /// Ties keep the heap's array order (`sortedCopyOf` is stable over `toArray`).
    #[test]
    fn ties_keep_the_priority_queue_array_order() {
        let mut extrema = Extrema::max_with(
            3,
            Arc::new(|a: &(i32, char), b: &(i32, char)| a.0.cmp(&b.0)),
        );
        for e in [(1, 'a'), (1, 'b'), (1, 'c'), (2, 'd')] {
            extrema.aggregate(e);
        }
        // Heap (reversed comparator, least extreme at the head) after a, b, c: [a, b, c].
        // (2, d) is more extreme than the head a: poll gives [c, b] (c sifted down from the
        // tail: c vs b is a tie, so c stays at 0), then add d: [c, b, d].
        // Stable sort by most extreme first: d, c, b.
        assert_eq!(
            extrema.get_extreme_elements(),
            vec![(2, 'd'), (1, 'c'), (1, 'b')]
        );
    }
}
