//! Port of `lib/dynamic/ShrinkableSemaphore.java`.

use std::sync::atomic::{AtomicI32, Ordering};

use parking_lot::Mutex;

/// A semaphore whose number of permits can be reduced (a `java.util.concurrent.Semaphore` with
/// `reducePermits`): the permits available can be negative after a reduction.
// Java: java/util/concurrent/Semaphore.java Semaphore
#[derive(Debug)]
pub struct Semaphore {
    permits: Mutex<i32>,
}

impl Semaphore {
    pub fn new(permits: i32) -> Semaphore {
        Semaphore {
            permits: Mutex::new(permits),
        }
    }

    /// `tryAcquire()`.
    pub fn try_acquire(&self) -> bool {
        let mut permits = self.permits.lock();
        if *permits > 0 {
            *permits -= 1;
            true
        } else {
            false
        }
    }

    /// `release(n)`.
    pub fn release_n(&self, n: i32) {
        *self.permits.lock() += n;
    }

    /// `release()`.
    pub fn release(&self) {
        self.release_n(1);
    }

    /// `reducePermits(reduction)`.
    pub fn reduce_permits(&self, reduction: i32) {
        *self.permits.lock() -= reduction;
    }

    /// `availablePermits()`.
    pub fn available_permits(&self) -> i32 {
        *self.permits.lock()
    }
}

/// A Semaphore that can dynamically change the number of permits. Used to restrict local
/// execution while there are many local jobs in the queue.
// Java: com/google/devtools/build/lib/dynamic/ShrinkableSemaphore.java:28 ShrinkableSemaphore
#[derive(Debug)]
pub struct ShrinkableSemaphore {
    semaphore: Semaphore,
    total_permits: i32,
    max_load: i32,
    load_factor: f64,
    permits_previously_shrunk_by: AtomicI32,
}

impl ShrinkableSemaphore {
    /// Creates a new semaphore with `permits` permits (the "number of things that can run"),
    /// `max_load` the maximum load (the "number of things that can be waiting"), and
    /// `load_factor` how much the load reduces the permits (0 or 1 for no shrinking).
    // Java: com/google/devtools/build/lib/dynamic/ShrinkableSemaphore.java:47 ShrinkableSemaphore.<init>
    pub fn new(permits: i32, max_load: i32, load_factor: f64) -> ShrinkableSemaphore {
        ShrinkableSemaphore {
            semaphore: Semaphore::new(permits),
            total_permits: permits,
            max_load,
            load_factor,
            permits_previously_shrunk_by: AtomicI32::new(0),
        }
    }

    /// Shrink the number of permits by `shrink_factor` (0 to 1) of the total, but never below 1.
    // Java: com/google/devtools/build/lib/dynamic/ShrinkableSemaphore.java:60 ShrinkableSemaphore.setShrinkFactor
    fn set_shrink_factor(&self, shrink_factor: f64) {
        let shrink_by =
            ((self.total_permits as f64 * shrink_factor) as i32).min(self.total_permits - 1);
        let old_shrink = self.permits_previously_shrunk_by.load(Ordering::SeqCst);
        let reduction = shrink_by - old_shrink;
        if self
            .permits_previously_shrunk_by
            .compare_exchange(old_shrink, shrink_by, Ordering::SeqCst, Ordering::SeqCst)
            .is_ok()
        {
            if reduction > 0 {
                self.semaphore.reduce_permits(reduction);
            } else {
                self.semaphore.release_n(-reduction);
            }
        }
    }

    /// Updates the load. If the load factor is between 0 and 1, the permits are shrunk
    /// proportionally to the load. Panics with Java's `IllegalArgumentException` on a negative
    /// load.
    // Java: com/google/devtools/build/lib/dynamic/ShrinkableSemaphore.java:80 ShrinkableSemaphore.updateLoad
    pub fn update_load(&self, load: i32) {
        if self.load_factor > 0.0 && self.load_factor < 1.0 {
            if load < 0 {
                panic!("java.lang.IllegalArgumentException: Cannot have negative load");
            }
            // (1 - loadFactor) * (((float) load) / maxLoad): a float division, then a double.
            self.set_shrink_factor(
                (1.0 - self.load_factor) * ((load as f32 / self.max_load as f32) as f64),
            );
        }
    }

    /// `tryAcquire()`.
    pub fn try_acquire(&self) -> bool {
        self.semaphore.try_acquire()
    }

    /// `release()`.
    pub fn release(&self) {
        self.semaphore.release();
    }

    /// `release(permits)`.
    pub fn release_n(&self, permits: i32) {
        self.semaphore.release_n(permits);
    }

    /// `acquire(permits)`: waits until `permits` permits are available.
    pub fn acquire_n(&self, permits: i32) {
        loop {
            {
                let mut available = self.semaphore.permits.lock();
                if *available >= permits {
                    *available -= permits;
                    return;
                }
            }
            std::thread::sleep(std::time::Duration::from_millis(1));
        }
    }

    /// `availablePermits()`.
    pub fn available_permits(&self) -> i32 {
        self.semaphore.available_permits()
    }
}

// Port of src/test/java/com/google/devtools/build/lib/dynamic/ShrinkableSemaphoreTest.java.
#[cfg(test)]
mod tests {
    use super::*;

    // Java: src/test/java/com/google/devtools/build/lib/dynamic/ShrinkableSemaphoreTest.java:27 ShrinkableSemaphoreTest.testUpdateLoad_shrinksUnderLoad
    #[test]
    fn test_update_load_shrinks_under_load() {
        let sem = ShrinkableSemaphore::new(12, 30, 0.75);
        assert_eq!(sem.available_permits(), 12);
        sem.update_load(0);
        assert_eq!(sem.available_permits(), 12);
        sem.update_load(10); // 1/3 load * 0.75 factor = 1/12 reduction
        assert_eq!(sem.available_permits(), 11);
        sem.update_load(20);
        assert_eq!(sem.available_permits(), 10);
        sem.update_load(30);
        assert_eq!(sem.available_permits(), 9);
        sem.update_load(4);
        assert_eq!(sem.available_permits(), 12);
    }

    // Java: src/test/java/com/google/devtools/build/lib/dynamic/ShrinkableSemaphoreTest.java:43 ShrinkableSemaphoreTest.testUpdateLoad_shrinksProperlyWhenPermitsTaken
    #[test]
    fn test_update_load_shrinks_properly_when_permits_taken() {
        let sem = ShrinkableSemaphore::new(12, 30, 0.5);
        assert_eq!(sem.available_permits(), 12);
        sem.acquire_n(5);
        sem.update_load(0);
        assert_eq!(sem.available_permits(), 7);
        sem.update_load(10); // 1/3 load * 0.5 factor = 1/6 reduction, and 5 acquired
        assert_eq!(sem.available_permits(), 5);
        sem.acquire_n(5);
        assert_eq!(sem.available_permits(), 0);
        sem.update_load(20); // More permits temporarily taken than available
        assert_eq!(sem.available_permits(), -2);
        sem.release();
        assert_eq!(sem.available_permits(), -1);
        sem.update_load(30); // Only 6 permits allowed under load, 9 still acquired
        assert_eq!(sem.available_permits(), -3);
        sem.update_load(10); // Now 10 permits allowed under load, 9 still acquired
        assert_eq!(sem.available_permits(), 1);
    }

    // Java: src/test/java/com/google/devtools/build/lib/dynamic/ShrinkableSemaphoreTest.java:64 ShrinkableSemaphoreTest.testUpdateLoad_noShrinkWithZeroFactor
    #[test]
    fn test_update_load_no_shrink_with_zero_factor() {
        let sem = ShrinkableSemaphore::new(12, 30, 0.0);
        assert_eq!(sem.available_permits(), 12);
        sem.acquire_n(5);
        sem.update_load(0);
        assert_eq!(sem.available_permits(), 7);
        sem.update_load(30);
        assert_eq!(sem.available_permits(), 7);
        sem.release_n(2);
        assert_eq!(sem.available_permits(), 9);
    }

    // Java: src/test/java/com/google/devtools/build/lib/dynamic/ShrinkableSemaphoreTest.java:77 ShrinkableSemaphoreTest.testUpdateLoad_noShrinkBelowZero
    #[test]
    fn test_update_load_no_shrink_below_zero() {
        let sem = ShrinkableSemaphore::new(12, 30, 0.5);
        assert_eq!(sem.available_permits(), 12);
        sem.update_load(60);
        assert_eq!(sem.available_permits(), 1);
        sem.update_load(80);
        assert_eq!(sem.available_permits(), 1);
        sem.update_load(40);
        assert_eq!(sem.available_permits(), 4);
    }
}
