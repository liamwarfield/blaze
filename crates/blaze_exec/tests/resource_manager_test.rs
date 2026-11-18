//! Port of `src/test/java/com/google/devtools/build/lib/actions/ResourceManagerTest.java` and
//! `ResourceSetTest.java`.
//!
//! Java leaks the handles it never releases; dropping a Rust `ResourceHandle` releases it, so the
//! ported tests keep (or `mem::forget`) such handles.

use std::sync::Arc;
use std::sync::atomic::{AtomicI32, Ordering};
use std::sync::mpsc;
use std::thread::JoinHandle;
use std::time::Duration;

use blaze_exec::actions::resource_manager::{ResourceHandle, ResourceManager, ResourcePriority};
use blaze_exec::actions::{ExecException, FakeOwner, ResourceSet};
use indexmap::IndexMap;
use parking_lot::{Condvar, Mutex};

const WAIT_TIMEOUT_MILLISECONDS: u64 = 60_000;

/// `java.util.concurrent.CyclicBarrier` (the parts the tests use).
struct CyclicBarrier {
    parties: usize,
    state: Mutex<(usize, u64)>,
    cv: Condvar,
}

impl CyclicBarrier {
    fn new(parties: usize) -> Arc<CyclicBarrier> {
        Arc::new(CyclicBarrier {
            parties,
            state: Mutex::new((0, 0)),
            cv: Condvar::new(),
        })
    }

    fn await_(&self) {
        self.await_timeout(Duration::from_secs(3600));
    }

    /// `await(timeout)`: panics (Java `TimeoutException`) when the barrier does not trip in time.
    fn await_timeout(&self, timeout: Duration) {
        let mut st = self.state.lock();
        let generation = st.1;
        st.0 += 1;
        if st.0 == self.parties {
            st.0 = 0;
            st.1 += 1;
            self.cv.notify_all();
            return;
        }
        let deadline = std::time::Instant::now() + timeout;
        while st.1 == generation {
            if self.cv.wait_until(&mut st, deadline).timed_out() && st.1 == generation {
                panic!("TimeoutException");
            }
        }
    }

    fn get_number_waiting(&self) -> usize {
        self.state.lock().0
    }
}

/// `TestThread`.
struct TestThread {
    handle: JoinHandle<()>,
    done: mpsc::Receiver<()>,
}

struct DoneGuard(mpsc::Sender<()>);

impl Drop for DoneGuard {
    fn drop(&mut self) {
        let _ = self.0.send(());
    }
}

impl TestThread {
    fn start(f: impl FnOnce() + Send + 'static) -> TestThread {
        let (tx, rx) = mpsc::channel();
        let handle = std::thread::spawn(move || {
            let _guard = DoneGuard(tx);
            f();
        });
        TestThread { handle, done: rx }
    }

    /// `joinAndAssertState(timeout)`: `Err` ("is still alive") if the thread did not finish.
    fn join_and_assert_state(self, timeout_ms: u64) -> Result<(), String> {
        match self.done.recv_timeout(Duration::from_millis(timeout_ms)) {
            Err(mpsc::RecvTimeoutError::Timeout) => {
                Err("Thread TestThread is still alive".to_string())
            }
            _ => {
                if let Err(e) = self.handle.join() {
                    std::panic::resume_unwind(e);
                }
                Ok(())
            }
        }
    }

    fn join(self) {
        if let Err(e) = self.handle.join() {
            std::panic::resume_unwind(e);
        }
    }
}

struct Fixture {
    manager: Arc<ResourceManager>,
    counter: AtomicI32,
    sync: Arc<CyclicBarrier>,
    sync2: Arc<CyclicBarrier>,
}

/// `configureResourceManager()`.
fn configure_resource_manager() -> Arc<Fixture> {
    let manager = ResourceManager::new();
    let mut resources = IndexMap::new();
    resources.insert(ResourceSet::MEMORY.to_string(), 1000.0);
    resources.insert(ResourceSet::CPU.to_string(), 1.0);
    resources.insert("gpu".to_string(), 2.0);
    resources.insert("fancyresource".to_string(), 1.5);
    manager.set_available_resources(ResourceSet::create_from_map(
        resources, /* local_test_count= */ 2,
    ));
    manager.reset_resource_usage();
    Arc::new(Fixture {
        manager,
        counter: AtomicI32::new(0),
        sync: CyclicBarrier::new(2),
        sync2: CyclicBarrier::new(2),
    })
}

/// `ResourceOwnerStub`.
fn resource_owner() -> FakeOwner {
    FakeOwner::new("Mnemonic", None, None)
}

impl Fixture {
    fn acquire_with_priority(
        &self,
        ram: f64,
        cpu: f64,
        tests: i32,
        priority: ResourcePriority,
    ) -> Result<ResourceHandle, ExecException> {
        self.manager.acquire_resources(
            &resource_owner(),
            &ResourceSet::create(ram, cpu, tests),
            priority,
        )
    }

    fn acquire(&self, ram: f64, cpu: f64, tests: i32) -> Result<ResourceHandle, ExecException> {
        self.acquire_with_priority(ram, cpu, tests, ResourcePriority::Local)
    }

    fn acquire_extra(
        &self,
        ram: f64,
        cpu: f64,
        extra_resources: &[(&str, f64)],
        tests: i32,
    ) -> Result<ResourceHandle, ExecException> {
        let mut resources = IndexMap::new();
        for (k, v) in extra_resources {
            resources.insert(k.to_string(), *v);
        }
        resources.insert(ResourceSet::MEMORY.to_string(), ram);
        resources.insert(ResourceSet::CPU.to_string(), cpu);
        self.manager.acquire_resources(
            &resource_owner(),
            &ResourceSet::create_from_map(resources, tests),
            ResourcePriority::Local,
        )
    }

    fn release(&self, handle: ResourceHandle) {
        handle.close().unwrap();
    }

    fn validate(&self, count: i32) {
        assert_eq!(self.counter.fetch_add(1, Ordering::SeqCst) + 1, count);
    }

    fn is_available(&self, ram: f64, cpu: f64, local_test_count: i32) -> bool {
        self.manager
            .are_resources_available(&ResourceSet::create(ram, cpu, local_test_count))
            .unwrap()
    }
}

fn yield_until(cond: impl Fn() -> bool) {
    while !cond() {
        std::thread::yield_now();
    }
}

#[test]
fn allow_one_action_on_resource_unavailable_success() {
    let f = configure_resource_manager();
    assert!(!f.manager.in_use());

    // TODO: b/405364605 - Add a test for false case after we start throwing an exception.
    f.manager.set_allow_one_action_on_resource_unavailable(true);

    // When nothing is consuming RAM, then ResourceManager will successfully acquire an over-budget
    // request for RAM if allow_one_action_on_resource_unavailable is set to true.
    let big_ram = 10000.0;
    let big_ram_handle = f.acquire(big_ram, 0.0, 0).unwrap();
    // When RAM is consumed, then Resource Manager will be "in use":
    assert!(f.manager.in_use());
    f.release(big_ram_handle);
    // When that RAM is released, then Resource Manager will not be "in use":
    assert!(!f.manager.in_use());

    // Ditto, for CPU:
    let big_cpu = 10.0;
    let big_cpu_handle = f.acquire(0.0, big_cpu, 0).unwrap();
    assert!(f.manager.in_use());
    f.release(big_cpu_handle);
    assert!(!f.manager.in_use());

    // Ditto, for tests:
    let big_tests = 10;
    let big_tests_handle = f.acquire(0.0, 0.0, big_tests).unwrap();
    assert!(f.manager.in_use());
    f.release(big_tests_handle);
    assert!(!f.manager.in_use());

    // Ditto, for extra resources:
    let big_gpu_handle = f
        .acquire_extra(0.0, 0.0, &[("gpu", 10.0), ("fancyresource", 10.0)], 0)
        .unwrap();
    assert!(f.manager.in_use());
    f.release(big_gpu_handle);
    assert!(!f.manager.in_use());
}

#[test]
fn no_allow_one_action_on_resource_unavailable_exception() {
    let f = configure_resource_manager();
    assert!(!f.manager.in_use());

    // When nothing is consuming RAM, then ResourceManager should fail to acquire an over-budget
    // request for RAM, when allow_one_action_on_resource_unavailable is not explicitly set.
    let big_ram = 10000.0;
    let e = f.acquire(big_ram, 0.0, 0).unwrap_err();
    assert!(matches!(e, ExecException::User(_)), "{e:?}");
}

#[test]
fn test_that_cpu_can_be_overallocated() {
    let f = configure_resource_manager();
    assert!(!f.manager.in_use());

    // Given CPU is partially acquired:
    let _h = f.acquire(0.0, 0.5, 0).unwrap();

    // When a request for CPU is made that would slightly overallocate CPU,
    // Then the request succeeds:
    let f1 = f.clone();
    let thread1 = TestThread::start(move || {
        f1.acquire(0.0, 0.6, 0).unwrap();
    });
    thread1.join_and_assert_state(10000).unwrap();
}

#[test]
fn test_that_cpu_allocation_is_noncommutative() {
    let f = configure_resource_manager();
    assert!(!f.manager.in_use());

    // Given that CPU has a small initial allocation:
    let small_cpu_handle = f.acquire(0.0, 0.099, 0).unwrap();

    // When a request for a large CPU allocation is made,
    // Then the request succeeds:
    let f1 = f.clone();
    let thread1 = TestThread::start(move || {
        let handle = f1.acquire(0.0, 0.99, 0).unwrap();
        // Cleanup
        f1.release(handle);
    });
    thread1.join_and_assert_state(10000).unwrap();

    // Cleanup
    f.release(small_cpu_handle);
    assert!(!f.manager.in_use());

    // Given that CPU has a large initial allocation:
    let _h = f.acquire(0.0, 0.99, 0).unwrap();

    // When a request for a small CPU allocation is made,
    // Then the request fails:
    let f2 = f.clone();
    let thread2 = TestThread::start(move || {
        let _ = f2.acquire(0.0, 0.099, 0);
    });
    let e = thread2.join_and_assert_state(1000).unwrap_err();
    assert!(e.contains("is still alive"));
    // Note that this behavior is surprising and probably not intended.
}

#[test]
fn test_that_ram_cannot_be_overallocated() {
    let f = configure_resource_manager();
    assert!(!f.manager.in_use());

    // Given RAM is partially acquired:
    let _h = f.acquire(500.0, 0.0, 0).unwrap();

    // When a request for RAM is made that would slightly overallocate RAM,
    // Then the request fails (got timeout):
    let f1 = f.clone();
    let thread1 = TestThread::start(move || {
        let _ = f1.acquire(600.0, 0.0, 0);
    });
    let e = thread1.join_and_assert_state(1000).unwrap_err();
    assert!(e.contains("is still alive"));
}

#[test]
fn test_that_tests_cannot_be_overallocated() {
    let f = configure_resource_manager();
    assert!(!f.manager.in_use());

    // Given test count is partially acquired:
    let _h = f.acquire(0.0, 0.0, 1).unwrap();

    // When a request for tests is made that would slightly overallocate tests,
    // Then the request fails:
    let f1 = f.clone();
    let thread1 = TestThread::start(move || {
        let _ = f1.acquire(0.0, 0.0, 2);
    });
    let e = thread1.join_and_assert_state(1000).unwrap_err();
    assert!(e.contains("is still alive"));
}

#[test]
fn test_that_extra_resources_cannot_be_overallocated() {
    let f = configure_resource_manager();
    assert!(!f.manager.in_use());

    // Given a partially acquired extra resources:
    let _h = f.acquire_extra(0.0, 0.0, &[("gpu", 1.0)], 1).unwrap();

    // When a request for extra resources is made that would overallocate,
    // Then the request fails:
    let f1 = f.clone();
    let thread1 = TestThread::start(move || {
        let _ = f1.acquire_extra(0.0, 0.0, &[("gpu", 1.1)], 0);
    });
    let e = thread1.join_and_assert_state(1000).unwrap_err();
    assert!(e.contains("is still alive"));
}

#[test]
fn test_has_resources() {
    let f = configure_resource_manager();
    assert!(!f.manager.in_use());
    assert!(!f.manager.thread_has_resources());
    let gpu_handle = f.acquire_extra(1.0, 0.1, &[("gpu", 1.0)], 1).unwrap();
    assert!(f.manager.thread_has_resources());

    // We have resources in this thread - make sure other threads are not affected.
    let f1 = f.clone();
    let thread1 = TestThread::start(move || {
        assert!(!f1.manager.thread_has_resources());
        let handle = f1.acquire(1.0, 0.0, 0).unwrap();
        assert!(f1.manager.thread_has_resources());
        f1.release(handle);
        assert!(!f1.manager.thread_has_resources());
        let handle = f1.acquire(0.0, 0.1, 0).unwrap();
        assert!(f1.manager.thread_has_resources());
        f1.release(handle);
        assert!(!f1.manager.thread_has_resources());
        let handle = f1.acquire(0.0, 0.0, 1).unwrap();
        assert!(f1.manager.thread_has_resources());
        f1.release(handle);
        assert!(!f1.manager.thread_has_resources());
        let handle = f1.acquire_extra(0.0, 0.0, &[("gpu", 1.0)], 0).unwrap();
        assert!(f1.manager.thread_has_resources());
        f1.release(handle);
        assert!(!f1.manager.thread_has_resources());
    });
    thread1.join_and_assert_state(10000).unwrap();

    f.release(gpu_handle);
    assert!(!f.manager.thread_has_resources());
    assert!(!f.manager.in_use());
}

#[test]
fn test_concurrent_large_requests() {
    let f = configure_resource_manager();
    assert!(!f.manager.in_use());

    let requested_ram = 700.0;
    let requested_cpu = 0.7;

    let f1 = f.clone();
    let thread1 = TestThread::start(move || {
        let handle1 = f1.acquire(requested_ram, requested_cpu, 0).unwrap();
        f1.sync.await_();
        f1.validate(1);
        f1.sync.await_();
        // Wait till other thread will be locked.
        yield_until(|| f1.manager.get_wait_count() != 0);
        f1.release(handle1);
        assert_eq!(f1.manager.get_wait_count(), 0);
        let handle2 = f1.acquire(requested_ram, requested_cpu, 0).unwrap(); // Will be blocked by the thread2.
        f1.validate(3);
        f1.release(handle2);
    });
    let f2 = f.clone();
    let thread2 = TestThread::start(move || {
        f2.sync2.await_();
        assert!(!f2.is_available(requested_ram, requested_cpu, 0));
        let handle = f2.acquire(requested_ram, requested_cpu, 0).unwrap(); // Will be blocked by the thread1.
        f2.validate(2);
        f2.sync2.await_();
        // Wait till other thread will be locked.
        yield_until(|| f2.manager.get_wait_count() != 0);
        f2.release(handle);
    });

    f.sync.await_timeout(Duration::from_secs(1));
    assert!(f.manager.in_use());
    assert_eq!(f.manager.get_wait_count(), 0);
    f.sync2.await_timeout(Duration::from_secs(1));
    f.sync.await_timeout(Duration::from_secs(1));
    f.sync2.await_timeout(Duration::from_secs(1));
    thread1.join_and_assert_state(1000).unwrap();
    thread2.join_and_assert_state(1000).unwrap();
    assert!(!f.manager.in_use());
}

#[test]
fn test_interrupted_acquisition_clears_resources() {
    let f = configure_resource_manager();
    assert!(!f.manager.in_use());
    // Acquire a small amount of resources so that future requests can block (the initial request
    // always succeeds even if it's for too much).
    let f0 = f.clone();
    let small_thread = TestThread::start(move || {
        // Java never releases this handle.
        std::mem::forget(f0.acquire(400.0, 0.0, 0).unwrap());
    });
    small_thread
        .join_and_assert_state(WAIT_TIMEOUT_MILLISECONDS)
        .unwrap();
    let f1 = f.clone();
    let thread1 = TestThread::start(move || {
        blaze_shell::thread_interrupt::interrupt_current();
        let r = f1.acquire(700.0, 0.0, 0);
        assert!(matches!(r, Err(ExecException::Interrupted)));
    });
    thread1
        .join_and_assert_state(WAIT_TIMEOUT_MILLISECONDS)
        .unwrap();
    // This should process the queue. If the request from above is still present, it will take
    // all the available memory. But it shouldn't.
    f.manager.set_available_resources(ResourceSet::create(
        /* memory_mb= */ 2000.0, /* cpu= */ 1.0, /* local_test_count= */ 2,
    ));
    let f2 = f.clone();
    let thread2 = TestThread::start(move || {
        let handle = f2.acquire(700.0, 0.0, 0).unwrap();
        f2.release(handle);
    });
    thread2
        .join_and_assert_state(WAIT_TIMEOUT_MILLISECONDS)
        .unwrap();
}

#[test]
fn test_out_of_order_allocation() {
    let f = configure_resource_manager();
    let sync3 = CyclicBarrier::new(2);
    let sync4 = CyclicBarrier::new(2);

    assert!(!f.manager.in_use());

    let f1 = f.clone();
    let thread1 = TestThread::start(move || {
        f1.sync.await_();
        let handle = f1.acquire(900.0, 0.5, 0).unwrap(); // Will be blocked by the main thread.
        f1.validate(5);
        f1.release(handle);
        f1.sync.await_();
    });

    let f2 = f.clone();
    let start_thread2 = move || {
        TestThread::start(move || {
            // Wait till other thread will be locked
            yield_until(|| f2.manager.get_wait_count() != 0);
            let handle = f2.acquire(100.0, 0.1, 0).unwrap();
            f2.validate(2);
            f2.release(handle);
            f2.sync2.await_();
            let handle = f2.acquire(200.0, 0.5, 0).unwrap();
            f2.validate(4);
            f2.sync2.await_();
            f2.release(handle);
        })
    };

    let f3 = f.clone();
    let s3 = sync3.clone();
    let thread3 = TestThread::start(move || {
        let handle = f3.acquire(100.0, 0.4, 0).unwrap();
        s3.await_();
        s3.await_();
        f3.release(handle);
    });

    let f4 = f.clone();
    let s4 = sync4.clone();
    let thread4 = TestThread::start(move || {
        let handle = f4.acquire(750.0, 0.3, 0).unwrap();
        s4.await_();
        s4.await_();
        f4.release(handle);
    });

    // Lock 900 MB, 0.9 CPU in total (spread over three threads so that we can individually
    // release parts of it).
    let handle = f.acquire(50.0, 0.2, 0).unwrap();
    sync3.await_timeout(Duration::from_secs(1));
    sync4.await_timeout(Duration::from_secs(1));
    f.validate(1);

    // Start thread1, which will try to acquire 900 MB, 0.5 CPU, but can't, so it has to wait.
    f.sync.await_timeout(Duration::from_secs(1));

    // Start thread2, which will successfully acquire and release 100 MB, 0.1 CPU.
    let thread2 = start_thread2();
    // Signal thread2 to acquire 200 MB and 0.5 CPU, which will block.
    f.sync2.await_timeout(Duration::from_secs(1));

    // Waiting till both threads are locked.
    yield_until(|| f.manager.get_wait_count() >= 2);

    f.validate(3); // Thread1 is now first in the queue and Thread2 is second.

    // Release 100 MB, 0.4 CPU. This allows Thread2 to continue out of order.
    sync3.await_timeout(Duration::from_secs(1));
    f.sync2.await_timeout(Duration::from_secs(1));

    // Release 750 MB, 0.3 CPU. At this point thread1 will finally acquire resources.
    sync4.await_timeout(Duration::from_secs(1));
    f.sync.await_timeout(Duration::from_secs(1));

    // Release all remaining resources.
    f.release(handle);
    thread1.join();
    thread2.join();
    thread3.join();
    thread4.join();

    assert!(!f.manager.in_use());
}

fn start_acquire_release_thread(
    f: &Arc<Fixture>,
    priority: ResourcePriority,
) -> Arc<CyclicBarrier> {
    let sync = CyclicBarrier::new(2);
    let f1 = f.clone();
    let s = sync.clone();
    // The thread is detached, as in Java.
    let _ = TestThread::start(move || {
        let handle = f1.acquire_with_priority(700.0, 0.0, 0, priority).unwrap();
        s.await_();
        f1.release(handle);
    });
    sync
}

#[test]
fn test_release_high_priority_first() {
    let f = configure_resource_manager();
    assert!(!f.manager.in_use());

    let f1 = f.clone();
    let _thread1 = TestThread::start(move || {
        let handle = f1.acquire(700.0, 0.0, 0).unwrap();
        f1.sync.await_();
        f1.sync2.await_();
        f1.release(handle);
    });
    // Wait for thread1 to have acquired its RAM
    f.sync.await_timeout(Duration::from_secs(1));

    // Set up threads that compete for resources
    let sync_dynamic_standalone =
        start_acquire_release_thread(&f, ResourcePriority::DynamicStandalone);
    yield_until(|| f.manager.get_wait_count() >= 1);
    let sync_dynamic_worker = start_acquire_release_thread(&f, ResourcePriority::DynamicWorker);
    yield_until(|| f.manager.get_wait_count() >= 2);
    let sync_local = start_acquire_release_thread(&f, ResourcePriority::Local);
    yield_until(|| f.manager.get_wait_count() >= 3);

    f.sync2.await_();

    yield_until(|| {
        sync_local.get_number_waiting()
            + sync_dynamic_worker.get_number_waiting()
            + sync_dynamic_standalone.get_number_waiting()
            != 0
    });
    assert_eq!(f.manager.get_wait_count(), 2);
    assert_eq!(sync_local.get_number_waiting(), 1);
    sync_local.await_timeout(Duration::from_secs(1));

    yield_until(|| {
        sync_dynamic_worker.get_number_waiting() + sync_dynamic_standalone.get_number_waiting() != 0
    });
    assert_eq!(sync_dynamic_worker.get_number_waiting(), 1);
    assert_eq!(f.manager.get_wait_count(), 1);

    sync_dynamic_worker.await_timeout(Duration::from_secs(1));
    yield_until(|| sync_dynamic_standalone.get_number_waiting() != 0);
    assert_eq!(sync_dynamic_standalone.get_number_waiting(), 1);
    assert_eq!(f.manager.get_wait_count(), 0);
    sync_dynamic_standalone.await_timeout(Duration::from_secs(1));
}

#[test]
fn test_release_dynamic_lifo() {
    let f = configure_resource_manager();
    assert!(!f.manager.in_use());

    let f1 = f.clone();
    let _thread1 = TestThread::start(move || {
        let handle = f1.acquire(700.0, 0.0, 0).unwrap();
        f1.sync.await_();
        f1.sync2.await_();
        f1.release(handle);
    });
    // Wait for thread1 to have acquired enough RAM to block the other threads.
    f.sync.await_timeout(Duration::from_secs(1));

    // Set up threads that compete for resources
    let sync_dynamic_standalone1 =
        start_acquire_release_thread(&f, ResourcePriority::DynamicStandalone);
    yield_until(|| f.manager.get_wait_count() >= 1);
    let sync_dynamic_worker1 = start_acquire_release_thread(&f, ResourcePriority::DynamicWorker);
    yield_until(|| f.manager.get_wait_count() >= 2);
    let sync_dynamic_standalone2 =
        start_acquire_release_thread(&f, ResourcePriority::DynamicStandalone);
    yield_until(|| f.manager.get_wait_count() >= 3);
    let sync_dynamic_worker2 = start_acquire_release_thread(&f, ResourcePriority::DynamicWorker);
    yield_until(|| f.manager.get_wait_count() >= 4);

    // Wewease the kwaken!
    f.sync2.await_();

    yield_until(|| {
        sync_dynamic_standalone1.get_number_waiting()
            + sync_dynamic_standalone2.get_number_waiting()
            + sync_dynamic_worker1.get_number_waiting()
            + sync_dynamic_worker2.get_number_waiting()
            != 0
    });
    assert_eq!(f.manager.get_wait_count(), 3);
    assert_eq!(sync_dynamic_worker2.get_number_waiting(), 1);
    sync_dynamic_worker2.await_timeout(Duration::from_secs(1));

    yield_until(|| {
        sync_dynamic_standalone1.get_number_waiting()
            + sync_dynamic_standalone2.get_number_waiting()
            + sync_dynamic_worker1.get_number_waiting()
            != 0
    });
    assert_eq!(f.manager.get_wait_count(), 2);
    assert_eq!(sync_dynamic_worker1.get_number_waiting(), 1);
    sync_dynamic_worker1.await_timeout(Duration::from_secs(1));

    yield_until(|| {
        sync_dynamic_standalone1.get_number_waiting()
            + sync_dynamic_standalone2.get_number_waiting()
            != 0
    });
    assert_eq!(f.manager.get_wait_count(), 1);
    assert_eq!(sync_dynamic_standalone2.get_number_waiting(), 1);
    sync_dynamic_standalone2.await_timeout(Duration::from_secs(1));

    yield_until(|| sync_dynamic_standalone1.get_number_waiting() != 0);
    assert_eq!(f.manager.get_wait_count(), 0);
    assert_eq!(sync_dynamic_standalone1.get_number_waiting(), 1);
    sync_dynamic_standalone1.await_timeout(Duration::from_secs(1));
}

#[test]
fn test_nonexisting_resource() {
    let f = configure_resource_manager();
    // If we try to use nonexisting resource we should return an error
    let f1 = f.clone();
    let thread1 = TestThread::start(move || {
        let e = f1
            .acquire_extra(0.0, 0.0, &[("nonexisting", 1.0)], 0)
            .unwrap_err();
        assert!(matches!(e, ExecException::User(_)), "{e:?}");
    });
    thread1.join_and_assert_state(1000).unwrap();
}

#[test]
#[ignore = "needs persistent workers (WorkerPool, WorkerKey), which are not ported"]
fn test_acquire_with_worker_acquire_and_release() {}

#[test]
#[ignore = "needs persistent workers (WorkerPool, WorkerKey), which are not ported"]
fn test_invalidate_and_close() {}

#[test]
#[ignore = "needs MachineLoadProvider (CPU load scheduling), which is not ported"]
fn test_cpu_load_scheduling_cant_acquire_while_window_full() {}

#[test]
#[ignore = "needs MachineLoadProvider (CPU load scheduling), which is not ported"]
fn test_cpu_load_scheduling_cant_acquire_while_cpu_loaded() {}

#[test]
#[ignore = "needs MachineLoadProvider (CPU load scheduling), which is not ported"]
fn test_cpu_load_scheduling_success() {}

#[test]
#[ignore = "needs MachineLoadProvider (CPU load scheduling), which is not ported"]
fn test_cpu_load_scheduling_cant_acquire_x3_cpu() {}

// ---- ResourceSetTest ----

#[test]
#[ignore = "needs ResourceSet.ResourceSetConverter, which is not ported"]
fn test_converter_parses_expected_format() {}

#[test]
#[ignore = "needs ResourceSet.ResourceSetConverter, which is not ported"]
fn test_converter_throws_when_given_insufficient_inputs() {}

#[test]
#[ignore = "needs ResourceSet.ResourceSetConverter, which is not ported"]
fn test_converter_throws_when_given_too_many_inputs() {}

#[test]
#[ignore = "needs ResourceSet.ResourceSetConverter, which is not ported"]
fn test_converter_throws_when_given_negative_inputs() {}

fn map(entries: &[(&str, f64)]) -> IndexMap<String, f64> {
    entries.iter().map(|(k, v)| (k.to_string(), *v)).collect()
}

#[test]
fn test_equals() {
    // EqualsTester: members of a group are equal; members of different groups are not.
    let groups: Vec<Vec<ResourceSet>> = vec![
        vec![
            ResourceSet::create_with_ram_cpu(100.0, 1.0),
            ResourceSet::create_with_ram_cpu(100.0, 1.0),
            ResourceSet::create_from_map(map(&[("memory", 100.0), ("cpu", 1.0)]), 0),
        ],
        vec![ResourceSet::create_with_ram_cpu(200.0, 1.0)],
        vec![ResourceSet::create_with_ram_cpu(100.0, 2.0)],
        vec![ResourceSet::create_from_map(
            map(&[("memory", 100.0), ("cpu", 1.0)]),
            5,
        )],
        vec![ResourceSet::create_from_map(
            map(&[("memory", 100.0), ("cpu", 1.0), ("my_resource", 1.0)]),
            5,
        )],
        vec![ResourceSet::create_from_map(
            map(&[("memory", 100.0), ("cpu", 1.0), ("my_resource", 2.0)]),
            5,
        )],
    ];
    for (i, gi) in groups.iter().enumerate() {
        for (j, gj) in groups.iter().enumerate() {
            for a in gi {
                for b in gj {
                    assert_eq!(a == b, i == j, "{a} vs {b}");
                }
            }
        }
    }
}

#[test]
fn with_resource_overrides_no_args_returns_same_instance() {
    let base = ResourceSet::create_with_ram_cpu(100.0, 1.0);
    // PORT: isSameInstanceAs; Rust values have no identity, so equality is checked.
    assert_eq!(base.with_resource_overrides(&[]), base);
}

#[test]
fn with_resource_overrides_all_empty_returns_same_instance() {
    let base = ResourceSet::create_with_ram_cpu(100.0, 1.0);
    assert_eq!(
        base.with_resource_overrides(&[IndexMap::new(), IndexMap::new()]),
        base
    );
}

#[test]
fn with_resource_overrides_overrides_existing_resource() {
    let base = ResourceSet::create_with_ram_cpu(100.0, 1.0);
    let result = base.with_resource_overrides(&[map(&[("cpu", 4.0)])]);
    assert_eq!(result.get_cpu_usage(), 4.0);
    assert_eq!(result.get_memory_mb(), 100.0);
}

#[test]
fn with_resource_overrides_adds_new_resource() {
    let base = ResourceSet::create_with_ram_cpu(100.0, 1.0);
    let result = base.with_resource_overrides(&[map(&[("gpu", 2.0)])]);
    assert_eq!(result.get("gpu"), 2.0);
    assert_eq!(result.get_cpu_usage(), 1.0);
    assert_eq!(result.get_memory_mb(), 100.0);
}

#[test]
fn with_resource_overrides_later_override_wins() {
    let base = ResourceSet::create_with_ram_cpu(100.0, 1.0);
    let result = base.with_resource_overrides(&[map(&[("cpu", 2.0)]), map(&[("cpu", 8.0)])]);
    assert_eq!(result.get_cpu_usage(), 8.0);
}

#[test]
fn with_resource_overrides_merges_across_overrides() {
    let base = ResourceSet::create_with_ram_cpu(100.0, 1.0);
    let result = base.with_resource_overrides(&[
        map(&[("cpu", 4.0), ("gpu", 1.0)]),
        map(&[("memory", 2000.0)]),
    ]);
    assert_eq!(result.get_cpu_usage(), 4.0);
    assert_eq!(result.get_memory_mb(), 2000.0);
    assert_eq!(result.get("gpu"), 1.0);
}

#[test]
fn with_resource_overrides_preserves_local_test_count() {
    let base = ResourceSet::create(100.0, 1.0, 5);
    let result = base.with_resource_overrides(&[map(&[("cpu", 4.0)])]);
    assert_eq!(result.get_local_test_count(), 5);
}

#[test]
fn with_resource_overrides_skips_empty_among_non_empty() {
    let base = ResourceSet::create_with_ram_cpu(100.0, 1.0);
    let result =
        base.with_resource_overrides(&[IndexMap::new(), map(&[("cpu", 4.0)]), IndexMap::new()]);
    assert_eq!(result.get_cpu_usage(), 4.0);
}
