//! Port of `src/test/java/com/google/devtools/build/lib/profiler/ProfilerTest.java`.
//!
//! PORT: the profiler is a process-wide singleton; JUnit runs these tests one at a time, so they
//! hold `SERIAL` (Rust runs tests on parallel threads). `BlazeClock.setClock(clock)` is the
//! `ManualClock` passed to `start`.

use std::io::{self, Write};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard};

use blaze_runtime::collect_local_resource_usage::CollectLocalResourceUsage;
use blaze_util::profiler::json_profile::{JsonProfile, TraceEvent};
use blaze_util::profiler::thread_metadata::current_thread_id;
use blaze_util::profiler::{Clock, Format, ProfilePhase, Profiler, ProfilerTask, ProfilerTaskSet};

static SERIAL: Mutex<()> = Mutex::new(());

/// `com.google.devtools.build.lib.testutil.ManualClock`.
struct ManualClock(AtomicI64);

impl Clock for ManualClock {
    fn current_time_millis(&self) -> i64 {
        self.0.load(Ordering::SeqCst)
    }

    fn nano_time(&self) -> i64 {
        self.0.load(Ordering::SeqCst) * 1_000_000 + 1000 * 1_000_000_000
    }
}

impl ManualClock {
    fn advance_millis(&self, ms: i64) {
        self.0.fetch_add(ms, Ordering::SeqCst);
    }
}

/// `ByteArrayOutputStream`.
#[derive(Clone, Default)]
struct Buffer(Arc<Mutex<Vec<u8>>>);

impl Write for Buffer {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(buf);
        Ok(buf.len())
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

impl Buffer {
    fn bytes(&self) -> Vec<u8> {
        self.0.lock().unwrap().clone()
    }
}

struct Fixture {
    _guard: MutexGuard<'static, ()>,
    profiler: &'static Profiler,
    clock: Arc<ManualClock>,
}

impl Fixture {
    // Java: src/test/java/com/google/devtools/build/lib/profiler/ProfilerTest.java:69 ProfilerTest.setManualClock
    fn new() -> Self {
        let guard = SERIAL.lock().unwrap_or_else(|e| e.into_inner());
        let profiler = Profiler::instance();
        // A failed test may have left the profiler running.
        let _ = profiler.stop();
        profiler.clear();
        Fixture {
            _guard: guard,
            profiler,
            clock: Arc::new(ManualClock(AtomicI64::new(0))),
        }
    }

    fn collector(&self) -> Arc<CollectLocalResourceUsage> {
        Arc::new(CollectLocalResourceUsage::new(
            blaze_exec::worker::worker_process_metrics::WorkerProcessMetricsCollector::instance(),
            None,
            false,
            false,
            false,
            false,
            false,
            false,
            self.clock.clone(),
        ))
    }

    #[allow(clippy::too_many_arguments)]
    fn start_with(
        &self,
        tasks: ProfilerTaskSet,
        stream: Option<Box<dyn Write + Send>>,
        record_all_durations: bool,
        slim_profile: bool,
        include_primary_output: bool,
        include_target_label: bool,
        include_configuration: bool,
    ) -> io::Result<()> {
        let format = stream.as_ref().map(|_| Format::JSON_TRACE_FILE_FORMAT);
        self.profiler.start(
            tasks,
            stream,
            format,
            "dummy_output_base".into(),
            "00000000-0000-0000-0000-000000000000".into(),
            record_all_durations,
            self.clock.clone(),
            self.clock.nano_time(),
            slim_profile,
            include_primary_output,
            include_target_label,
            include_configuration,
            true,
            self.collector(),
            "release 9.3.2".into(),
        )
    }

    // Java: src/test/java/com/google/devtools/build/lib/profiler/ProfilerTest.java:104 ProfilerTest.start
    fn start(&self, tasks: ProfilerTaskSet) -> Buffer {
        let buffer = Buffer::default();
        self.start_with(
            tasks,
            Some(Box::new(buffer.clone())),
            false,
            false,
            false,
            false,
            false,
        )
        .unwrap();
        buffer
    }
}

impl Drop for Fixture {
    // Java: src/test/java/com/google/devtools/build/lib/profiler/ProfilerTest.java:80 ProfilerTest.forceStopToAvoidPoisoningTheProfiler
    fn drop(&mut self) {
        let _ = self.profiler.stop();
    }
}

fn all_tasks() -> ProfilerTaskSet {
    ProfilerTaskSet::all()
}

fn slowest_tasks() -> ProfilerTaskSet {
    ProfilerTask::VALUES
        .iter()
        .copied()
        .filter(|t| t.collects_slowest_instances())
        .collect()
}

fn parse(buffer: &Buffer) -> JsonProfile {
    JsonProfile::from_reader(&buffer.bytes()[..]).unwrap()
}

// Java: src/test/java/com/google/devtools/build/lib/profiler/ProfilerTest.java:688 ProfilerTest.removeCounterEvents
fn remove_counter_events(events: &[TraceEvent]) -> Vec<TraceEvent> {
    events
        .iter()
        .filter(|e| e.type_() != Some("C"))
        .cloned()
        .collect()
}

// Java: src/test/java/com/google/devtools/build/lib/profiler/ProfilerTest.java:697 ProfilerTest.getTraceEventsForPhase
fn get_trace_events_for_phase(phase: ProfilePhase, trace_events: &[TraceEvent]) -> Vec<TraceEvent> {
    let mut filtered_events = Vec::new();
    let mut found_phase = false;
    for trace_event in trace_events {
        if trace_event.category() == Some(ProfilerTask::PHASE.description()) {
            if found_phase {
                break;
            } else if trace_event.name() == phase.description() {
                found_phase = true;
                continue;
            }
        }
        if found_phase
            && trace_event.name() != "thread_name"
            && trace_event.name() != "thread_sort_index"
        {
            filtered_events.push(trace_event.clone());
        }
    }
    filtered_events
}

fn count(events: &[TraceEvent], f: impl Fn(&TraceEvent) -> bool) -> usize {
    events.iter().filter(|e| f(e)).count()
}

// Java: src/test/java/com/google/devtools/build/lib/profiler/ProfilerTest.java:164 ProfilerTest.testProfilerActivation
#[test]
fn test_profiler_activation() {
    let f = Fixture::new();
    assert!(!f.profiler.is_active());
    f.start(all_tasks());
    assert!(f.profiler.is_active());
    f.profiler.stop().unwrap();
    assert!(!f.profiler.is_active());
}

// Java: src/test/java/com/google/devtools/build/lib/profiler/ProfilerTest.java:174 ProfilerTest.testProfiler
#[test]
fn test_profiler() {
    let f = Fixture::new();
    let p = f.profiler;
    let buffer = f.start(all_tasks());
    p.log_simple_task(f.clock.nano_time(), ProfilerTask::PHASE, "profiler start");
    {
        let _c = p.profile(ProfilerTask::ACTION, "complex task");
        p.log_event(ProfilerTask::PHASE, "event1");
        {
            let _c2 = p.profile(ProfilerTask::ACTION_CHECK, "complex subtask");
            // next task takes less than 10 ms and should be only aggregated
            p.log_simple_task(f.clock.nano_time(), ProfilerTask::VFS_STAT, "stat1");
            let start_time = f.clock.nano_time();
            f.clock.advance_millis(20);
            // this one will take at least 20 ms and should be present
            p.log_simple_task(start_time, ProfilerTask::VFS_STAT, "stat2");
        }
    }
    p.stop().unwrap();
    // all other calls to profiler should be ignored
    p.log_event(ProfilerTask::PHASE, "should be ignored");

    let json_profile = parse(&buffer);
    let events = json_profile.get_trace_events();
    assert_eq!(remove_counter_events(events).len(), 2 + 2 + 2 + 1 + 2 + 1);
    assert_eq!(count(events, |e| e.name() == "thread_name"), 2);
    assert_eq!(count(events, |e| e.name() == "thread_sort_index"), 2);
    assert_eq!(
        count(events, |e| e.category()
            == Some(ProfilerTask::PHASE.description())),
        2
    );
    let vfs: Vec<_> = events
        .iter()
        .filter(|e| e.category() == Some(ProfilerTask::VFS_STAT.description()))
        .collect();
    assert_eq!(vfs.len(), 1);
    assert_eq!(vfs[0].duration(), Some(20_000_000));
    assert_eq!(
        count(events, |e| e
            .category()
            .is_some_and(|c| c.starts_with("action"))),
        2
    );
    assert_eq!(count(events, |e| e.name() == "action count"), 1);
    assert_eq!(count(events, |e| e.name() == "Finishing"), 1);
}

// Java: src/test/java/com/google/devtools/build/lib/profiler/ProfilerTest.java:248 ProfilerTest.testProfilerRecordingAllEvents
#[test]
fn test_profiler_recording_all_events() {
    let f = Fixture::new();
    let p = f.profiler;
    let buffer = Buffer::default();
    f.start_with(
        all_tasks(),
        Some(Box::new(buffer.clone())),
        true,
        false,
        false,
        false,
        false,
    )
    .unwrap();
    {
        let _c = p.profile(ProfilerTask::ACTION, "action task");
        // Next task takes less than 10 ms but should be recorded anyway.
        let before = f.clock.nano_time();
        f.clock.advance_millis(1);
        p.log_simple_task(before, ProfilerTask::VFS_STAT, "stat1");
    }
    p.stop().unwrap();

    let json_profile = parse(&buffer);
    let events = json_profile.get_trace_events();
    assert_eq!(remove_counter_events(events).len(), 2 + 2 + 1 + 1 + 1);
    let vfs: Vec<_> = events
        .iter()
        .filter(|e| e.category() == Some(ProfilerTask::VFS_STAT.description()))
        .collect();
    assert_eq!(vfs.len(), 1);
    assert!(vfs[0].duration().unwrap() / 1_000_000 < ProfilerTask::VFS_STAT.min_duration());
    // There is only one action count event since we only let the clock run for 1ms.
    assert_eq!(count(events, |e| e.name() == "action count"), 1);
}

// Java: src/test/java/com/google/devtools/build/lib/profiler/ProfilerTest.java:306 ProfilerTest.testProfilerWorkerMetrics
#[test]
#[ignore = "needs a mockable WorkerProcessMetricsCollector (Mockito: getLiveWorkerProcessMetrics)"]
fn test_profiler_worker_metrics() {}

// Java: src/test/java/com/google/devtools/build/lib/profiler/ProfilerTest.java:391 ProfilerTest.testProfilerRecordingOnlySlowestEvents
#[test]
fn test_profiler_recording_only_slowest_events() {
    let f = Fixture::new();
    let p = f.profiler;
    let buffer = Buffer::default();
    f.start_with(
        slowest_tasks(),
        Some(Box::new(buffer.clone())),
        true,
        false,
        false,
        false,
        false,
    )
    .unwrap();
    p.log_simple_task_start_stop(10000, 20000, ProfilerTask::VFS_STAT, "stat");
    // Unlike the VFS_STAT event above, the remote execution event will not be recorded since we
    // don't record the slowest remote exec events (see ProfilerTask.java).
    p.log_simple_task_start_stop(
        20000,
        30000,
        ProfilerTask::REMOTE_EXECUTION,
        "remote execution",
    );

    assert!(p.is_profiling(ProfilerTask::VFS_STAT));
    assert!(!p.is_profiling(ProfilerTask::REMOTE_EXECUTION));

    p.stop().unwrap();

    let json_profile = parse(&buffer);
    let filtered = remove_counter_events(json_profile.get_trace_events());
    assert_eq!(filtered.len(), 2 + 2 + 1);
    assert_eq!(
        count(&filtered, |e| e.name() != "thread_name"
            && e.name() != "thread_sort_index"),
        1
    );
}

// Java: src/test/java/com/google/devtools/build/lib/profiler/ProfilerTest.java:444 ProfilerTest.testSlowestTasks
#[test]
fn test_slowest_tasks() {
    let f = Fixture::new();
    let p = f.profiler;
    f.start_with(all_tasks(), None, false, false, false, false, false)
        .unwrap();
    p.log_simple_task_duration(
        Profiler::nano_time_maybe(),
        10_000_000_000,
        ProfilerTask::LOCAL_PARSE,
        "foo",
    );
    let slowest_tasks = p.get_slowest_tasks();
    assert_eq!(slowest_tasks.len(), 1);
    assert_eq!(slowest_tasks[0].get_type(), Some(ProfilerTask::LOCAL_PARSE));
    p.stop().unwrap();
}

// Java: src/test/java/com/google/devtools/build/lib/profiler/ProfilerTest.java:456 ProfilerTest.testGetSlowestTasksCapped
#[test]
fn test_get_slowest_tasks_capped() {
    let f = Fixture::new();
    let p = f.profiler;
    f.start_with(slowest_tasks(), None, false, false, false, false, false)
        .unwrap();
    let min = ProfilerTask::VFS_STAT.min_duration();
    // Add some fast tasks - these shouldn't show up in the slowest.
    for _ in 0..30 {
        p.log_simple_task_start_stop(1, min + 10, ProfilerTask::VFS_STAT, "stat");
    }
    // Add some slow tasks we expect to show up in the slowest.
    let mut expected_slowest_durations = Vec::new();
    for i in 0..30 {
        let fake_duration = min + i + 10_000;
        p.log_simple_task_start_stop(1, fake_duration + 1, ProfilerTask::VFS_STAT, "stat");
        expected_slowest_durations.push(fake_duration);
    }
    // Sprinkle in a whole bunch of fast tasks from different thread ids.
    let threads: Vec<_> = (0..32)
        .map(|_| {
            std::thread::spawn(move || {
                for j in 0..100 {
                    Profiler::instance().log_simple_task_start_stop(
                        1,
                        min + j + 1,
                        ProfilerTask::VFS_STAT,
                        "stat",
                    );
                }
            })
        })
        .collect();
    for t in threads {
        t.join().unwrap();
    }
    let slow_tasks = p.get_slowest_tasks();
    assert_eq!(slow_tasks.len(), 30);
    let mut slowest_durations: Vec<i64> =
        slow_tasks.iter().map(|t| t.get_duration_nanos()).collect();
    slowest_durations.sort();
    expected_slowest_durations.sort();
    assert_eq!(slowest_durations, expected_slowest_durations);
}

// Java: src/test/java/com/google/devtools/build/lib/profiler/ProfilerTest.java:526 ProfilerTest.testProfilerRecordsNothing
#[test]
fn test_profiler_records_nothing() {
    let f = Fixture::new();
    let p = f.profiler;
    let buffer = Buffer::default();
    f.start_with(
        ProfilerTaskSet::empty(),
        Some(Box::new(buffer.clone())),
        true,
        false,
        false,
        false,
        false,
    )
    .unwrap();
    p.log_simple_task_start_stop(10000, 20000, ProfilerTask::VFS_STAT, "stat");
    assert!(ProfilerTask::VFS_STAT.collects_slowest_instances());
    assert!(!p.is_profiling(ProfilerTask::VFS_STAT));
    p.stop().unwrap();
    let json_profile = parse(&buffer);
    assert_eq!(
        count(json_profile.get_trace_events(), |e| e.type_() != Some("M")
            && e.type_() != Some("C")),
        0
    );
}

// Java: src/test/java/com/google/devtools/build/lib/profiler/ProfilerTest.java:572 ProfilerTest.testConcurrentProfiling
#[test]
fn test_concurrent_profiling() {
    let f = Fixture::new();
    let p = f.profiler;
    let buffer = f.start(all_tasks());
    let spawn = |name: &'static str| {
        std::thread::spawn(move || {
            for _ in 0..10000 {
                Profiler::instance().log_event(ProfilerTask::INFO, name);
            }
            current_thread_id()
        })
    };
    let (id1, id2);
    {
        let _c = p.profile(ProfilerTask::PHASE, "main task");
        p.log_event(ProfilerTask::INFO, "starting threads");
        let thread1 = spawn("thread1");
        let thread2 = spawn("thread2");
        id2 = thread2.join().unwrap();
        id1 = thread1.join().unwrap();
        p.log_event(ProfilerTask::INFO, "joined");
    }
    p.stop().unwrap();

    let json_profile = parse(&buffer);
    let events = json_profile.get_trace_events();
    assert_eq!(
        remove_counter_events(events).len(),
        4 + 4 + 1 + 2 + 2 * 10000 + 1
    );
    let tids = |name: &str| {
        let mut t: Vec<i64> = events
            .iter()
            .filter(|e| e.name() == name)
            .map(|e| e.thread_id())
            .collect();
        t.dedup();
        assert_eq!(t.len(), 1);
        t[0]
    };
    let tid1 = tids("thread1");
    let tid2 = tids("thread2");
    assert_ne!(tid1, tid2);
    assert_eq!(tid1, id1);
    assert_eq!(tid2, id2);
}

// Java: src/test/java/com/google/devtools/build/lib/profiler/ProfilerTest.java:628 ProfilerTest.testPhaseTasks
#[test]
fn test_phase_tasks() {
    let f = Fixture::new();
    let p = f.profiler;
    let buffer = f.start(all_tasks());
    p.mark_phase(ProfilePhase::INIT); // Empty phase.
    p.mark_phase(ProfilePhase::TARGET_PATTERN_EVAL);
    std::thread::spawn(|| {
        for _ in 0..100 {
            Profiler::instance().log_event(ProfilerTask::INFO, "thread1");
        }
    })
    .join()
    .unwrap();
    f.clock.advance_millis(1);
    p.mark_phase(ProfilePhase::ANALYZE);
    std::thread::spawn(|| {
        let p = Profiler::instance();
        {
            let _c = p.profile(ProfilerTask::INFO, "complex task");
            for _ in 0..100 {
                p.log_event(ProfilerTask::INFO, "thread2a");
            }
        }
        p.mark_phase(ProfilePhase::EXECUTE);
        for _ in 0..100 {
            p.log_event(ProfilerTask::INFO, "thread2b");
        }
    })
    .join()
    .unwrap();
    p.log_event(ProfilerTask::INFO, "last task");
    f.clock.advance_millis(1);
    p.stop().unwrap();

    let json_profile = parse(&buffer);
    let filtered = remove_counter_events(json_profile.get_trace_events());
    assert_eq!(filtered.len(), 4 + 4 + 4 + 3 * 100 + 1 + 1 + 1);
    assert!(get_trace_events_for_phase(ProfilePhase::INIT, &filtered).is_empty());
    assert_eq!(
        get_trace_events_for_phase(ProfilePhase::TARGET_PATTERN_EVAL, &filtered).len(),
        100
    );
    assert_eq!(
        get_trace_events_for_phase(ProfilePhase::ANALYZE, &filtered).len(),
        101
    );
    assert_eq!(
        get_trace_events_for_phase(ProfilePhase::EXECUTE, &filtered).len(),
        102
    );
}

struct BadClock {
    initial_nano_time: i64,
    num_nano_time_calls: AtomicI64,
}

impl Clock for BadClock {
    fn current_time_millis(&self) -> i64 {
        0
    }
    fn nano_time(&self) -> i64 {
        self.initial_nano_time - (self.num_nano_time_calls.fetch_add(1, Ordering::SeqCst) + 1)
    }
}

// Java: src/test/java/com/google/devtools/build/lib/profiler/ProfilerTest.java:720 ProfilerTest.testResilenceToNonDecreasingNanoTimes
#[test]
fn test_resilence_to_non_decreasing_nano_times() {
    let f = Fixture::new();
    let p = f.profiler;
    let initial_nano_time = f.clock.nano_time();
    let bad_clock = Arc::new(BadClock {
        initial_nano_time,
        num_nano_time_calls: AtomicI64::new(0),
    });
    p.start(
        all_tasks(),
        Some(Box::new(Buffer::default())),
        Some(Format::JSON_TRACE_FILE_FORMAT),
        "dummy_output_base".into(),
        "id".into(),
        false,
        bad_clock.clone(),
        initial_nano_time,
        false,
        false,
        false,
        false,
        true,
        f.collector(),
        "release 9.3.2".into(),
    )
    .unwrap();
    p.log_simple_task(bad_clock.nano_time(), ProfilerTask::INFO, "some task");
    p.stop().unwrap();
}

// Java: src/test/java/com/google/devtools/build/lib/profiler/ProfilerTest.java:765 ProfilerTest.testTaskHistograms
#[test]
fn test_task_histograms() {
    let f = Fixture::new();
    let p = f.profiler;
    f.start_with(all_tasks(), None, false, false, false, false, false)
        .unwrap();
    p.log_simple_task_duration(
        Profiler::nano_time_maybe(),
        10_000_000_000,
        ProfilerTask::INFO,
        "foo",
    );
    let info = p.with_tasks_histograms(|h| {
        let r = h[ProfilerTask::INFO.ordinal()].as_ref().unwrap();
        assert!(!r.is_empty());
        r.to_string()
    });
    assert!(info.contains("'INFO'"), "{info}");
    assert!(info.contains("Count: 1"), "{info}");
    assert!(info.contains("[8192..16384 ms]"), "{info}");
    p.stop().unwrap();
}

struct FailingOutputStream;

impl Write for FailingOutputStream {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(io::Error::other("Expected failure."))
    }
    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn io_exception_in_output_stream() {
    let f = Fixture::new();
    let p = f.profiler;
    f.start_with(
        all_tasks(),
        Some(Box::new(FailingOutputStream)),
        false,
        false,
        false,
        false,
        false,
    )
    .unwrap();
    p.log_simple_task_duration(
        Profiler::nano_time_maybe(),
        10_000_000_000,
        ProfilerTask::INFO,
        "foo",
    );
    let expected = p.stop().unwrap_err();
    assert_eq!(expected.to_string(), "Expected failure.");
}

// Java: src/test/java/com/google/devtools/build/lib/profiler/ProfilerTest.java:782 ProfilerTest.testIOExceptionInOutputStreamBinaryFormat
#[test]
fn test_io_exception_in_output_stream_binary_format() {
    io_exception_in_output_stream();
}

// Java: src/test/java/com/google/devtools/build/lib/profiler/ProfilerTest.java:822 ProfilerTest.testIOExceptionInOutputStreamJsonFormat
#[test]
fn test_io_exception_in_output_stream_json_format() {
    io_exception_in_output_stream();
}

fn action_profile_test(
    include: (bool, bool, bool),
    args: (Option<&str>, &str, Option<&str>, Option<&str>, Option<&str>),
) -> JsonProfile {
    let f = Fixture::new();
    let p = f.profiler;
    let buffer = Buffer::default();
    f.start_with(
        all_tasks(),
        Some(Box::new(buffer.clone())),
        true,
        false,
        include.0,
        include.1,
        include.2,
    )
    .unwrap();
    {
        let _c = p.profile_action(ProfilerTask::ACTION, args.0, args.1, args.2, args.3, args.4);
        p.log_event(ProfilerTask::PHASE, "event1");
    }
    p.stop().unwrap();
    parse(&buffer)
}

// Java: src/test/java/com/google/devtools/build/lib/profiler/ProfilerTest.java:862 ProfilerTest.testPrimaryOutputForAction
#[test]
fn test_primary_output_for_action() {
    let json_profile = action_profile_test(
        (true, false, false),
        (None, "test", Some("foo.out"), Some(""), None),
    );
    assert_eq!(
        count(json_profile.get_trace_events(), |e| e.primary_output_path()
            == Some("foo.out")),
        1
    );
}

// Java: src/test/java/com/google/devtools/build/lib/profiler/ProfilerTest.java:912 ProfilerTest.testTargetLabelForAction
#[test]
fn test_target_label_for_action() {
    let json_profile = action_profile_test(
        (false, true, false),
        (None, "test", Some("foo.out"), Some("//foo:bar"), None),
    );
    assert_eq!(
        count(json_profile.get_trace_events(), |e| e.target_label()
            == Some("//foo:bar")),
        1
    );
}

// Java: src/test/java/com/google/devtools/build/lib/profiler/ProfilerTest.java:962 ProfilerTest.testTargetConfigurationForAction
#[test]
fn test_target_configuration_for_action() {
    let json_profile = action_profile_test(
        (false, false, true),
        (
            None,
            "test",
            Some("foo.out"),
            Some("//foo:bar"),
            Some("012345"),
        ),
    );
    assert_eq!(
        count(json_profile.get_trace_events(), |e| e.configuration()
            == Some("012345")),
        1
    );
}

// Java: src/test/java/com/google/devtools/build/lib/profiler/ProfilerTest.java:1006 ProfilerTest.getJsonProfileOutputStream
fn get_json_profile_output_stream(f: &Fixture, slim_profile: bool) -> Vec<u8> {
    let p = f.profiler;
    let buffer = Buffer::default();
    f.start_with(
        all_tasks(),
        Some(Box::new(buffer.clone())),
        false,
        slim_profile,
        false,
        false,
        false,
    )
    .unwrap();
    let mut cur_time = Profiler::nano_time_maybe();
    for i in 0..100_000i64 {
        let duration = if i % 100 == 0 {
            1_000_000_000
        } else {
            (i % 250) * 1_000_000
        };
        p.log_simple_task_duration(cur_time, duration, ProfilerTask::INFO, "foo");
        cur_time += duration;
    }
    p.stop().unwrap();
    p.clear();
    buffer.bytes()
}

// Java: src/test/java/com/google/devtools/build/lib/profiler/ProfilerTest.java:1049 ProfilerTest.testSlimProfileSize
#[test]
fn test_slim_profile_size() {
    let f = Fixture::new();
    let fat = get_json_profile_output_stream(&f, false);
    let fat_output = String::from_utf8_lossy(&fat).into_owned();
    assert!(!fat_output.contains("x foo"));
    let slim = get_json_profile_output_stream(&f, true);
    let slim_output = String::from_utf8_lossy(&slim).into_owned();
    assert!(slim_output.contains("x foo"));
    assert!(fat.len() >= 8 * slim.len());
    assert!(fat_output.split('\n').count() >= 8 * slim_output.split('\n').count());
}

// Java: src/test/java/com/google/devtools/build/lib/profiler/ProfilerTest.java:1068 ProfilerTest.testProfileMnemonicIncluded
#[test]
fn test_profile_mnemonic_included() {
    let f = Fixture::new();
    let p = f.profiler;
    let buffer = f.start(all_tasks());
    {
        let _c = p.profile_action(
            ProfilerTask::ACTION,
            None,
            "without mnemonic",
            Some(""),
            Some(""),
            None,
        );
        f.clock.advance_millis(100);
    }
    {
        let _c = p.profile_action(
            ProfilerTask::ACTION,
            Some("foo"),
            "with mnemonic",
            Some(""),
            Some(""),
            None,
        );
        f.clock.advance_millis(100);
    }
    p.stop().unwrap();
    let json_profile = parse(&buffer);
    let events = json_profile.get_trace_events();
    let without = events
        .iter()
        .find(|e| e.name() == "without mnemonic")
        .unwrap();
    let with = events.iter().find(|e| e.name() == "with mnemonic").unwrap();
    assert_eq!(without.mnemonic(), None);
    assert_eq!(with.mnemonic(), Some("foo"));
}

fn arg(e: &TraceEvent, k: &str) -> f64 {
    e.args().unwrap().get(k).unwrap().as_f64().unwrap()
}

// Java: src/test/java/com/google/devtools/build/lib/profiler/ProfilerTest.java:1120 ProfilerTest.testActionCacheHitsCounted
#[test]
fn test_action_cache_hits_counted() {
    let f = Fixture::new();
    let p = f.profiler;
    let buffer = f.start(all_tasks());
    // The setup here is one action and one action check taking 200ms (the bucket duration),
    // another action check for 300ms, so spilling over in the next bucket.
    {
        let _c = p.profile_action(
            ProfilerTask::ACTION_CHECK,
            Some("bar action"),
            "with mnemonic",
            Some(""),
            Some(""),
            None,
        );
        {
            let _c2 = p.profile_action(
                ProfilerTask::ACTION,
                None,
                "foo action",
                Some(""),
                Some(""),
                None,
            );
            let _c3 = p.profile_action(
                ProfilerTask::ACTION_CHECK,
                None,
                "bar action",
                Some(""),
                Some(""),
                None,
            );
            f.clock.advance_millis(200);
        }
        f.clock.advance_millis(100);
    }
    p.stop().unwrap();

    let json_profile = parse(&buffer);
    let events: Vec<_> = json_profile
        .get_trace_events()
        .iter()
        .filter(|e| e.name() == "action count")
        .collect();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].thread_id(), current_thread_id());
    // Two cache hit checks and one executed action.
    assert_eq!(events[0].args().unwrap().len(), 2);
    assert_eq!(arg(events[0], "action"), 1.0);
    assert_eq!(arg(events[0], "local action cache"), 2.0);
    // One of the cache hit checks spilled over and used half of the second bucket.
    assert_eq!(arg(events[1], "action"), 0.0);
    assert_eq!(arg(events[1], "local action cache"), 0.5);
}

// Java: src/test/java/com/google/devtools/build/lib/profiler/ProfilerTest.java:1176 ProfilerTest.testLocalActionsCounted
#[test]
fn test_local_actions_counted() {
    let f = Fixture::new();
    let p = f.profiler;
    // Given 2 subsequent local actions with length of 200 ms and 100ms
    let buffer = f.start(all_tasks());
    let start_time1 = f.clock.nano_time();
    f.clock.advance_millis(200);
    p.complete_task(
        start_time1,
        ProfilerTask::LOCAL_ACTION_COUNTS,
        "resource usage 1",
    );
    let start_time2 = f.clock.nano_time();
    f.clock.advance_millis(100);
    p.complete_task(
        start_time2,
        ProfilerTask::LOCAL_ACTION_COUNTS,
        "resource usage 2",
    );
    p.stop().unwrap();

    let json_profile = parse(&buffer);
    let events: Vec<_> = json_profile
        .get_trace_events()
        .iter()
        .filter(|e| e.name() == "action count (local)")
        .collect();
    assert_eq!(events.len(), 2);
    assert_eq!(events[0].thread_id(), current_thread_id());
    assert_eq!(arg(events[0], "local action"), 1.0);
    assert_eq!(arg(events[1], "local action"), 0.5);
}

// Java: src/test/java/com/google/devtools/build/lib/profiler/ProfilerTest.java:1212 ProfilerTest.testVirtualThread
#[test]
#[ignore = "needs virtual threads (Rust has none; Profiler.borrowLane is always null)"]
fn test_virtual_thread() {}

// Java: src/test/java/com/google/devtools/build/lib/profiler/ProfilerTest.java:1286 ProfilerTest.testVirtualThreadTaskStartedAfterStop
#[test]
#[ignore = "needs virtual threads (Rust has none; Profiler.borrowLane is always null)"]
fn test_virtual_thread_task_started_after_stop() {}

// Java: src/test/java/com/google/devtools/build/lib/profiler/ProfilerTest.java:1313 ProfilerTest.testVirtualThreadTaskEndedAfterStop
#[test]
#[ignore = "needs virtual threads (Rust has none; Profiler.borrowLane is always null)"]
fn test_virtual_thread_task_ended_after_stop() {}
