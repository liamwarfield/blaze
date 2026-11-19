//! Port of `com/google/devtools/build/lib/profiler/ProfilerTask.java`.

/// Threshold for skipping tasks in the profile, in nanoseconds.
// Java: com/google/devtools/build/lib/profiler/ProfilerTask.java:103 ProfilerTask.Threshold
const TEN_MILLIS: i64 = 10_000_000;
const FIFTY_MILLIS: i64 = 50_000_000;

macro_rules! profiler_tasks {
    ($($name:ident ($desc:expr $(, $min:expr $(, $slowest:expr)?)?)),* $(,)?) => {
        /// All possible types of profiler tasks. Each type also defines description and minimum
        /// duration in nanoseconds for it to be recorded as separate event and not just be
        /// aggregated into the parent event.
        // Java: com/google/devtools/build/lib/profiler/ProfilerTask.java:23 ProfilerTask
        #[allow(non_camel_case_types)]
        #[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
        #[repr(u8)]
        pub enum ProfilerTask {
            $($name),*
        }

        impl ProfilerTask {
            /// `ProfilerTask.values()`, in declaration order.
            pub const VALUES: &'static [ProfilerTask] = &[$(ProfilerTask::$name),*];

            /// `name()`.
            pub fn name(self) -> &'static str {
                match self {
                    $(ProfilerTask::$name => stringify!($name)),*
                }
            }

            /// Human readable description for the task (`description`).
            pub fn description(self) -> &'static str {
                match self {
                    $(ProfilerTask::$name => $desc),*
                }
            }

            /// Threshold for skipping tasks in the profile in nanoseconds, unless
            /// --record_full_profiler_data is used (`minDuration`; -1 without a threshold).
            // Java: com/google/devtools/build/lib/profiler/ProfilerTask.java:125 ProfilerTask.<init>
            pub fn min_duration(self) -> i64 {
                match self {
                    $(ProfilerTask::$name => profiler_tasks!(@min $($min)?)),*
                }
            }

            /// Whether the Profiler collects the slowest instances of this task.
            // Java: com/google/devtools/build/lib/profiler/ProfilerTask.java:139 ProfilerTask.collectsSlowestInstances
            pub fn collects_slowest_instances(self) -> bool {
                match self {
                    $(ProfilerTask::$name => profiler_tasks!(@slowest $($($slowest)?)?)),*
                }
            }
        }
    };
    (@min) => { -1 };
    (@min None) => { -1 };
    (@min $min:expr) => { $min };
    (@slowest) => { false };
    (@slowest $s:expr) => { $s };
}

profiler_tasks! {
    PHASE("build phase marker"),
    ACTION("action processing"),
    DISCOVER_INPUTS("discover inputs"),
    ACTION_CHECK("action dependency checking", TEN_MILLIS),
    ACTION_LOCK("action resource lock", TEN_MILLIS),
    ACTION_UPDATE("update action information", TEN_MILLIS),
    ACTION_COMPLETE("complete action execution"),
    ACTION_REWINDING("prepare action rewind plan"),
    BZLMOD("bazel module processing"),
    INFO("general information"),
    CREATE_PACKAGE("package creation"),
    REMOTE_EXECUTION("remote action execution"),
    LOCAL_EXECUTION("local action execution"),
    SCANNER("include scanner"),
    LOCAL_PARSE("Local parse to prepare for remote execution", FIFTY_MILLIS, true),
    UPLOAD_TIME("Remote execution upload time", FIFTY_MILLIS),
    REMOTE_PROCESS_TIME("Remote execution process wall time", FIFTY_MILLIS),
    REMOTE_QUEUE("Remote execution queuing time", FIFTY_MILLIS),
    REMOTE_SETUP("Remote execution setup", FIFTY_MILLIS),
    FETCH("Remote execution file fetching", FIFTY_MILLIS),
    LOCAL_PROCESS_TIME("Local execution process wall time", FIFTY_MILLIS),
    VFS_STAT("VFS stat", TEN_MILLIS, true),
    VFS_DIR("VFS readdir", TEN_MILLIS, true),
    VFS_READLINK("VFS readlink", TEN_MILLIS, true),
    // TODO(olaola): rename to VFS_DIGEST. This refers to all digest function computations.
    VFS_MD5("VFS md5", TEN_MILLIS, true),
    VFS_XATTR("VFS xattr", TEN_MILLIS, true),
    VFS_DELETE("VFS delete", TEN_MILLIS),
    VFS_OPEN("VFS open", TEN_MILLIS, true),
    VFS_READ("VFS read", TEN_MILLIS, true),
    VFS_WRITE("VFS write", TEN_MILLIS, true),
    VFS_GLOB("globbing", -1, true),
    VFS_VMFS_STAT("VMFS stat", TEN_MILLIS),
    VFS_VMFS_DIR("VMFS readdir", TEN_MILLIS),
    VFS_VMFS_READ("VMFS read", TEN_MILLIS),
    WAIT("thread wait", TEN_MILLIS),
    THREAD_NAME("thread name"), // Do not use directly!
    THREAD_SORT_INDEX("thread sort index"),
    SKYFRAME_EVAL("skyframe evaluator"),
    SKYFUNCTION("skyfunction"),
    CRITICAL_PATH("critical path"),
    CRITICAL_PATH_COMPONENT("critical path component"),
    HANDLE_GC_NOTIFICATION("gc notification"),
    LOCAL_ACTION_COUNTS("action count (local)"),
    STARLARK_PARSER("Starlark Parser", FIFTY_MILLIS),
    STARLARK_USER_FN("Starlark user function call", FIFTY_MILLIS),
    STARLARK_BUILTIN_FN("Starlark builtin function call", FIFTY_MILLIS),
    STARLARK_USER_COMPILED_FN("Starlark compiled user function call", FIFTY_MILLIS),
    STARLARK_REPOSITORY_FN("Starlark repository function call", FIFTY_MILLIS),
    STARLARK_THREAD_CONTEXT("Starlark thread context", FIFTY_MILLIS),
    ACTION_FS_STAGING("Staging per-action file system"),
    REMOTE_CACHE_CHECK("remote action cache check"),
    REMOTE_DOWNLOAD("remote output download", TEN_MILLIS),
    REMOTE_NETWORK("remote network"),
    FILESYSTEM_TRAVERSAL("filesystem traversal"),
    WORKER_EXECUTION("local execution in worker"),
    WORKER_SETUP("setting up inputs for worker"),
    WORKER_BORROW("borrowing a worker"),
    WORKER_WORKING("waiting for response from worker"),
    WORKER_COPYING_OUTPUTS("copying outputs from worker"),
    CREDENTIAL_HELPER("calling credential helper"),
    CONFLICT_CHECK("Conflict checking"),
    DYNAMIC_LOCK("Acquiring dynamic execution output lock", FIFTY_MILLIS),
    REPOSITORY_FETCH("Fetching repository"),
    REPOSITORY_VENDOR("Vendoring repository"),
    REPO_CACHE_GC_WAIT("blocked on repo contents cache GC", TEN_MILLIS),
    SPAWN_LOG("logging spawn", TEN_MILLIS),
    RPC("RPC"),
    SKYCACHE("Skycache"),
    WASM_LOAD("load WebAssembly module"),
    WASM_EXEC("execute WebAssembly function"),
    UNKNOWN("Unknown event"),
}

impl ProfilerTask {
    /// `ordinal()`.
    pub fn ordinal(self) -> usize {
        self as usize
    }

    /// True if the metric records VFS operations (`this.name().startsWith("VFS")`).
    // Java: com/google/devtools/build/lib/profiler/ProfilerTask.java:143 ProfilerTask.isVfs
    pub fn is_vfs(self) -> bool {
        self.name().starts_with("VFS")
    }

    /// `ProfilerTask.valueOf(name)`.
    pub fn value_of(name: &str) -> Option<ProfilerTask> {
        Self::VALUES.iter().copied().find(|t| t.name() == name)
    }
}

impl std::fmt::Display for ProfilerTask {
    /// `Enum.toString()`: the constant's name.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(self.name())
    }
}

/// A set of [`ProfilerTask`]s (`EnumSet<ProfilerTask>`), as a bit set.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct ProfilerTaskSet(u128);

impl ProfilerTaskSet {
    pub const fn empty() -> Self {
        ProfilerTaskSet(0)
    }

    pub fn all() -> Self {
        let mut s = Self::empty();
        for t in ProfilerTask::VALUES {
            s.add(*t);
        }
        s
    }

    pub fn add(&mut self, task: ProfilerTask) {
        self.0 |= 1u128 << task.ordinal();
    }

    pub fn contains(self, task: ProfilerTask) -> bool {
        self.0 & (1u128 << task.ordinal()) != 0
    }

    pub fn is_empty(self) -> bool {
        self.0 == 0
    }

    pub fn bits(self) -> u128 {
        self.0
    }

    pub fn from_bits(bits: u128) -> Self {
        ProfilerTaskSet(bits)
    }

    /// The tasks in ordinal order.
    pub fn iter(self) -> impl Iterator<Item = ProfilerTask> {
        ProfilerTask::VALUES
            .iter()
            .copied()
            .filter(move |t| self.contains(*t))
    }
}

impl FromIterator<ProfilerTask> for ProfilerTaskSet {
    fn from_iter<I: IntoIterator<Item = ProfilerTask>>(iter: I) -> Self {
        let mut s = Self::empty();
        for t in iter {
            s.add(t);
        }
        s
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fits_in_the_bit_set() {
        assert!(ProfilerTask::VALUES.len() <= 128);
        assert_eq!(
            ProfilerTask::UNKNOWN.ordinal(),
            ProfilerTask::VALUES.len() - 1
        );
    }

    #[test]
    fn thresholds_and_vfs() {
        assert_eq!(ProfilerTask::ACTION_CHECK.min_duration(), 10_000_000);
        assert_eq!(ProfilerTask::ACTION.min_duration(), -1);
        assert_eq!(ProfilerTask::VFS_GLOB.min_duration(), -1);
        assert!(ProfilerTask::VFS_GLOB.collects_slowest_instances());
        assert!(ProfilerTask::VFS_GLOB.is_vfs());
        assert!(!ProfilerTask::WAIT.is_vfs());
        assert!(!ProfilerTask::VFS_DELETE.collects_slowest_instances());
        assert_eq!(ProfilerTask::PHASE.description(), "build phase marker");
    }
}
