//! A model of `java.util.PriorityQueue` with a comparator, reproducing its poll order.
//!
//! PORT: `Digraph.subgraph` removes nodes in the order a `PriorityQueue` polls them, with a
//! comparator that reads live edge counts (so the heap may no longer be ordered after other
//! nodes are removed). The order decides the order of the edges the removal adds, which reaches
//! query output. This is the JDK's binary heap (`offer` with `siftUpUsingComparator`, `poll`
//! with `siftDownUsingComparator`), evaluated with the comparator at the time of each call.

use std::cmp::Ordering;

/// `java.util.PriorityQueue<E>` with a comparator passed to each mutating call.
pub(crate) struct JavaPriorityQueue<E> {
    queue: Vec<E>,
}

impl<E: Copy> JavaPriorityQueue<E> {
    pub(crate) fn new() -> Self {
        JavaPriorityQueue { queue: Vec::new() }
    }

    /// `PriorityQueue.offer`.
    pub(crate) fn offer(&mut self, e: E, cmp: &dyn Fn(&E, &E) -> Ordering) {
        let i = self.queue.len();
        self.queue.push(e);
        if i != 0 {
            self.sift_up(i, e, cmp);
        }
    }

    /// `PriorityQueue.poll`.
    pub(crate) fn poll(&mut self, cmp: &dyn Fn(&E, &E) -> Ordering) -> Option<E> {
        if self.queue.is_empty() {
            return None;
        }
        let result = self.queue[0];
        let x = self.queue.pop().unwrap();
        let n = self.queue.len();
        if n > 0 {
            self.sift_down(0, x, n, cmp);
        }
        Some(result)
    }

    /// `PriorityQueue.siftUpUsingComparator`.
    fn sift_up(&mut self, mut k: usize, x: E, cmp: &dyn Fn(&E, &E) -> Ordering) {
        while k > 0 {
            let parent = (k - 1) >> 1;
            let e = self.queue[parent];
            if cmp(&x, &e) != Ordering::Less {
                break;
            }
            self.queue[k] = e;
            k = parent;
        }
        self.queue[k] = x;
    }

    /// `PriorityQueue.siftDownUsingComparator`.
    fn sift_down(&mut self, mut k: usize, x: E, n: usize, cmp: &dyn Fn(&E, &E) -> Ordering) {
        let half = n >> 1;
        while k < half {
            let mut child = (k << 1) + 1;
            let mut c = self.queue[child];
            let right = child + 1;
            if right < n && cmp(&c, &self.queue[right]) == Ordering::Greater {
                child = right;
                c = self.queue[child];
            }
            if cmp(&x, &c) != Ordering::Greater {
                break;
            }
            self.queue[k] = c;
            k = child;
        }
        self.queue[k] = x;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn polls_in_priority_order() {
        let mut q = JavaPriorityQueue::new();
        let cmp = |a: &i32, b: &i32| a.cmp(b);
        for x in [5, 1, 4, 2, 3] {
            q.offer(x, &cmp);
        }
        let mut out = Vec::new();
        while let Some(x) = q.poll(&cmp) {
            out.push(x);
        }
        assert_eq!(out, vec![1, 2, 3, 4, 5]);
    }
}
