//! Port of `com/google/devtools/build/lib/profiler/MemoryProfiler.java`: the memory profile of
//! `--memory_profile` and the heap size after the build that the BEP's `memoryMetrics` reports.
//!
//! Blaze has no JVM. Under ruling D15 (docs/decisions.md, 2026-11-26 and the memory-profile
//! ruling of `ea-5`) the memory bean's figures are the process figures of
//! `blaze_runtime::process_memory`, installed with [`set_platform_memory_bean`]: a heap number is
//! resident memory in the same format, and `gc()` is the allocator's `malloc_trim`.

use std::io::Write;
use std::sync::{Arc, OnceLock};
use std::time::Duration;

use parking_lot::Mutex;

use crate::profiler::ProfilePhase;

/// `java.lang.management.MemoryUsage`.
// Java: java/lang/management/MemoryUsage.java MemoryUsage
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct MemoryUsage {
    pub init: i64,
    pub used: i64,
    pub committed: i64,
    pub max: i64,
}

impl MemoryUsage {
    pub fn new(init: i64, used: i64, committed: i64, max: i64) -> MemoryUsage {
        MemoryUsage {
            init,
            used,
            committed,
            max,
        }
    }
}

/// The part of `java.lang.management.MemoryMXBean` that the profiler uses.
// Java: java/lang/management/MemoryMXBean.java MemoryMXBean
pub trait MemoryMXBean: Send + Sync {
    fn gc(&self);
    fn get_heap_memory_usage(&self) -> MemoryUsage;
    fn get_non_heap_memory_usage(&self) -> MemoryUsage;
}

static PLATFORM_MEMORY_BEAN: OnceLock<Arc<dyn MemoryMXBean>> = OnceLock::new();

/// Installs `ManagementFactory.getMemoryMXBean()` (the server does at startup, with the D15
/// figures). A second call is ignored.
pub fn set_platform_memory_bean(bean: Arc<dyn MemoryMXBean>) {
    let _ = PLATFORM_MEMORY_BEAN.set(bean);
}

/// `ManagementFactory.getMemoryMXBean()`.
fn platform_memory_bean() -> Arc<dyn MemoryMXBean> {
    match PLATFORM_MEMORY_BEAN.get() {
        Some(bean) => bean.clone(),
        None => crate::unported!("ManagementFactory.getMemoryMXBean (no process memory bean set)"),
    }
}

/// `MemoryProfiler.Sleeper`.
// Java: com/google/devtools/build/lib/profiler/MemoryProfiler.java:223 MemoryProfiler.Sleeper
pub trait Sleeper {
    fn sleep(&mut self, duration: Duration);
}

/// The heap and non-heap usage read together.
// Java: com/google/devtools/build/lib/profiler/MemoryProfiler.java:228 MemoryProfiler.HeapAndNonHeap
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct HeapAndNonHeap {
    pub heap: MemoryUsage,
    pub non_heap: MemoryUsage,
}

/// Blaze memory profiler: at each phase it "collects garbage" and writes the heap and non-heap
/// memory usage to the profile file.
// Java: com/google/devtools/build/lib/profiler/MemoryProfiler.java:53 MemoryProfiler
pub struct MemoryProfiler {
    state: Mutex<State>,
}

struct State {
    memory_profile: Option<Box<dyn Write + Send>>,
    current_phase: ProfilePhase,
    heap_used_memory_at_finish: i64,
    /// `memoryProfileStableHeapParameters.gcSpecs`: (number of times to GC, sleep between GCs).
    memory_profile_stable_heap_parameters: Option<Vec<(i32, Duration)>>,
}

impl MemoryProfiler {
    // Java: com/google/devtools/build/lib/profiler/MemoryProfiler.java:57 MemoryProfiler.instance
    pub fn instance() -> &'static MemoryProfiler {
        static INSTANCE: OnceLock<MemoryProfiler> = OnceLock::new();
        INSTANCE.get_or_init(|| MemoryProfiler {
            state: Mutex::new(State {
                memory_profile: None,
                current_phase: ProfilePhase::INIT,
                heap_used_memory_at_finish: 0,
                memory_profile_stable_heap_parameters: None,
            }),
        })
    }

    /// PORT: the `internalJvmObjectPattern` (`--jvm_heap_histogram_internal_object_pattern`) is
    /// not kept: it only feeds `HeapOffsetHelper`, a JVM heap-histogram workaround (see
    /// [`MemoryProfiler::mark_phase`]).
    // Java: com/google/devtools/build/lib/profiler/MemoryProfiler.java:67 MemoryProfiler.setStableMemoryParameters
    pub fn set_stable_memory_parameters(&self, gc_specs: Option<Vec<(i32, Duration)>>) {
        self.state.lock().memory_profile_stable_heap_parameters = gc_specs;
    }

    // Java: com/google/devtools/build/lib/profiler/MemoryProfiler.java:74 MemoryProfiler.start
    pub fn start(&self, out: Option<Box<dyn Write + Send>>) {
        let mut state = self.state.lock();
        state.memory_profile = out;
        state.current_phase = ProfilePhase::INIT;
        state.heap_used_memory_at_finish = 0;
    }

    // Java: com/google/devtools/build/lib/profiler/MemoryProfiler.java:80 MemoryProfiler.stop
    pub fn stop(&self) {
        let mut state = self.state.lock();
        if let Some(mut memory_profile) = state.memory_profile.take() {
            // PrintStream.close() flushes and swallows the IOException.
            let _ = memory_profile.flush();
        }
        state.heap_used_memory_at_finish = 0;
    }

    // Java: com/google/devtools/build/lib/profiler/MemoryProfiler.java:88 MemoryProfiler.getHeapUsedMemoryAtFinish
    pub fn get_heap_used_memory_at_finish(&self) -> i64 {
        self.state.lock().heap_used_memory_at_finish
    }

    /// PORT: `HeapOffsetHelper.getSizeOfFillerArrayOnHeap` is 0: it subtracts the filler arrays
    /// of OpenJDK 21's heap (`isWorkaroundNeeded`), and Blaze's figures are not a JVM heap. The
    /// sleeps between GCs are not interruptible (Java's `Thread.sleep` is); they happen only at
    /// `FINISH` with `--memory_profile_stable_heap_parameters`.
    // Java: com/google/devtools/build/lib/profiler/MemoryProfiler.java:92 MemoryProfiler.markPhase
    pub fn mark_phase(&self, next_phase: ProfilePhase) {
        if self.state.lock().memory_profile.is_none() {
            return;
        }
        let bean = platform_memory_bean();
        let memory_usages =
            self.prepare_bean_and_get_local_min_usage(next_phase, &*bean, &mut ThreadSleeper);
        let mut state = self.state.lock();
        let name = state.current_phase.description();
        let memory_usage = memory_usages.heap;
        let used_memory = memory_usage.used;
        if next_phase == ProfilePhase::FINISH {
            // usedMemory -= HeapOffsetHelper.getSizeOfFillerArrayOnHeap(...): 0, see above.
            state.heap_used_memory_at_finish = used_memory;
        }
        let non_heap = memory_usages.non_heap;
        let text = format!(
            "{name}:heap:init:{}\n{name}:heap:used:{used_memory}\n\
             {name}:heap:commited:{}\n{name}:heap:max:{}\n\
             {name}:non-heap:init:{}\n{name}:non-heap:used:{}\n\
             {name}:non-heap:commited:{}\n{name}:non-heap:max:{}\n",
            memory_usage.init,
            memory_usage.committed,
            memory_usage.max,
            non_heap.init,
            non_heap.used,
            non_heap.committed,
            non_heap.max,
        );
        if let Some(memory_profile) = state.memory_profile.as_mut() {
            // PrintStream.println swallows the IOException (it sets checkError).
            let _ = memory_profile.write_all(text.as_bytes());
        }
        state.current_phase = next_phase;
    }

    // Java: com/google/devtools/build/lib/profiler/MemoryProfiler.java:123 MemoryProfiler.prepareBeanAndGetLocalMinUsage
    pub fn prepare_bean_and_get_local_min_usage(
        &self,
        next_phase: ProfilePhase,
        bean: &dyn MemoryMXBean,
        sleeper: &mut dyn Sleeper,
    ) -> HeapAndNonHeap {
        bean.gc();
        let mut min_heap_used = bean.get_heap_memory_usage();
        let mut min_non_heap_used = bean.get_non_heap_memory_usage();
        let gc_specs = self
            .state
            .lock()
            .memory_profile_stable_heap_parameters
            .clone();
        if next_phase == ProfilePhase::FINISH {
            if let Some(gc_specs) = gc_specs {
                for (j, (num_times_to_do_gc, time_to_sleep_between_gcs)) in
                    gc_specs.iter().enumerate()
                {
                    for i in 0..*num_times_to_do_gc {
                        if j == 0 && i == 0 {
                            continue;
                        }
                        sleeper.sleep(*time_to_sleep_between_gcs);
                        bean.gc();
                        let current_heap_used = bean.get_heap_memory_usage();
                        if current_heap_used.used < min_heap_used.used {
                            min_heap_used = current_heap_used;
                            min_non_heap_used = bean.get_non_heap_memory_usage();
                        }
                    }
                }
            }
        }
        HeapAndNonHeap {
            heap: min_heap_used,
            non_heap: min_non_heap_used,
        }
    }
}

/// `(duration) -> Thread.sleep(duration.toMillis())`.
struct ThreadSleeper;

impl Sleeper for ThreadSleeper {
    fn sleep(&mut self, duration: Duration) {
        std::thread::sleep(Duration::from_millis(duration.as_millis() as u64));
    }
}

#[cfg(test)]
mod tests {
    //! Port of `MemoryProfilerTest`. The two converter tests are in
    //! `blaze_optdefs::command::memory_profiler` (the converter lives there).
    // Java: src/test/java/com/google/devtools/build/lib/profiler/MemoryProfilerTest.java

    use std::collections::VecDeque;

    use super::*;

    /// A `Mockito.mock(MemoryMXBean.class)` with queued answers (the last one repeats) and call
    /// counts.
    #[derive(Default)]
    struct MockBean {
        heap: Mutex<VecDeque<MemoryUsage>>,
        non_heap: Mutex<VecDeque<MemoryUsage>>,
        gc_calls: Mutex<usize>,
        heap_calls: Mutex<usize>,
        non_heap_calls: Mutex<usize>,
    }

    impl MockBean {
        fn new(heap: &[MemoryUsage], non_heap: &[MemoryUsage]) -> MockBean {
            MockBean {
                heap: Mutex::new(heap.iter().copied().collect()),
                non_heap: Mutex::new(non_heap.iter().copied().collect()),
                ..Default::default()
            }
        }

        fn then_return(&self, heap: &[MemoryUsage], non_heap: &[MemoryUsage]) {
            *self.heap.lock() = heap.iter().copied().collect();
            *self.non_heap.lock() = non_heap.iter().copied().collect();
        }

        fn next(queue: &Mutex<VecDeque<MemoryUsage>>) -> MemoryUsage {
            let mut queue = queue.lock();
            if queue.len() > 1 {
                queue.pop_front().unwrap()
            } else {
                *queue.front().unwrap()
            }
        }
    }

    impl MemoryMXBean for MockBean {
        fn gc(&self) {
            *self.gc_calls.lock() += 1;
        }
        fn get_heap_memory_usage(&self) -> MemoryUsage {
            *self.heap_calls.lock() += 1;
            Self::next(&self.heap)
        }
        fn get_non_heap_memory_usage(&self) -> MemoryUsage {
            *self.non_heap_calls.lock() += 1;
            Self::next(&self.non_heap)
        }
    }

    #[derive(Default)]
    struct RecordingSleeper {
        sleeps: Vec<Duration>,
    }

    impl Sleeper for RecordingSleeper {
        fn sleep(&mut self, duration: Duration) {
            self.sleeps.push(duration);
        }
    }

    /// The profiler is a process-wide singleton, as in Java; the tests take turns with it.
    static SERIAL: Mutex<()> = Mutex::new(());

    fn specs(pairs: &[(i32, u64)]) -> Option<Vec<(i32, Duration)>> {
        Some(
            pairs
                .iter()
                .map(|(n, s)| (*n, Duration::from_secs(*s)))
                .collect(),
        )
    }

    fn null_output() -> Option<Box<dyn Write + Send>> {
        Some(Box::new(std::io::sink()))
    }

    #[test]
    fn profiler_does_one_gc_and_no_sleep_normally() {
        let _serial = SERIAL.lock();
        let profiler = MemoryProfiler::instance();
        profiler.set_stable_memory_parameters(specs(&[(1, 10)]));
        profiler.start(null_output());
        let heap_usage = MemoryUsage::new(0, 0, 0, 0);
        let non_heap_usage = MemoryUsage::new(5, 5, 5, 5);
        let bean = MockBean::new(&[heap_usage], &[non_heap_usage]);
        let mut sleeper = RecordingSleeper::default();
        let result = profiler.prepare_bean_and_get_local_min_usage(
            ProfilePhase::ANALYZE,
            &bean,
            &mut sleeper,
        );
        assert_eq!(result.heap, heap_usage);
        assert_eq!(result.non_heap, non_heap_usage);
        assert!(sleeper.sleeps.is_empty());
        assert_eq!(*bean.gc_calls.lock(), 1);
        profiler.prepare_bean_and_get_local_min_usage(ProfilePhase::FINISH, &bean, &mut sleeper);
        assert_eq!(*bean.gc_calls.lock(), 2);
        assert!(sleeper.sleeps.is_empty());
        profiler.stop();
    }

    #[test]
    fn profiler_does_one_gc_and_no_sleep_except_in_finish() {
        let _serial = SERIAL.lock();
        let profiler = MemoryProfiler::instance();
        profiler.set_stable_memory_parameters(specs(&[(3, 10)]));
        profiler.start(null_output());
        let empty_heap = MemoryUsage::new(0, 0, 0, 0);
        let empty_non_heap = MemoryUsage::new(0, 0, 0, 0);
        let bean = MockBean::new(&[empty_heap], &[empty_non_heap]);
        let mut sleeper = RecordingSleeper::default();
        let result = profiler.prepare_bean_and_get_local_min_usage(
            ProfilePhase::ANALYZE,
            &bean,
            &mut sleeper,
        );
        assert_eq!(result.heap, empty_heap);
        assert_eq!(result.non_heap, empty_non_heap);
        assert!(sleeper.sleeps.is_empty());
        assert_eq!(*bean.gc_calls.lock(), 1);
        assert_eq!(*bean.heap_calls.lock(), 1);
        assert_eq!(*bean.non_heap_calls.lock(), 1);
        let heap_usage = MemoryUsage::new(0, 1, 2, 2);
        let non_heap_usage = MemoryUsage::new(2, 2, 2, 2);
        bean.then_return(
            &[
                MemoryUsage::new(5, 5, 5, 5),
                heap_usage,
                MemoryUsage::new(10, 1, 10, 10),
            ],
            &[
                MemoryUsage::new(1, 1, 1, 1),
                non_heap_usage,
                MemoryUsage::new(2, 2, 2, 2),
            ],
        );
        let result = profiler.prepare_bean_and_get_local_min_usage(
            ProfilePhase::FINISH,
            &bean,
            &mut sleeper,
        );
        assert_eq!(result.heap, heap_usage);
        assert_eq!(result.non_heap, non_heap_usage);
        assert_eq!(
            sleeper.sleeps,
            vec![Duration::from_secs(10), Duration::from_secs(10)]
        );
        assert_eq!(*bean.gc_calls.lock(), 4);
        assert_eq!(*bean.heap_calls.lock(), 4);
        // Avoid call when heap usage is not minimal.
        assert_eq!(*bean.non_heap_calls.lock(), 3);
        profiler.stop();
    }

    #[test]
    fn profiler_has_multiple_pairs() {
        let _serial = SERIAL.lock();
        let profiler = MemoryProfiler::instance();
        profiler.set_stable_memory_parameters(specs(&[(2, 1), (3, 4), (5, 6)]));
        profiler.start(null_output());
        let heap_usage = MemoryUsage::new(0, 0, 0, 0);
        let non_heap_usage = MemoryUsage::new(5, 5, 5, 5);
        let bean = MockBean::new(&[heap_usage], &[non_heap_usage]);
        let mut sleeper = RecordingSleeper::default();
        let result = profiler.prepare_bean_and_get_local_min_usage(
            ProfilePhase::ANALYZE,
            &bean,
            &mut sleeper,
        );
        assert_eq!(result.heap, heap_usage);
        assert_eq!(result.non_heap, non_heap_usage);
        assert!(sleeper.sleeps.is_empty());
        assert_eq!(*bean.gc_calls.lock(), 1);
        profiler.prepare_bean_and_get_local_min_usage(ProfilePhase::FINISH, &bean, &mut sleeper);
        // 1 for call to ANALYZE + spec'd runs
        assert_eq!(*bean.gc_calls.lock(), 1 + 2 + 3 + 5);
        let s = Duration::from_secs;
        assert_eq!(
            sleeper.sleeps,
            vec![s(1), s(4), s(4), s(4), s(6), s(6), s(6), s(6), s(6)]
        );
        profiler.stop();
    }

    /// Not in Java: the profile lines of one phase, in Java's format, and the heap size at
    /// `FINISH`.
    #[test]
    fn mark_phase_writes_java_format() {
        struct Shared(Arc<Mutex<Vec<u8>>>);
        impl Write for Shared {
            fn write(&mut self, buf: &[u8]) -> std::io::Result<usize> {
                self.0.lock().extend_from_slice(buf);
                Ok(buf.len())
            }
            fn flush(&mut self) -> std::io::Result<()> {
                Ok(())
            }
        }
        let _serial = SERIAL.lock();
        set_platform_memory_bean(Arc::new(MockBean::new(
            &[MemoryUsage::new(1, 2, 3, 4)],
            &[MemoryUsage::new(0, 0, 0, -1)],
        )));
        let profiler = MemoryProfiler::instance();
        profiler.set_stable_memory_parameters(None);
        let buffer = Arc::new(Mutex::new(Vec::new()));
        profiler.start(Some(Box::new(Shared(buffer.clone()))));
        profiler.mark_phase(ProfilePhase::FINISH);
        assert_eq!(profiler.get_heap_used_memory_at_finish(), 2);
        profiler.stop();
        assert_eq!(profiler.get_heap_used_memory_at_finish(), 0);
        let text = String::from_utf8(buffer.lock().clone()).unwrap();
        let keys: Vec<&str> = text
            .lines()
            .map(|l| l.rsplit_once(':').unwrap().0)
            .collect();
        assert_eq!(
            keys,
            vec![
                "Initialize command:heap:init",
                "Initialize command:heap:used",
                "Initialize command:heap:commited",
                "Initialize command:heap:max",
                "Initialize command:non-heap:init",
                "Initialize command:non-heap:used",
                "Initialize command:non-heap:commited",
                "Initialize command:non-heap:max",
            ]
        );
    }
}
