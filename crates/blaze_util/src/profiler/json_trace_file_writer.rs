//! Port of `com/google/devtools/build/lib/profiler/JsonTraceFileWriter.java`.

use std::cell::Cell;
use std::collections::HashMap;
use std::io::{self, BufWriter, Write};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, sync_channel};
use std::thread::JoinHandle;

use parking_lot::Mutex;

use super::java_hash_order::{Insertion, java_hash_map_order, long_hash_code};
use super::json_writer::JsonWriter;
use super::profiler_task::ProfilerTask;
use super::thread_metadata::ThreadMetadata;
use super::trace_data::{TaskData, TraceData};

/// The output stream of the profile: the stream itself, or the `GZIPOutputStream` around it.
pub enum ProfileOutputStream {
    Plain(Box<dyn Write + Send>),
    Gzip(flate2::write::GzEncoder<Box<dyn Write + Send>>),
}

impl ProfileOutputStream {
    /// `new GZIPOutputStream(stream)`: the header Java writes (no name, mtime 0, OS byte 255,
    /// "unknown": `java.util.zip.GZIPOutputStream.writeHeader`, `OS_UNKNOWN`) and
    /// `Deflater.DEFAULT_COMPRESSION` (zlib level 6).
    pub fn gzip(stream: Box<dyn Write + Send>) -> Self {
        ProfileOutputStream::Gzip(
            flate2::GzBuilder::new()
                .mtime(0)
                .operating_system(255)
                .write(stream, flate2::Compression::default()),
        )
    }

    /// `close()`: finishes the gzip member and flushes the stream.
    fn close(self) -> io::Result<()> {
        match self {
            ProfileOutputStream::Plain(mut s) => s.flush(),
            ProfileOutputStream::Gzip(g) => {
                let mut s = g.finish()?;
                s.flush()
            }
        }
    }
}

impl Write for ProfileOutputStream {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        match self {
            ProfileOutputStream::Plain(s) => s.write(buf),
            ProfileOutputStream::Gzip(g) => g.write(buf),
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        match self {
            ProfileOutputStream::Plain(s) => s.flush(),
            ProfileOutputStream::Gzip(g) => g.flush(),
        }
    }
}

/// An element of the writer's queue.
pub enum QueuedTraceData {
    Task(TaskData),
    Other(Box<dyn TraceData>),
    /// `POISON_PILL`.
    PoisonPill,
}

// 1_000_000 is a randomly chosen value that is large enough to ensure that:
//   1. If the speed of producers is slower than consumer (normal cases), they don't get overhead
//      on posting new events.
//   2. Otherwise (e.g. with --noslim_profile and --record_full_profiler_data), it eventually
//      slowed down the producers to avoid OOM.
const AVAILABLE_EVENT_SLOTS: usize = 1_000_000;

const SLIM_PROFILE_EVENT_THRESHOLD: i64 = 10_000;
const SLIM_PROFILE_MAXIMAL_PAUSE_NS: i64 = 100_000_000;
const SLIM_PROFILE_MAXIMAL_DURATION_NS: i64 = 250_000_000;

static NEXT_WRITER_ID: AtomicU64 = AtomicU64::new(1);

thread_local! {
    /// `metadataPosted` (a `ThreadLocal` of one writer): the id of the last writer this thread
    /// posted its metadata to.
    static METADATA_POSTED_TO: Cell<u64> = const { Cell::new(0) };
}

/// The data the writer thread needs (`JsonTraceFileWriter`'s final fields).
struct WriterConfig {
    profile_start_time_nanos: i64,
    profile_start_epoch_millis: i64,
    slim_profile: bool,
    build_id: String,
    output_base: String,
    bazel_version: String,
}

/// Writes the profile in Json Trace file format.
// Java: com/google/devtools/build/lib/profiler/JsonTraceFileWriter.java:39 JsonTraceFileWriter
pub struct JsonTraceFileWriter {
    id: u64,
    queue: SyncSender<QueuedTraceData>,
    pending: Mutex<Option<(Receiver<QueuedTraceData>, ProfileOutputStream, WriterConfig)>>,
    thread: Mutex<Option<JoinHandle<Option<io::Error>>>>,
}

impl JsonTraceFileWriter {
    /// `profile_start_time_nanos`: the monotonic clock reading that every event in the profile is
    /// emitted relative to; `profile_start_epoch_millis`: the same instant on the wall clock.
    ///
    /// PORT: `bazel_version` is `BlazeVersionInfo.instance().getReleaseName()`, which Java reads
    /// in `run`; `BlazeVersionInfo` lives in `blaze_runtime`, above this crate.
    // Java: com/google/devtools/build/lib/profiler/JsonTraceFileWriter.java:72 JsonTraceFileWriter.<init>
    pub fn new(
        out_stream: ProfileOutputStream,
        profile_start_time_nanos: i64,
        profile_start_epoch_millis: i64,
        slim_profile: bool,
        output_base: String,
        build_id: String,
        bazel_version: String,
    ) -> Self {
        let (queue, receiver) = sync_channel(AVAILABLE_EVENT_SLOTS);
        JsonTraceFileWriter {
            id: NEXT_WRITER_ID.fetch_add(1, Ordering::Relaxed),
            queue,
            pending: Mutex::new(Some((
                receiver,
                out_stream,
                WriterConfig {
                    profile_start_time_nanos,
                    profile_start_epoch_millis,
                    slim_profile,
                    build_id,
                    output_base,
                    bazel_version,
                },
            ))),
            thread: Mutex::new(None),
        }
    }

    // Java: com/google/devtools/build/lib/profiler/JsonTraceFileWriter.java:89 JsonTraceFileWriter.shutdown
    pub fn shutdown(&self) -> io::Result<()> {
        // Add poison pill to queue and then wait for writer thread to shut down.
        let _ = self.queue.send(QueuedTraceData::PoisonPill);
        let handle = self.thread.lock().take();
        let saved_exception = match handle {
            Some(handle) => match handle.join() {
                Ok(saved) => saved,
                Err(panic) => std::panic::resume_unwind(panic),
            },
            None => None,
        };
        match saved_exception {
            Some(e) => Err(e),
            None => Ok(()),
        }
    }

    // Java: com/google/devtools/build/lib/profiler/JsonTraceFileWriter.java:105 JsonTraceFileWriter.start
    pub fn start(&self) {
        let (receiver, out_stream, config) = self
            .pending
            .lock()
            .take()
            .expect("IllegalThreadStateException: profile-writer-thread already started");
        let handle = std::thread::Builder::new()
            .name("profile-writer-thread".to_string())
            .spawn(move || run(receiver, out_stream, config))
            .expect("failed to start profile-writer-thread");
        *self.thread.lock() = Some(handle);
    }

    // Java: com/google/devtools/build/lib/profiler/JsonTraceFileWriter.java:109 JsonTraceFileWriter.enqueue
    pub fn enqueue(&self, data: QueuedTraceData) {
        // We assign a virtual lane for virtual thread and the metadata for the virtual lane is
        // posted at creation time.
        // PORT: Rust threads are never virtual.
        let first = METADATA_POSTED_TO.with(|posted| {
            if posted.get() != self.id {
                posted.set(self.id);
                true
            } else {
                false
            }
        });
        if first {
            let _ = self
                .queue
                .send(QueuedTraceData::Other(Box::new(ThreadMetadata::current())));
        }
        // PORT: the bounded channel is the semaphore of event slots and the condition the
        // consumer waits on.
        let _ = self.queue.send(data);
    }
}

/// Merges short events of one thread (`--slim_profile`).
// Java: com/google/devtools/build/lib/profiler/JsonTraceFileWriter.java:129 JsonTraceFileWriter.MergedEvent
#[derive(Default)]
struct MergedEvent {
    count: i32,
    start_time_nanos: i64,
    end_time_nanos: i64,
    data: Option<TaskData>,
    /// `None` if merged events have different descriptions.
    description: Option<String>,
}

impl MergedEvent {
    /// Tries to merge an additional event, i.e. if the event is close enough to the already
    /// merged event.
    ///
    /// Returns `None`, if merging was possible. If not mergeable, returns the TaskData of the
    /// previously merged events and clears the internal data structures.
    // Java: com/google/devtools/build/lib/profiler/JsonTraceFileWriter.java:145 JsonTraceFileWriter.MergedEvent.maybeMerge
    fn maybe_merge(&mut self, data: TaskData) -> Option<TaskData> {
        let start_time_nanos = data.start_time_nanos;
        let end_time_nanos = start_time_nanos + data.duration_nanos;
        if self.count > 0
            && start_time_nanos >= self.start_time_nanos
            && end_time_nanos <= self.end_time_nanos
        {
            // Skips child tasks.
            return None;
        }
        if self.count == 0 {
            self.description = Some(data.description.clone());
            self.data = Some(data);
            self.start_time_nanos = start_time_nanos;
            self.end_time_nanos = end_time_nanos;
            self.count += 1;
            None
        } else if start_time_nanos <= self.end_time_nanos + SLIM_PROFILE_MAXIMAL_PAUSE_NS {
            if self.description.as_deref() != Some(data.description.as_str()) {
                self.description = None;
            }
            self.end_time_nanos = end_time_nanos;
            self.count += 1;
            None
        } else {
            let ret = self.get_and_reset();
            self.description = Some(data.description.clone());
            self.data = Some(data);
            self.start_time_nanos = start_time_nanos;
            self.end_time_nanos = end_time_nanos;
            self.count = 1;
            ret
        }
    }

    /// Returns a TaskData object representing the merged data and clears internal data
    /// structures.
    // Java: com/google/devtools/build/lib/profiler/JsonTraceFileWriter.java:178 JsonTraceFileWriter.MergedEvent.getAndReset
    fn get_and_reset(&mut self) -> Option<TaskData> {
        let ret = if self.data.is_none() || self.count <= 1 {
            self.data.take()
        } else {
            let data = self.data.as_ref().unwrap();
            let merged_description = match &self.description {
                Some(description) => format!("{}x {}", self.count, description),
                None => format!("{}x various events", self.count),
            };
            Some(TaskData::new_unknown(
                data.thread_id,
                self.start_time_nanos,
                self.end_time_nanos - self.start_time_nanos,
                merged_description,
            ))
        };
        self.count = 0;
        self.data = None;
        ret
    }
}

// Java: com/google/devtools/build/lib/profiler/JsonTraceFileWriter.java:202 JsonTraceFileWriter.isCandidateForMerging
fn is_candidate_for_merging(data: &TaskData) -> bool {
    data.duration_nanos > 0
        && data.duration_nanos < SLIM_PROFILE_MAXIMAL_DURATION_NS
        && data.type_ != Some(ProfilerTask::CRITICAL_PATH_COMPONENT)
}

fn write_data(
    data: &QueuedTraceData,
    writer: &mut JsonWriter<&mut dyn Write>,
    profile_start_time_nanos: i64,
) -> io::Result<()> {
    match data {
        QueuedTraceData::Task(t) => t.write_trace_data(writer, profile_start_time_nanos),
        QueuedTraceData::Other(o) => o.write_trace_data(writer, profile_start_time_nanos),
        QueuedTraceData::PoisonPill => Ok(()),
    }
}

/// `Instant.ofEpochMilli(millis).toString()` (ISO-8601 in UTC; the fraction is omitted when
/// zero, otherwise written in groups of three digits).
pub(crate) fn instant_to_string(millis: i64) -> String {
    let secs = millis.div_euclid(1000);
    let ms = millis.rem_euclid(1000);
    let days = secs.div_euclid(86_400);
    let sod = secs.rem_euclid(86_400);
    // Days since 1970-01-01 to (year, month, day), proleptic Gregorian.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let mut y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    if m <= 2 {
        y += 1;
    }
    let mut out = format!(
        "{:04}-{:02}-{:02}T{:02}:{:02}:{:02}",
        y,
        m,
        d,
        sod / 3600,
        (sod / 60) % 60,
        sod % 60
    );
    if ms != 0 {
        out.push_str(&format!(".{ms:03}"));
    }
    out.push('Z');
    out
}

/// Saves all gathered information from taskQueue queue to the file. Method is invoked internally
/// by the Timer-based thread and at the end of profiling session. Returns the saved exception.
// Java: com/google/devtools/build/lib/profiler/JsonTraceFileWriter.java:237 JsonTraceFileWriter.run
fn run(
    queue: Receiver<QueuedTraceData>,
    out_stream: ProfileOutputStream,
    config: WriterConfig,
) -> Option<io::Error> {
    let mut received_poison_pill = false;
    let result = (|| -> io::Result<()> {
        // The buffer size of 262144 is chosen at random.
        // Bazel internally stores strings as raw bytes encoded in ISO_8859_1, so we use the
        // same encoding here to also write out raw bytes.
        let mut buffered = BufWriter::with_capacity(262144, out_stream);
        {
            let out: &mut dyn Write = &mut buffered;
            let mut writer = JsonWriter::new(out);
            let start_date = config.profile_start_epoch_millis;
            writer.begin_object()?;
            writer.name("otherData")?;
            writer.begin_object()?;
            writer
                .name("bazel_version")?
                .value_str(&config.bazel_version)?;
            writer.name("build_id")?.value_str(&config.build_id)?;
            writer.name("output_base")?.value_str(&config.output_base)?;
            writer
                .name("date")?
                .value_str(&instant_to_string(start_date))?;
            writer.name("profile_start_ts")?.value_long(start_date)?;
            writer.end_object()?;
            writer.name("traceEvents")?;
            writer.begin_array()?;

            // Generate metadata event for the critical path as thread 0 in disguise.
            let critical_path_metadata =
                ThreadMetadata::create_fake_thread_metadata_for_critical_path();
            critical_path_metadata
                .write_trace_data(&mut writer, config.profile_start_time_nanos)?;

            // PORT: java-hash-order: Java iterates a HashMap<Long, MergedEvent> at the end;
            // `order` keeps the insertion order the iteration is computed from.
            let mut events_per_thread: HashMap<i64, MergedEvent> = HashMap::new();
            let mut order: Vec<i64> = Vec::new();
            let mut event_count: i64 = 0;
            loop {
                let data = match queue.recv() {
                    Ok(QueuedTraceData::PoisonPill) | Err(_) => break,
                    Ok(data) => data,
                };
                event_count += 1;

                match data {
                    QueuedTraceData::Task(task_data)
                        if config.slim_profile
                            && event_count > SLIM_PROFILE_EVENT_THRESHOLD
                            && is_candidate_for_merging(&task_data) =>
                    {
                        let thread_id = task_data.thread_id;
                        let merged = events_per_thread.entry(thread_id).or_insert_with(|| {
                            order.push(thread_id);
                            MergedEvent::default()
                        });
                        if let Some(merged_task_data) = merged.maybe_merge(task_data) {
                            merged_task_data
                                .write_trace_data(&mut writer, config.profile_start_time_nanos)?;
                        }
                    }
                    data => write_data(&data, &mut writer, config.profile_start_time_nanos)?,
                }
            }
            for thread_id in java_hash_map_order(&order, |k| long_hash_code(*k), Insertion::Put) {
                let value = events_per_thread.get_mut(&thread_id).unwrap();
                if let Some(task_data) = value.get_and_reset() {
                    task_data.write_trace_data(&mut writer, config.profile_start_time_nanos)?;
                }
            }
            received_poison_pill = true;
            writer.set_indent("  ");
            writer.end_array()?;
            writer.end_object()?;
            writer.close()?;
        }
        let out_stream = buffered.into_inner().map_err(|e| e.into_error())?;
        out_stream.close()
    })();
    match result {
        Ok(()) => None,
        Err(e) => {
            if !received_poison_pill {
                // We keep emptying the queue, but we can't write anything.
                while let Ok(data) = queue.recv() {
                    if matches!(data, QueuedTraceData::PoisonPill) {
                        break;
                    }
                }
            }
            Some(e)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn instant_to_string_matches_java() {
        assert_eq!(instant_to_string(1795126627689), "2026-11-19T22:17:07.689Z");
        assert_eq!(instant_to_string(0), "1970-01-01T00:00:00Z");
    }
}
