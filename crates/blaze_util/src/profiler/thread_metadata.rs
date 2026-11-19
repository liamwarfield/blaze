//! Port of `com/google/devtools/build/lib/profiler/ThreadMetadata.java`.

use std::io::{self, Write};
use std::sync::atomic::{AtomicI64, Ordering};

use super::json_writer::JsonWriter;
use super::trace_data::TraceData;

/// The JVM never returns 0 as thread ID, so we use that as fake thread ID for the critical path.
// Java: com/google/devtools/build/lib/profiler/ThreadMetadata.java:47 ThreadMetadata.CRITICAL_PATH_THREAD_ID
pub const CRITICAL_PATH_THREAD_ID: i64 = 0;

static NEXT_THREAD_ID: AtomicI64 = AtomicI64::new(1);

thread_local! {
    static THREAD_ID: i64 = NEXT_THREAD_ID.fetch_add(1, Ordering::Relaxed);
}

/// `Thread.currentThread().getId()`.
///
/// PORT: Rust's `ThreadId::as_u64` is unstable; each thread gets the next number of a process-wide
/// counter the first time it asks (like the JVM, never 0).
pub fn current_thread_id() -> i64 {
    THREAD_ID.with(|id| *id)
}

/// `Thread.currentThread().getName()`.
///
/// PORT: a Rust thread without a name (one started by a library without `Builder::name`) has the
/// JVM's default name scheme, `Thread-<id>`.
pub fn current_thread_name() -> String {
    match std::thread::current().name() {
        Some(name) => name.to_string(),
        None => format!("Thread-{}", current_thread_id()),
    }
}

/// This class is used to insert metadata about threads into the JSON trace profile.
// Java: com/google/devtools/build/lib/profiler/ThreadMetadata.java:24 ThreadMetadata
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ThreadMetadata {
    readable_name: String,
    thread_id: i64,
    sort_index: i64,
}

/// These constants describe ranges of threads. We suppose that there are no more than 10_000
/// threads of each kind, otherwise the profile becomes unreadable anyway. So the sort index of
/// skyframe threads is in range [10_000..20_000) for example.
const SKYFRAME_EVALUATOR_SHIFT: i64 = 10_000;
const SKYFRAME_EVALUATOR_CPU_HEAVY_SHIFT: i64 = 20_000;
const SKYFRAME_EVALUATOR_EXECUTION_SHIFT: i64 = 30_000;
const SKYFRAME_EVALUATOR_MEMOIZING_SHIFT: i64 = 40_000;
const DYNAMIC_EXECUTION_SHIFT: i64 = 100_000;
const INCLUDE_SCANNER_SHIFT: i64 = 200_000;
const CRITICAL_PATH_SORT_INDEX: i64 = 0;
const MAIN_THREAD_SORT_INDEX: i64 = 1;
const GC_THREAD_SORT_INDEX: i64 = 2;
const MAX_SORT_INDEX: i64 = 1_000_000;

impl ThreadMetadata {
    // Java: com/google/devtools/build/lib/profiler/ThreadMetadata.java:29 ThreadMetadata.<init>
    pub fn new(readable_name: impl Into<String>, thread_id: i64) -> Self {
        let readable_name = readable_name.into();
        let sort_index = get_sort_index(&readable_name);
        ThreadMetadata {
            readable_name,
            thread_id,
            sort_index,
        }
    }

    // Java: com/google/devtools/build/lib/profiler/ThreadMetadata.java:35 ThreadMetadata.<init>
    pub fn with_sort_index(readable_name: impl Into<String>, thread_id: i64, sort_index: i64) -> Self {
        ThreadMetadata {
            readable_name: readable_name.into(),
            thread_id,
            sort_index,
        }
    }

    /// The metadata of the current thread.
    // Java: com/google/devtools/build/lib/profiler/ThreadMetadata.java:41 ThreadMetadata.<init>
    pub fn current() -> Self {
        let name = current_thread_name();
        ThreadMetadata {
            readable_name: get_readable_name(&name),
            thread_id: current_thread_id(),
            sort_index: get_sort_index(&name),
        }
    }

    /// In order to display the actions on the critical path in a nice way, we fake a thread for
    /// them.
    // Java: com/google/devtools/build/lib/profiler/ThreadMetadata.java:50 ThreadMetadata.createFakeThreadMetadataForCriticalPath
    pub fn create_fake_thread_metadata_for_critical_path() -> Self {
        Self::with_sort_index(
            "Critical Path",
            CRITICAL_PATH_THREAD_ID,
            CRITICAL_PATH_SORT_INDEX,
        )
    }

    pub fn get_readable_name(&self) -> &str {
        &self.readable_name
    }

    pub fn get_thread_id(&self) -> i64 {
        self.thread_id
    }

    pub fn get_sort_index(&self) -> i64 {
        self.sort_index
    }
}

// Java: com/google/devtools/build/lib/profiler/ThreadMetadata.java:54 ThreadMetadata.isMainThread
fn is_main_thread(thread_name: &str) -> bool {
    thread_name.starts_with("grpc-command")
}

// Java: com/google/devtools/build/lib/profiler/ThreadMetadata.java:58 ThreadMetadata.isGCThread
fn is_gc_thread(thread_name: &str) -> bool {
    thread_name == "Notification Thread"
}

// Java: com/google/devtools/build/lib/profiler/ThreadMetadata.java:62 ThreadMetadata.getReadableName
fn get_readable_name(thread_name: &str) -> String {
    if is_main_thread(thread_name) {
        return "Main Thread".to_string();
    }

    if is_gc_thread(thread_name) {
        return "Garbage Collector".to_string();
    }

    thread_name.to_string()
}

/// `NUMBER_PATTERN.matcher(threadName).find()` with `\d+` (ASCII digits) and
/// `Long.parseLong(group)`; `None` when there is no number or it does not fit a long.
fn first_number(thread_name: &str) -> Option<Option<i64>> {
    let start = thread_name.find(|c: char| c.is_ascii_digit())?;
    let rest = &thread_name[start..];
    let end = rest
        .find(|c: char| !c.is_ascii_digit())
        .unwrap_or(rest.len());
    Some(rest[..end].parse::<i64>().ok())
}

// Java: com/google/devtools/build/lib/profiler/ThreadMetadata.java:95 ThreadMetadata.getSortIndex
fn get_sort_index(thread_name: &str) -> i64 {
    if is_main_thread(thread_name) {
        return MAIN_THREAD_SORT_INDEX;
    }

    if is_gc_thread(thread_name) {
        return GC_THREAD_SORT_INDEX;
    }

    let extracted_number = match first_number(thread_name) {
        None => return MAX_SORT_INDEX,
        // If the number cannot be parsed, e.g. is larger than a long, the actual position is
        // not really relevant.
        Some(None) => return MAX_SORT_INDEX,
        Some(Some(n)) => n,
    };

    if thread_name.starts_with("skyframe-evaluator") {
        if thread_name.starts_with("skyframe-evaluator-cpu-heavy") {
            return SKYFRAME_EVALUATOR_CPU_HEAVY_SHIFT.wrapping_add(extracted_number);
        }
        if thread_name.starts_with("skyframe-evaluator-execution") {
            return SKYFRAME_EVALUATOR_EXECUTION_SHIFT.wrapping_add(extracted_number);
        }
        if thread_name.starts_with("skyframe-evaluator-memoizing") {
            return SKYFRAME_EVALUATOR_MEMOIZING_SHIFT.wrapping_add(extracted_number);
        }
        return SKYFRAME_EVALUATOR_SHIFT.wrapping_add(extracted_number);
    }

    if thread_name.starts_with("dynamic-execution") {
        return DYNAMIC_EXECUTION_SHIFT.wrapping_add(extracted_number);
    }

    if thread_name.starts_with("Include scanner") {
        return INCLUDE_SCANNER_SHIFT.wrapping_add(extracted_number);
    }

    MAX_SORT_INDEX
}

impl TraceData for ThreadMetadata {
    // Java: com/google/devtools/build/lib/profiler/ThreadMetadata.java:146 ThreadMetadata.writeTraceData
    fn write_trace_data(
        &self,
        json_writer: &mut JsonWriter<&mut dyn Write>,
        _profile_start_time_nanos: i64,
    ) -> io::Result<()> {
        json_writer.set_indent("  ");
        json_writer.begin_object()?;
        json_writer.set_indent("");
        json_writer.name("name")?.value_str("thread_name")?;
        json_writer.name("ph")?.value_str("M")?;
        json_writer.name("pid")?.value_long(1)?;
        json_writer.name("tid")?.value_long(self.thread_id)?;
        json_writer.name("args")?;

        json_writer.begin_object()?;
        json_writer.name("name")?.value_str(&self.readable_name)?;
        json_writer.end_object()?;

        json_writer.end_object()?;

        json_writer.set_indent("  ");
        json_writer.begin_object()?;
        json_writer.set_indent("");
        json_writer.name("name")?.value_str("thread_sort_index")?;
        json_writer.name("ph")?.value_str("M")?;
        json_writer.name("pid")?.value_long(1)?;
        json_writer.name("tid")?.value_long(self.thread_id)?;
        json_writer.name("args")?;

        json_writer.begin_object()?;
        json_writer.name("sort_index")?.value_long(self.sort_index)?;
        json_writer.end_object()?;

        json_writer.end_object()?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sort_indices_follow_the_thread_kinds() {
        assert_eq!(get_sort_index("grpc-command-3"), 1);
        assert_eq!(get_sort_index("Notification Thread"), 2);
        assert_eq!(get_sort_index("skyframe-evaluator-12"), 10_012);
        assert_eq!(get_sort_index("skyframe-evaluator-memoizing-3"), 40_003);
        assert_eq!(get_sort_index("skyframe-evaluator"), 1_000_000);
        assert_eq!(get_sort_index("collect-local-resources"), 1_000_000);
        assert_eq!(get_readable_name("grpc-command-0"), "Main Thread");
    }
}
