//! Port of `src/main/cpp/util/exit_code.h`: exit codes for Blaze. Must be kept in sync with
//! the Java counterpart under com/google/devtools/build/lib/util/ExitCode.java.

// Java: src/main/cpp/util/exit_code.h:22 blaze_exit_code::ExitCode

/// Success.
pub const SUCCESS: i32 = 0;
/// Command Line Problem, Bad or Illegal flags or command combination, or Bad environment
/// variables. The user must modify their command line.
pub const BAD_ARGV: i32 = 2;
/// The user interrupted the build, most probably with Ctrl-C.
pub const INTERRUPTED: i32 = 8;
/// The client or server lock is held, and --noblock_for_lock was passed, so this command fails
/// fast.
pub const LOCK_HELD_NOBLOCK_FOR_LOCK: i32 = 9;
/// The server process ran out of memory.
pub const OOM_ERROR: i32 = 33;
/// Something is wrong with the host Bazel is running on and a re-run of the same command
/// probably will not help.
pub const LOCAL_ENVIRONMENTAL_ERROR: i32 = 36;
/// Unexpected server termination, due to e.g. external SIGKILL, misplaced System.exit(), or a
/// JVM crash. This exit code should be a last resort.
pub const INTERNAL_ERROR: i32 = 37;

/// Converts the rc-processing exit code of `blaze_options` into the client's integer code.
pub fn from_options_exit_code(code: blaze_options::rc::option_processor::ExitCode) -> i32 {
    code as i32
}
