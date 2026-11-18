//! Port of src/test/cpp/blaze_archive_test.cc, on Blaze's embedded archive (a zstd-compressed
//! tar instead of a zip appended to the binary).

use blaze_client::archive_utils::{EmbeddedArchive, LoggingInfo, extract_data};
use blaze_client::platform::get_milliseconds_monotonic;
use blaze_client::startup_options::StartupOptions;
use blaze_client::util::file::is_untampered;
use blaze_client::util::path::{Path, join_path};
use blaze_options::rc::option_processor::RcStartupFlag;

// Java: src/test/cpp/blaze_archive_test.cc:46 get_archive_path_to_contents
fn get_archive_path_to_contents(expected_install_md5: &str) -> Vec<(String, Vec<u8>)> {
    vec![
        ("foo".into(), b"foo content".to_vec()),
        ("bar".into(), b"bar content".to_vec()),
        ("path/to/subdir/baz".into(), b"baz content".to_vec()),
        (
            "install_base_key".into(),
            expected_install_md5.as_bytes().to_vec(),
        ),
    ]
}

// Java: src/test/cpp/blaze_archive_test.cc:58 get_archive_paths
fn get_archive_paths() -> Vec<String> {
    vec!["foo".into(), "bar".into(), "path/to/subdir/baz".into()]
}

// Java: src/test/cpp/blaze_archive_test.cc:66 MakeZipAndReturnInstallBase
fn make_archive(contents: &[(String, Vec<u8>)]) -> Vec<u8> {
    let encoder = zstd::stream::write::Encoder::new(Vec::new(), 3).unwrap();
    let mut builder = tar::Builder::new(encoder);
    for (path, data) in contents {
        let mut header = tar::Header::new_gnu();
        header.set_entry_type(tar::EntryType::Regular);
        header.set_size(data.len() as u64);
        header.set_mode(0o644);
        builder.append_data(&mut header, path, &data[..]).unwrap();
    }
    builder.into_inner().unwrap().finish().unwrap()
}

// Java: src/test/cpp/blaze_archive_test.cc:79 set_startup_options
fn set_startup_options(startup_options: &mut StartupOptions, output_dir: &str) {
    let flags = vec![RcStartupFlag {
        source: "somewhere".into(),
        value: format!("--install_base={}", output_dir),
    }];
    assert_eq!(Ok(()), startup_options.process_args_impl(&flags));
}

struct Fixture {
    _temp: tempfile::TempDir,
    archive: EmbeddedArchive,
    archive_contents: Vec<String>,
    expected_install_md5: String,
    blaze_path: String,
    output_dir: String,
}

// Java: src/test/cpp/blaze_archive_test.cc:108 BlazeArchiveTest::SetUp
fn set_up() -> Fixture {
    let temp = tempfile::tempdir().unwrap();
    let expected_install_md5 = "expected_install_md5".to_string();
    let blaze_zip_file_to_contents = get_archive_path_to_contents(&expected_install_md5);
    let archive_contents = get_archive_paths();
    let temp_path = temp.path().to_str().unwrap().to_string();
    let blaze_path = join_path(&temp_path, "blaze");
    let archive = EmbeddedArchive::new(
        make_archive(&blaze_zip_file_to_contents),
        blaze_zip_file_to_contents
            .iter()
            .map(|(p, _)| p.clone())
            .collect(),
        expected_install_md5.clone(),
        "",
        "A-server.jar",
    );
    let install_base = "install_base";
    let output_dir = join_path(&temp_path, install_base);
    Fixture {
        _temp: temp,
        archive,
        archive_contents,
        expected_install_md5,
        blaze_path,
        output_dir,
    }
}

fn mtime_secs(path: &str) -> i64 {
    use std::os::unix::fs::MetadataExt;
    std::fs::metadata(path).unwrap().mtime()
}

// Java: src/test/cpp/blaze_archive_test.cc:126 BlazeArchiveTest.TestZipExtractionAndFarOutMTimes
#[test]
fn test_zip_extraction_and_far_out_m_times() {
    let f = set_up();
    let mut startup_options = StartupOptions::bazel();
    set_startup_options(&mut startup_options, &f.output_dir);
    let mut logging_info = LoggingInfo::new(f.blaze_path.clone(), get_milliseconds_monotonic());

    let extraction_time = extract_data(
        &f.blaze_path,
        &f.archive,
        &f.archive_contents,
        &f.expected_install_md5,
        &startup_options,
        &mut logging_info,
    );
    assert!(extraction_time.is_some());

    let foo_path = join_path(&f.output_dir, "foo");
    let bar_path = join_path(&f.output_dir, "bar");
    let baz_path = join_path(&f.output_dir, "path/to/subdir/baz");

    assert_eq!(std::fs::read(&foo_path).unwrap(), b"foo content");
    assert_eq!(std::fs::read(&bar_path).unwrap(), b"bar content");
    assert_eq!(std::fs::read(&baz_path).unwrap(), b"baz content");

    assert!(is_untampered(&Path::new(&foo_path)));
    assert!(is_untampered(&Path::new(&bar_path)));
    assert!(is_untampered(&Path::new(&baz_path)));

    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap()
        .as_secs() as i64;
    let far_future = now + 3600 * 24 * 365 * 9;
    assert!(mtime_secs(&foo_path) > far_future);
    assert!(mtime_secs(&bar_path) > far_future);
    assert!(mtime_secs(&baz_path) > far_future);

    // PosixDumper writes every member with mode 0755.
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        0o755,
        std::fs::metadata(&foo_path).unwrap().permissions().mode() & 0o777
    );
    // install_base_key is extracted too.
    assert_eq!(
        std::fs::read(join_path(&f.output_dir, "install_base_key")).unwrap(),
        b"expected_install_md5"
    );
}

// Java: src/test/cpp/blaze_archive_test.cc:158 BlazeArchiveTest.TestNoDataExtractionIfInstallBaseExists
#[test]
fn test_no_data_extraction_if_install_base_exists() {
    let f = set_up();
    let mut startup_options = StartupOptions::bazel();
    set_startup_options(&mut startup_options, &f.output_dir);
    let mut logging_info = LoggingInfo::new(f.blaze_path.clone(), get_milliseconds_monotonic());

    let extraction_time_one = extract_data(
        &f.blaze_path,
        &f.archive,
        &f.archive_contents,
        &f.expected_install_md5,
        &startup_options,
        &mut logging_info,
    );
    assert!(extraction_time_one.is_some());

    let extraction_time_two = extract_data(
        &f.blaze_path,
        &f.archive,
        &f.archive_contents,
        &f.expected_install_md5,
        &startup_options,
        &mut logging_info,
    );
    assert!(extraction_time_two.is_none());
}

/// A tampered file (mtime reset) makes ExtractData reinstall.
#[test]
fn tampered_installation_is_reextracted() {
    let f = set_up();
    let mut startup_options = StartupOptions::bazel();
    set_startup_options(&mut startup_options, &f.output_dir);
    let mut logging_info = LoggingInfo::new(f.blaze_path.clone(), get_milliseconds_monotonic());
    let run = |so: &StartupOptions, li: &mut LoggingInfo| {
        extract_data(
            &f.blaze_path,
            &f.archive,
            &f.archive_contents,
            &f.expected_install_md5,
            so,
            li,
        )
    };
    assert!(run(&startup_options, &mut logging_info).is_some());
    std::fs::write(join_path(&f.output_dir, "bar"), "changed").unwrap();
    assert!(run(&startup_options, &mut logging_info).is_some());
    assert_eq!(
        std::fs::read(join_path(&f.output_dir, "bar")).unwrap(),
        b"bar content"
    );
    assert!(run(&startup_options, &mut logging_info).is_none());
}
