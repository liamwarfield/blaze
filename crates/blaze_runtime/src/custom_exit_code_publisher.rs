//! `lib/util/CustomExitCodePublisher` and `lib/util/CustomFailureDetailPublisher`: the files
//! through which a server that halts tells its client the exit code and the failure detail (the
//! client reads `<output_base>/exit_code_to_use_on_abrupt_exit` when the gRPC stream breaks,
//! `blaze.cc` `GetExitCodeForAbruptExit`).
//!
//! PORT: both classes belong to `lib/util` (`blaze_util`); they live here, next to their only
//! callers (`BlazeRuntime`, `BugReport`), until `blaze_util` may depend on `blaze_protos`.

use std::path::PathBuf;

use blaze_protos::JavaMessage;
use blaze_protos::failure_details::FailureDetail;
use blaze_util::string_encoding::internal_to_platform;
use parking_lot::Mutex;

/// Provides an external way for the Bazel server to communicate its exit code to the client,
/// when the main gRPC channel is unavailable because the exit is too abrupt or originated in an
/// async thread.
// Java: com/google/devtools/build/lib/util/CustomExitCodePublisher.java:29 CustomExitCodePublisher
pub struct CustomExitCodePublisher;

// Java: com/google/devtools/build/lib/util/CustomExitCodePublisher.java:32 CustomExitCodePublisher.EXIT_CODE_FILENAME
const EXIT_CODE_FILENAME: &str = "exit_code_to_use_on_abrupt_exit";

/// `abruptExitCodeFilePath` (a platform path).
static ABRUPT_EXIT_CODE_FILE_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

/// `Paths.get(path)` of an internal string.
fn platform_path(path: &str) -> PathBuf {
    PathBuf::from(internal_to_platform(path).into_owned())
}

impl CustomExitCodePublisher {
    // Java: com/google/devtools/build/lib/util/CustomExitCodePublisher.java:37 CustomExitCodePublisher.setAbruptExitStatusFileDir
    pub fn set_abrupt_exit_status_file_dir(path: &str) {
        *ABRUPT_EXIT_CODE_FILE_PATH.lock() = Some(platform_path(path).join(EXIT_CODE_FILENAME));
    }

    // Java: com/google/devtools/build/lib/util/CustomExitCodePublisher.java:41 CustomExitCodePublisher.maybeDeleteAbruptExitStatusFile
    pub fn maybe_delete_abrupt_exit_status_file() {
        let path = ABRUPT_EXIT_CODE_FILE_PATH.lock().clone();
        if let Some(path) = path {
            // Files.deleteIfExists.
            // PORT: skipped, logging ("Deleted old abrupt exit status file", "Failed to delete
            // old abrupt exit status file").
            let _ = std::fs::remove_file(&path);
        }
    }

    // Java: com/google/devtools/build/lib/util/CustomExitCodePublisher.java:55 CustomExitCodePublisher.resetAbruptExitStatusFile
    pub fn reset_abrupt_exit_status_file() {
        *ABRUPT_EXIT_CODE_FILE_PATH.lock() = None;
    }

    // Java: com/google/devtools/build/lib/util/CustomExitCodePublisher.java:59 CustomExitCodePublisher.maybeWriteExitStatusFile
    pub fn maybe_write_exit_status_file(exit_code: i32) -> bool {
        let path = ABRUPT_EXIT_CODE_FILE_PATH.lock().clone();
        if let Some(path) = path {
            match std::fs::write(&path, exit_code.to_string().as_bytes()) {
                Ok(()) => return true,
                Err(ioe) => {
                    // System.err.printf("io error writing %d to abrupt exit status file %s: %s\n")
                    let message = format!(
                        "io error writing {} to abrupt exit status file {}: {}\n",
                        exit_code,
                        blaze_util::string_encoding::platform_to_internal(path.as_os_str()),
                        ioe
                    );
                    let _ = blaze_events::OutErr::system_out_err()
                        .get_error_stream()
                        .write(&blaze_util::string_encoding::latin1_encode_lossy(&message));
                }
            }
        }
        false
    }
}

/// Provides an external way for the Bazel server to communicate its `FailureDetail` to the
/// client, when the main gRPC channel is unavailable because the exit is too abrupt or
/// originated in an async thread.
// Java: com/google/devtools/build/lib/util/CustomFailureDetailPublisher.java:28 CustomFailureDetailPublisher
pub struct CustomFailureDetailPublisher;

/// `failureDetailFilePath` (a platform path).
static FAILURE_DETAIL_FILE_PATH: Mutex<Option<PathBuf>> = Mutex::new(None);

impl CustomFailureDetailPublisher {
    // Java: com/google/devtools/build/lib/util/CustomFailureDetailPublisher.java:34 CustomFailureDetailPublisher.setFailureDetailFilePath
    pub fn set_failure_detail_file_path(path: &str) {
        *FAILURE_DETAIL_FILE_PATH.lock() = Some(platform_path(path));
    }

    // Java: com/google/devtools/build/lib/util/CustomFailureDetailPublisher.java:38 CustomFailureDetailPublisher.resetFailureDetailFilePath
    pub fn reset_failure_detail_file_path() {
        *FAILURE_DETAIL_FILE_PATH.lock() = None;
    }

    // Java: com/google/devtools/build/lib/util/CustomFailureDetailPublisher.java:43 CustomFailureDetailPublisher.maybeWriteFailureDetailFile
    pub fn maybe_write_failure_detail_file(failure_detail: &FailureDetail) -> bool {
        let path = FAILURE_DETAIL_FILE_PATH.lock().clone();
        if let Some(path) = path {
            match std::fs::write(&path, failure_detail.to_byte_array()) {
                Ok(()) => return true,
                Err(ioe) => {
                    // PORT: Java prints the failure detail with its `toString()` (the proto
                    // text format); Rust's `Debug` stands for it.
                    let message = format!(
                        "io error writing failure detail to file {}.\nfailure_detail: {:?}\nIOException: {}",
                        blaze_util::string_encoding::platform_to_internal(path.as_os_str()),
                        failure_detail,
                        ioe
                    );
                    let _ = blaze_events::OutErr::system_out_err()
                        .get_error_stream()
                        .write(&blaze_util::string_encoding::latin1_encode_lossy(&message));
                }
            }
        }
        false
    }
}

/// The unit tests of this crate that set process-wide crash state (these paths, `BugReport`'s
/// runtime) run one at a time.
#[cfg(test)]
pub(crate) static TEST_SERIAL: Mutex<()> = Mutex::new(());

#[cfg(test)]
mod tests {
    use super::*;

    use super::TEST_SERIAL as SERIAL;

    #[test]
    fn exit_status_file_is_written_and_deleted() {
        let _serial = SERIAL.lock();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join(EXIT_CODE_FILENAME);
        CustomExitCodePublisher::reset_abrupt_exit_status_file();
        assert!(!CustomExitCodePublisher::maybe_write_exit_status_file(37));
        CustomExitCodePublisher::set_abrupt_exit_status_file_dir(dir.path().to_str().unwrap());
        assert!(CustomExitCodePublisher::maybe_write_exit_status_file(37));
        assert_eq!(std::fs::read_to_string(&file).unwrap(), "37");
        CustomExitCodePublisher::maybe_delete_abrupt_exit_status_file();
        assert!(!file.exists());
        CustomExitCodePublisher::reset_abrupt_exit_status_file();
    }

    #[test]
    fn failure_detail_file_is_written() {
        let _serial = SERIAL.lock();
        let dir = tempfile::tempdir().unwrap();
        let file = dir.path().join("failure_detail.rawproto");
        let detail = FailureDetail {
            message: "Crashed: (panic) x".to_string(),
            category: None,
        };
        CustomFailureDetailPublisher::reset_failure_detail_file_path();
        assert!(!CustomFailureDetailPublisher::maybe_write_failure_detail_file(&detail));
        CustomFailureDetailPublisher::set_failure_detail_file_path(file.to_str().unwrap());
        assert!(CustomFailureDetailPublisher::maybe_write_failure_detail_file(&detail));
        assert_eq!(std::fs::read(&file).unwrap(), detail.to_byte_array());
        CustomFailureDetailPublisher::reset_failure_detail_file_path();
    }
}
