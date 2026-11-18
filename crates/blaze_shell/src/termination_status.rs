//! Port of `com.google.devtools.build.lib.shell.TerminationStatus`.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::time::Duration;

/// Represents the termination status of a command. `Process.waitFor` is not very precisely
/// specified, so this class encapsulates the interpretation of values returned by it.
///
/// Caveat: due to the lossy encoding, it's not always possible to accurately distinguish signal
/// and exit cases. In particular, processes that exit with a value within the interval [129, 191]
/// will be mistaken for having been terminated by a signal.
///
/// Instances are immutable. Equality and hashing use only the wait result.
// Java: com/google/devtools/build/lib/shell/TerminationStatus.java:34 TerminationStatus
#[derive(Clone, Debug)]
pub struct TerminationStatus {
    wait_result: i32,
    timed_out: bool,
    wall_execution_time: Option<Duration>,
    user_execution_time: Option<Duration>,
    system_execution_time: Option<Duration>,
}

/// Values taken from the glibc strsignal(3) function.
// Java: com/google/devtools/build/lib/shell/TerminationStatus.java:45 TerminationStatus.SIGNAL_STRINGS
const SIGNAL_STRINGS: [Option<&str>; 32] = [
    None,
    Some("Hangup"),
    Some("Interrupt"),
    Some("Quit"),
    Some("Illegal instruction"),
    Some("Trace/breakpoint trap"),
    Some("Aborted"),
    Some("Bus error"),
    Some("Floating point exception"),
    Some("Killed"),
    Some("User defined signal 1"),
    Some("Segmentation fault"),
    Some("User defined signal 2"),
    Some("Broken pipe"),
    Some("Alarm clock"),
    Some("Terminated"),
    Some("Stack fault"),
    Some("Child exited"),
    Some("Continued"),
    Some("Stopped (signal)"),
    Some("Stopped"),
    Some("Stopped (tty input)"),
    Some("Stopped (tty output)"),
    Some("Urgent I/O condition"),
    Some("CPU time limit exceeded"),
    Some("File size limit exceeded"),
    Some("Virtual timer expired"),
    Some("Profiling timer expired"),
    Some("Window changed"),
    Some("I/O possible"),
    Some("Power failure"),
    Some("Bad system call"),
];

// Java: com/google/devtools/build/lib/shell/TerminationStatus.java:80 TerminationStatus.getSignalString
fn get_signal_string(signum: i32) -> String {
    if signum > 0 && (signum as usize) < SIGNAL_STRINGS.len() {
        SIGNAL_STRINGS[signum as usize].unwrap().to_string()
    } else {
        format!("Signal {signum}")
    }
}

impl TerminationStatus {
    // We're relying on undocumented behaviour of Process.waitFor, specifically that waitResult is
    // the exit status when the process returns normally, or 128+signalnumber when the process is
    // terminated by a signal. We further assume that value signal numbers fall in the interval
    // [1, 63].
    pub const SIGNAL_1: i32 = 128 + 1;
    pub const SIGNAL_63: i32 = 128 + 63;
    pub const SIGNAL_SIGABRT: i32 = 128 + 6;
    pub const SIGNAL_SIGKILL: i32 = 128 + 9;
    pub const SIGNAL_SIGBUS: i32 = 128 + 10;
    pub const SIGNAL_SIGTERM: i32 = 128 + 15;

    /// Construct a TerminationStatus instance from a Process waitFor code.
    // Java: com/google/devtools/build/lib/shell/TerminationStatus.java:92 TerminationStatus.<init>
    pub fn new(wait_result: i32, timed_out: bool) -> TerminationStatus {
        TerminationStatus::with_times(wait_result, timed_out, None, None, None)
    }

    /// Construct a TerminationStatus instance from a Process waitFor code, with execution times.
    // Java: com/google/devtools/build/lib/shell/TerminationStatus.java:107 TerminationStatus.<init>
    pub fn with_times(
        wait_result: i32,
        timed_out: bool,
        wall_execution_time: Option<Duration>,
        user_execution_time: Option<Duration>,
        system_execution_time: Option<Duration>,
    ) -> TerminationStatus {
        TerminationStatus {
            wait_result,
            timed_out,
            wall_execution_time,
            user_execution_time,
            system_execution_time,
        }
    }

    /// Returns the exit code returned by the subprocess.
    // Java: com/google/devtools/build/lib/shell/TerminationStatus.java:123 TerminationStatus.getRawExitCode
    pub fn get_raw_exit_code(&self) -> i32 {
        self.wait_result
    }

    /// Returns true iff the process exited with code 0.
    // Java: com/google/devtools/build/lib/shell/TerminationStatus.java:130 TerminationStatus.success
    pub fn success(&self) -> bool {
        self.exited() && self.get_exit_code() == 0
    }

    /// Returns true if the given exit code represents a crash.
    // Java: com/google/devtools/build/lib/shell/TerminationStatus.java:153 TerminationStatus.crashed
    pub fn crashed(raw_status: i32) -> bool {
        raw_status == Self::SIGNAL_SIGABRT || raw_status == Self::SIGNAL_SIGBUS
    }

    /// Returns true iff the process exited normally.
    // Java: com/google/devtools/build/lib/shell/TerminationStatus.java:158 TerminationStatus.exited
    pub fn exited(&self) -> bool {
        !self.timed_out && (self.wait_result < Self::SIGNAL_1 || self.wait_result > Self::SIGNAL_63)
    }

    /// Returns true if the process timed out.
    // Java: com/google/devtools/build/lib/shell/TerminationStatus.java:163 TerminationStatus.timedOut
    pub fn timed_out(&self) -> bool {
        self.timed_out
    }

    /// Returns the exit code of the subprocess. Undefined if exited() is false.
    // Java: com/google/devtools/build/lib/shell/TerminationStatus.java:170 TerminationStatus.getExitCode
    pub fn get_exit_code(&self) -> i32 {
        if !self.exited() {
            panic!("IllegalStateException: getExitCode() not defined");
        }
        self.wait_result
    }

    /// Returns the number of the signal that terminated the process. Undefined if exited()
    /// returns true.
    // Java: com/google/devtools/build/lib/shell/TerminationStatus.java:181 TerminationStatus.getTerminatingSignal
    pub fn get_terminating_signal(&self) -> i32 {
        if self.exited() || self.timed_out {
            panic!("IllegalStateException: getTerminatingSignal() not defined");
        }
        self.wait_result - Self::SIGNAL_1 + 1
    }

    // Java: com/google/devtools/build/lib/shell/TerminationStatus.java:194 TerminationStatus.getWallExecutionTime
    pub fn get_wall_execution_time(&self) -> Option<Duration> {
        self.wall_execution_time
    }

    // Java: com/google/devtools/build/lib/shell/TerminationStatus.java:204 TerminationStatus.getUserExecutionTime
    pub fn get_user_execution_time(&self) -> Option<Duration> {
        self.user_execution_time
    }

    // Java: com/google/devtools/build/lib/shell/TerminationStatus.java:214 TerminationStatus.getSystemExecutionTime
    pub fn get_system_execution_time(&self) -> Option<Duration> {
        self.system_execution_time
    }

    /// Returns a short string describing the termination status. e.g. "Exit 1" or "Hangup".
    // Java: com/google/devtools/build/lib/shell/TerminationStatus.java:222 TerminationStatus.toShortString
    pub fn to_short_string(&self) -> String {
        if self.exited() {
            format!("Exit {}", self.get_exit_code())
        } else if self.timed_out {
            "Timeout".to_string()
        } else {
            get_signal_string(self.get_terminating_signal())
        }
    }

    /// Returns a new builder.
    // Java: com/google/devtools/build/lib/shell/TerminationStatus.java:251 TerminationStatus.builder
    pub fn builder() -> TerminationStatusBuilder {
        TerminationStatusBuilder::default()
    }
}

impl fmt::Display for TerminationStatus {
    // Java: com/google/devtools/build/lib/shell/TerminationStatus.java:229 TerminationStatus.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.exited() {
            write!(f, "Process exited with status {}", self.get_exit_code())
        } else if self.timed_out {
            f.write_str("Timed out")
        } else {
            write!(
                f,
                "Process terminated by signal {}",
                self.get_terminating_signal()
            )
        }
    }
}

impl PartialEq for TerminationStatus {
    // Java: com/google/devtools/build/lib/shell/TerminationStatus.java:245 TerminationStatus.equals
    fn eq(&self, other: &Self) -> bool {
        self.wait_result == other.wait_result
    }
}

impl Eq for TerminationStatus {}

impl Hash for TerminationStatus {
    // Java: com/google/devtools/build/lib/shell/TerminationStatus.java:240 TerminationStatus.hashCode
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.wait_result.hash(state);
    }
}

/// Builder for [`TerminationStatus`] objects.
// Java: com/google/devtools/build/lib/shell/TerminationStatus.java:256 TerminationStatus.Builder
#[derive(Clone, Debug, Default)]
pub struct TerminationStatusBuilder {
    wait_response: Option<i32>,
    timed_out: Option<bool>,
    wall_execution_time: Option<Duration>,
    user_execution_time: Option<Duration>,
    system_execution_time: Option<Duration>,
}

impl TerminationStatusBuilder {
    /// Sets the value returned by `Process.waitFor`.
    pub fn set_wait_response(&mut self, wait_response: i32) -> &mut Self {
        self.wait_response = Some(wait_response);
        self
    }

    /// Sets whether the action timed out or not.
    pub fn set_timed_out(&mut self, timed_out: bool) -> &mut Self {
        self.timed_out = Some(timed_out);
        self
    }

    pub fn set_wall_execution_time(&mut self, d: Duration) -> &mut Self {
        self.wall_execution_time = Some(d);
        self
    }

    pub fn set_user_execution_time(&mut self, d: Duration) -> &mut Self {
        self.user_execution_time = Some(d);
        self
    }

    pub fn set_system_execution_time(&mut self, d: Duration) -> &mut Self {
        self.system_execution_time = Some(d);
        self
    }

    /// Builds a [`TerminationStatus`] object.
    // Java: com/google/devtools/build/lib/shell/TerminationStatus.java:304 TerminationStatus.Builder.build
    pub fn build(&self) -> TerminationStatus {
        let Some(wait_response) = self.wait_response else {
            panic!("IllegalStateException: waitResponse never set");
        };
        let Some(timed_out) = self.timed_out else {
            panic!("IllegalStateException: timedOut never set");
        };
        TerminationStatus::with_times(
            wait_response,
            timed_out,
            self.wall_execution_time,
            self.user_execution_time,
            self.system_execution_time,
        )
    }
}
