//! Port of `src/main/cpp/archive_utils.{h,cc}`, adapted to Blaze's embedded install archive.
//!
//! Bazel appends a zip to its binary: the install base files (`A-server.jar` first), then
//! `install_base_key` last; `DetermineArchiveContents` lists the members up to and including
//! `install_base_key`, and `ExtractBuildLabel` reads `build-label.txt`.
//!
//! Blaze embeds a zstd-compressed tar instead ([`EmbeddedArchive`], built by
//! `crates/blaze/build.rs`) with the same members in the same order, except the server jar: Blaze
//! is its own server, so the archive does not carry `A-server.jar`, and [`get_server_jar_path`]
//! returns the jar name the oracle archive had first.

use std::borrow::Cow;
use std::collections::BTreeSet;
use std::io::Read;

use crate::platform::{DurationMillis, get_milliseconds_monotonic};
use crate::startup_options::StartupOptions;
use crate::util::errors::get_last_error_string;
use crate::util::exit_code;
use crate::util::file::{
    K_RENAME_DIRECTORY_FAILURE_NOT_EMPTY, K_RENAME_DIRECTORY_SUCCESS, create_sibling_temp_dir,
    get_all_files_under, is_directory_path, is_untampered, make_directories_path, path_exists_path,
    read_file_path, remove_recursively, rename_directory, set_mtime_to_distant_future,
    sync_file_path, write_file_path,
};
use crate::util::logging::{LogLevel, bazel_die, bazel_log};
use crate::util::path::{Path, is_root_directory_path};
use crate::util::strings::from_platform_bytes;
use blaze_util::string_encoding::latin1_encode;

const ARCHIVE_UTILS_CC: &str = "src/main/cpp/archive_utils.cc";

/// The install archive embedded in the Blaze binary (Bazel: the zip appended to the client).
#[derive(Clone, Debug)]
pub struct EmbeddedArchive {
    /// A zstd-compressed tar: the install base files (no directory entries) in archive order,
    /// `install_base_key` last.
    pub data: Cow<'static, [u8]>,
    /// The member names in archive order, up to and including `install_base_key` (what
    /// `DetermineArchiveContents` returns).
    pub contents: Vec<String>,
    /// The contents of the `install_base_key` member.
    pub install_md5: String,
    /// The contents of the `build-label.txt` member.
    pub build_label: String,
    /// The server jar's path relative to the install base (the first member of Bazel's zip, which
    /// Blaze does not embed).
    pub server_jar: String,
}

impl EmbeddedArchive {
    pub fn new(
        data: impl Into<Cow<'static, [u8]>>,
        contents: Vec<String>,
        install_md5: impl Into<String>,
        build_label: impl Into<String>,
        server_jar: impl Into<String>,
    ) -> Self {
        EmbeddedArchive {
            data: data.into(),
            contents,
            install_md5: install_md5.into(),
            build_label: build_label.into(),
            server_jar: server_jar.into(),
        }
    }
}

// Java: src/main/cpp/archive_utils.h:33 RestartReason
/// The reason for a blaze server restart. Keep in sync with logging.proto.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum RestartReason {
    NoRestart = 0,
    NoDaemon,
    NewVersion,
    NewOptions,
    PidFileButNoServer,
    ServerVanished,
    ServerUnresponsive,
}

// Java: src/main/cpp/archive_utils.h:45 LoggingInfo
/// Miscellaneous information reported to the server for logging and profiling purposes.
#[derive(Clone, Debug)]
pub struct LoggingInfo {
    /// Path of this binary.
    pub binary_path: String,
    /// The time in ms the binary started up, measured from approximately the time that "main"
    /// was called.
    pub start_time_ms: u64,
    /// The reason the server was restarted.
    pub restart_reason: RestartReason,
}

impl LoggingInfo {
    pub fn new(binary_path: impl Into<String>, start_time_ms: u64) -> Self {
        LoggingInfo {
            binary_path: binary_path.into(),
            start_time_ms,
            restart_reason: RestartReason::NoRestart,
        }
    }

    // Java: src/main/cpp/archive_utils.h:52 LoggingInfo::SetRestartReasonIfNotSet
    pub fn set_restart_reason_if_not_set(&mut self, restart_reason: RestartReason) {
        if self.restart_reason == RestartReason::NoRestart {
            self.restart_reason = restart_reason;
        }
    }
}

/// One regular-file member of the archive.
struct Member {
    name: String,
    data: Vec<u8>,
}

fn read_error(archive_path: &str, e: std::io::Error) -> ! {
    bazel_die(
        exit_code::LOCAL_ENVIRONMENTAL_ERROR,
        ARCHIVE_UTILS_CC,
        71,
        format!("Error reading zip file '{}': {}", archive_path, e),
    )
}

// Java: src/main/cpp/archive_utils.cc:41 PartialZipExtractor::UnzipUntil
/// Scans the archive until a member named `stop_entry` is seen, calling `callback` with each
/// member up to and including it. Returns the contents of `stop_entry`.
fn unzip_until(
    archive: &EmbeddedArchive,
    archive_path: &str,
    stop_entry: &str,
    callback: &mut dyn FnMut(&Member),
) -> Vec<u8> {
    let decoder = match zstd::stream::read::Decoder::new(&archive.data[..]) {
        Ok(d) => d,
        Err(e) => bazel_die(
            exit_code::LOCAL_ENVIRONMENTAL_ERROR,
            ARCHIVE_UTILS_CC,
            59,
            format!("Failed to open '{}' as a zip file: {}", archive_path, e),
        ),
    };
    let mut tar = tar::Archive::new(decoder);
    let entries = match tar.entries() {
        Ok(e) => e,
        Err(e) => bazel_die(
            exit_code::LOCAL_ENVIRONMENTAL_ERROR,
            ARCHIVE_UTILS_CC,
            71,
            format!("Error reading zip file '{}': {}", archive_path, e),
        ),
    };
    for entry in entries {
        let mut entry = match entry {
            Ok(e) => e,
            Err(e) => read_error(archive_path, e),
        };
        let entry_type = entry.header().entry_type();
        // The C++ zip extractor keeps member names as raw bytes.
        let name = crate::util::strings::from_platform_bytes(&entry.path_bytes());
        // Accept(): skip directories.
        if entry_type.is_dir() || name.is_empty() || name.ends_with('/') {
            continue;
        }
        if !entry_type.is_file() {
            ::blaze_util::unported!(
                "PartialZipExtractor::Accept",
                "non-regular archive member '{}' ({:?})",
                name,
                entry_type
            );
        }
        let mut data = Vec::new();
        if let Err(e) = entry.read_to_end(&mut data) {
            read_error(archive_path, e);
        }
        let member = Member { name, data };
        callback(&member);
        if member.name == stop_entry {
            return member.data;
        }
    }
    bazel_die(
        exit_code::LOCAL_ENVIRONMENTAL_ERROR,
        ARCHIVE_UTILS_CC,
        75,
        format!(
            "Failed to find member '{}' in zip file '{}'",
            stop_entry, archive_path
        ),
    )
}

// Java: src/main/cpp/archive_utils.cc:115 ExtractData
/// Installs Blaze by extracting the embedded data files, iff necessary. The MD5-named
/// install_base directory on disk is trusted; we assume no-one has modified the extracted files
/// beneath this directory once it is in place. Concurrency during extraction is handled by
/// extracting in a tmp dir and then renaming it into place where it becomes visible atomically
/// at the new path.
pub fn extract_data(
    self_path: &str,
    archive: &EmbeddedArchive,
    archive_contents: &[String],
    expected_install_md5: &str,
    startup_options: &StartupOptions,
    _logging_info: &mut LoggingInfo,
) -> Option<DurationMillis> {
    let install_base = &startup_options.install_base;

    if path_exists_path(install_base) {
        // This would be detected implicitly below, but checking explicitly lets us give a
        // better error message.
        if !is_directory_path(install_base) {
            bazel_log(
                LogLevel::User,
                ARCHIVE_UTILS_CC,
                126,
                format!(
                    "Install base ('{}') exists but is not a directory; removing.",
                    install_base.as_printable_path()
                ),
            );
        } else {
            let install_dir = install_base.clone();
            let mut all_files_ok = true;
            // Check that all files are present and have timestamps from BlessFiles().
            for it in archive_contents {
                let path = install_dir.get_relative(it);
                if !is_untampered(&path) {
                    bazel_log(
                        LogLevel::User,
                        ARCHIVE_UTILS_CC,
                        135,
                        format!(
                            "Corrupt installation: file '{}' is missing or modified (this is \
                             expected if the installation is too old).",
                            path.as_printable_path()
                        ),
                    );
                    all_files_ok = false;
                    break;
                }
            }

            if all_files_ok {
                // Also check that the installed files claim to match this binary. We check this
                // afterward because the above diagnostic is better for a missing
                // install_base_key file.
                let key_path = install_dir.get_relative("install_base_key");
                match read_file_path(&key_path, 0) {
                    None => {
                        bazel_log(
                            LogLevel::User,
                            ARCHIVE_UTILS_CC,
                            152,
                            format!("Cannot read '{}'.", key_path.as_printable_path()),
                        );
                    }
                    Some(on_disk_key) if on_disk_key != *latin1_encode(expected_install_md5) => {
                        // The C++ prints the on-disk key bytes unchanged.
                        bazel_log(
                            LogLevel::User,
                            ARCHIVE_UTILS_CC,
                            155,
                            format!(
                                "The install_base directory '{}' contains a different {} \
                                 version (found {} but this binary is {}).",
                                install_base.as_printable_path(),
                                startup_options.product_name,
                                from_platform_bytes(&on_disk_key),
                                expected_install_md5
                            ),
                        );
                    }
                    Some(_) => {
                        // All good.
                        return None;
                    }
                }
            }
        }
        // If we get here, the installation is bad. Remove it.
        bazel_log(
            LogLevel::User,
            ARCHIVE_UTILS_CC,
            168,
            format!(
                "Removing previous installation at '{}' because it is corrupted or matches a \
                 different version",
                install_base.as_printable_path()
            ),
        );
        if !remove_recursively(install_base) {
            bazel_die(
                exit_code::LOCAL_ENVIRONMENTAL_ERROR,
                ARCHIVE_UTILS_CC,
                172,
                format!(
                    "Failed to remove previous installation at '{}': {}",
                    install_base.as_printable_path(),
                    get_last_error_string()
                ),
            );
        }
    }

    // If we get here, the installation directory does not exist or was removed. Time to
    // install.
    let st = get_milliseconds_monotonic();
    // Work in a sibling temporary directory to avoid races.
    let tmp_install = create_sibling_temp_dir(install_base);
    extract_archive_or_die(
        self_path,
        archive,
        &startup_options.product_name,
        expected_install_md5,
        &tmp_install,
    );
    bless_files(&tmp_install);

    let et = get_milliseconds_monotonic();
    let extract_data_duration = DurationMillis::new(st, et);

    // Now rename the completed installation to its final name.
    let mut attempts = 0;
    while attempts < 120 {
        let result = rename_directory(&tmp_install, install_base);
        if result == K_RENAME_DIRECTORY_SUCCESS || result == K_RENAME_DIRECTORY_FAILURE_NOT_EMPTY {
            // If renaming fails because the directory already exists and is not empty, then we
            // assume another good installation snuck in before us.
            remove_recursively(&tmp_install);
            break;
        } else {
            // Otherwise the install directory may still be scanned by the antivirus (in case
            // we're running on Windows) so we need to wait for that to finish and try renaming
            // again.
            attempts += 1;
            bazel_log(
                LogLevel::User,
                ARCHIVE_UTILS_CC,
                207,
                format!(
                    "install base directory '{}' could not be renamed into place after {} \
                     second(s), trying again\r",
                    tmp_install.as_printable_path(),
                    attempts
                ),
            );
            std::thread::sleep(std::time::Duration::from_secs(1));
        }
    }

    // Give up renaming after 120 failed attempts / 2 minutes.
    if attempts == 120 {
        remove_recursively(&tmp_install);
        bazel_die(
            exit_code::LOCAL_ENVIRONMENTAL_ERROR,
            ARCHIVE_UTILS_CC,
            218,
            format!(
                "install base directory '{}' could not be renamed into place: {}",
                tmp_install.as_printable_path(),
                get_last_error_string()
            ),
        );
    }
    Some(extract_data_duration)
}

// Java: src/main/cpp/archive_utils.cc:225 DetermineArchiveContents
/// Returns the names of the archive members (up to and including `install_base_key`) and the
/// install md5 key.
pub fn determine_archive_contents(archive: &EmbeddedArchive) -> (Vec<String>, String) {
    // PORT: the member list and key are generated at build time (crates/blaze/build.rs) instead
    // of scanning the archive on every run.
    (archive.contents.clone(), archive.install_md5.clone())
}

// Java: src/main/cpp/blaze_util_posix.cc:85 PosixDumper::Dump
/// The PosixDumper: writes each member (mode 0755), creating parent directories.
struct PosixDumper {
    dir_cache: BTreeSet<Path>,
    error_msg: String,
    was_error: bool,
}

impl PosixDumper {
    fn dump(&mut self, data: &[u8], path: &Path) {
        if self.was_error {
            return;
        }
        let parent = path.get_parent();
        // Performance optimization: memoize the paths we already created a directory for, to
        // spare a stat in attempting to recreate an already existing directory.
        if self.dir_cache.insert(parent.clone()) && !make_directories_path(&parent, 0o777) {
            self.was_error = true;
            let msg = get_last_error_string();
            self.error_msg = format!("couldn't create '{}': {}", path.as_printable_path(), msg);
        }
        if self.was_error {
            return;
        }
        if !write_file_path(data, path, 0o755) {
            self.was_error = true;
            let msg = get_last_error_string();
            self.error_msg = format!(
                "Failed to write zipped file '{}': {}",
                path.as_printable_path(),
                msg
            );
        }
    }

    // Java: src/main/cpp/blaze_util_posix.cc:118 PosixDumper::Finish
    fn finish(&self) -> Result<(), String> {
        if self.was_error {
            Err(self.error_msg.clone())
        } else {
            Ok(())
        }
    }
}

// Java: src/main/cpp/archive_utils.cc:231 ExtractArchiveOrDie
/// Extracts the embedded data files into `output_dir`, which must exist. Fails if
/// `expected_install_md5` doesn't match the archive's `install_base_key`.
pub fn extract_archive_or_die(
    archive_path: &str,
    archive: &EmbeddedArchive,
    product_name: &str,
    expected_install_md5: &str,
    output_dir: &Path,
) {
    let mut dumper = PosixDumper {
        dir_cache: BTreeSet::new(),
        error_msg: String::new(),
        was_error: false,
    };

    if !path_exists_path(output_dir) {
        bazel_die(
            exit_code::INTERNAL_ERROR,
            ARCHIVE_UTILS_CC,
            243,
            format!(
                "Archive output directory didn't exist: {}",
                output_dir.as_printable_path()
            ),
        );
    }

    bazel_log(
        LogLevel::User,
        ARCHIVE_UTILS_CC,
        248,
        format!("Extracting {} installation...", product_name),
    );

    let install_md5 = unzip_until(archive, archive_path, "install_base_key", &mut |m| {
        dumper.dump(&m.data, &output_dir.get_relative(&m.name));
    });

    if let Err(error) = dumper.finish() {
        bazel_die(
            exit_code::LOCAL_ENVIRONMENTAL_ERROR,
            ARCHIVE_UTILS_CC,
            258,
            format!("Failed to extract embedded binaries: {}", error),
        );
    }

    if install_md5 != *latin1_encode(expected_install_md5) {
        // The C++ prints the extracted install_base_key bytes unchanged.
        bazel_die(
            exit_code::LOCAL_ENVIRONMENTAL_ERROR,
            ARCHIVE_UTILS_CC,
            263,
            format!(
                "The {} binary at {} was replaced during the client's self-extraction (old md5: \
                 {} new md5: {}). If you expected this then you should simply re-run {} in \
                 order to pick up the different version. If you didn't expect this then you \
                 should investigate what happened.",
                product_name,
                archive_path,
                expected_install_md5,
                from_platform_bytes(&install_md5),
                product_name
            ),
        );
    }
}

// Java: src/main/cpp/archive_utils.cc:274 BlessFiles
/// Sets the timestamps of the extracted files to the future and makes sure (or at least as sure
/// as we can...) that the files we have written are actually on the disk.
pub fn bless_files(embedded_binaries: &Path) {
    let mut extracted_files = Vec::new();

    // Walks the temporary directory recursively and collects full file paths.
    get_all_files_under(embedded_binaries, &mut extracted_files);

    let mut synced_directories: BTreeSet<Path> = BTreeSet::new();
    for file in &extracted_files {
        // Set the time to a distantly futuristic value so we can observe tampering. Note that
        // keeping a static, deterministic timestamp, such as the default timestamp set by unzip
        // (1970-01-01) and using that to detect tampering is not enough, because we also need
        // the timestamp to change between Bazel releases so that the metadata cache knows that
        // the files may have changed. This is essential for the correctness of actions that use
        // embedded binaries as artifacts.
        if !set_mtime_to_distant_future(file) {
            let err = get_last_error_string();
            bazel_die(
                exit_code::LOCAL_ENVIRONMENTAL_ERROR,
                ARCHIVE_UTILS_CC,
                294,
                format!(
                    "failed to set timestamp on '{}': {}",
                    file.as_printable_path(),
                    err
                ),
            );
        }

        sync_file_path(file);

        let mut directory = file.get_parent();

        // Now walk up until embedded_binaries and sync every directory in between.
        // synced_directories is used to avoid syncing the same directory twice.
        while directory != *embedded_binaries
            && !directory.is_empty()
            && !is_root_directory_path(&directory)
            && synced_directories.insert(directory.clone())
        {
            sync_file_path(&directory);
            directory = directory.get_parent();
        }
    }

    sync_file_path(embedded_binaries);
}

// Java: src/main/cpp/archive_utils.cc:319 ExtractBuildLabel
/// Retrieves the build label (version string) from the archive.
pub fn extract_build_label(archive: &EmbeddedArchive) -> String {
    // PORT: build-label.txt's contents are generated at build time (crates/blaze/build.rs)
    // instead of scanning the archive on every run.
    archive.build_label.clone()
}

// Java: src/main/cpp/archive_utils.cc:324 GetServerJarPath
/// Returns the server jar path from the archive contents.
pub fn get_server_jar_path(archive: &EmbeddedArchive) -> String {
    // PORT: Bazel returns archive_contents[0] (A-server.jar). Blaze does not embed the jar, so
    // the archive records its name separately.
    if archive.server_jar.is_empty() {
        bazel_die(
            exit_code::LOCAL_ENVIRONMENTAL_ERROR,
            ARCHIVE_UTILS_CC,
            327,
            "Couldn't find server jar in archive",
        );
    }
    archive.server_jar.clone()
}
