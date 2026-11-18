//! Port of `src/test/java/com/google/devtools/build/lib/runtime/TargetSummaryEventTest.java`
//! and of the build-event test of `TestSummaryTest.java` (`testAsStreamProto`); the other
//! `TestSummaryTest` tests cover `TestSummary` itself (crate `blaze_buildtool`).

use blaze_bep::buildeventstream::build_event::{BuildEvent, BuildEventClass, LocalFileType};
use blaze_bep::buildeventstream::build_event_context::{
    ArtifactGroupNamer, ArtifactGroupNode, BuildEventContext, FileUriPathConverter, NoConversion,
    PathConverter,
};
use blaze_bep::buildeventstream::build_event_id_util;
use blaze_bep::runtime::test_summary_event::{TargetSummaryEvent, TestSummaryEvent};
use blaze_bep::util::{duration_from_millis, timestamp_from_millis};
use blaze_cmdline::label::Label;
use blaze_optdefs::misc::build_event_protocol_options::BuildEventProtocolOptions;
use blaze_protos::blaze::BlazeTestStatus;
use blaze_protos::build_event_stream::build_event::Payload;
use blaze_protos::build_event_stream::build_event_id::NamedSetOfFilesId;
use blaze_protos::build_event_stream::{self as bes, TestStatus};
use blaze_protos::json::JsonFormat;
use blaze_vfs::file_system::FileSystemRef;
use blaze_vfs::{DigestHashFunction, Path, UnixFileSystem};

// ---------------------------------------------------------------------------------------------
// Test support
// ---------------------------------------------------------------------------------------------

struct NoNamer;

impl ArtifactGroupNamer for NoNamer {
    fn apply(&self, _node: &ArtifactGroupNode) -> Option<NamedSetOfFilesId> {
        None
    }
}

struct TestContext {
    path_converter: Box<dyn PathConverter>,
}

impl BuildEventContext for TestContext {
    fn path_converter(&self) -> &dyn PathConverter {
        self.path_converter.as_ref()
    }

    fn artifact_group_namer(&self) -> &dyn ArtifactGroupNamer {
        &NoNamer
    }

    fn get_options(&self) -> &BuildEventProtocolOptions {
        unreachable!("summary events do not read the BEP options")
    }
}

/// Java passes a null context to `TargetSummaryEvent.asStreamProto`.
fn null_context() -> TestContext {
    TestContext {
        path_converter: Box::new(NoConversion),
    }
}

/// `pathConverter.apply(path)` answers `"/path/to" + path.getPathString()`.
struct PrefixPathConverter;

impl PathConverter for PrefixPathConverter {
    fn apply(&self, path: &Path) -> Option<String> {
        Some(format!("/path/to{}", path.get_path_string()))
    }
}

fn file_system() -> FileSystemRef {
    UnixFileSystem::create(DigestHashFunction::Sha256)
}

fn uri_file(uri: &str) -> bes::File {
    bes::File {
        file: Some(bes::file::File::Uri(uri.to_string())),
        ..Default::default()
    }
}

// ---------------------------------------------------------------------------------------------
// TargetSummaryEventTest
// ---------------------------------------------------------------------------------------------

const PATH: &str = "package";
const TARGET_NAME: &str = "name";

/// `BuildConfigurationKey.create(BuildOptions.builder().build()).getOptions().checksum()`.
/// PORT: the tests only need some checksum; the value of the empty options' checksum is not
/// computed here.
const CHECKSUM: &str = "5b0bdbba0e5c2ac41b7aa7b84e0f2fa3e3a4bb6a0ebed52d8b1e5e4d1d0a2f11";

fn label() -> Label {
    Label::parse_canonical_unchecked(&format!("//{PATH}:{TARGET_NAME}"))
}

/// `TargetSummaryEvent.create(stubTarget(), ...)`.
fn create_for_stub_target(
    overall_build_success: bool,
    expect_test_summary: bool,
    overall_test_status: Option<BlazeTestStatus>,
) -> TargetSummaryEvent {
    TargetSummaryEvent::create(
        &label(),
        Some(CHECKSUM),
        overall_build_success,
        expect_test_summary,
        overall_test_status,
    )
}

fn target_summary_payload(proto: &bes::BuildEvent) -> bes::TargetSummary {
    match &proto.payload {
        Some(Payload::TargetSummary(summary)) => *summary,
        other => panic!("not a target summary: {other:?}"),
    }
}

#[test]
fn test_get_event_id() {
    let event = TargetSummaryEvent::create(&label(), Some(CHECKSUM), false, false, None);
    assert_eq!(
        event.get_event_id(),
        build_event_id_util::target_summary(
            &label().to_string(),
            &build_event_id_util::configuration_id(CHECKSUM)
        )
    );
}

#[test]
fn test_get_event_id_null_config() {
    let event = TargetSummaryEvent::create(&label(), None, false, false, None);
    assert_eq!(
        event.get_event_id(),
        build_event_id_util::target_summary(
            &label().to_string(),
            &build_event_id_util::null_configuration_id()
        )
    );
}

#[test]
fn test_posted_after_no_test_summary() {
    let event = create_for_stub_target(false, false, None);
    assert_eq!(
        event.posted_after(),
        Some(vec![build_event_id_util::target_completed(
            &label(),
            &build_event_id_util::configuration_id(CHECKSUM)
        )])
    );
}

#[test]
fn test_posted_after_expect_test_summary() {
    let event = create_for_stub_target(false, true, None);
    assert_eq!(
        event.posted_after(),
        Some(vec![
            build_event_id_util::target_completed(
                &label(),
                &build_event_id_util::configuration_id(CHECKSUM)
            ),
            build_event_id_util::test_summary(
                &label().to_string(),
                &build_event_id_util::configuration_id(CHECKSUM)
            ),
        ])
    );
}

#[test]
fn test_posted_after_null_config() {
    let event = TargetSummaryEvent::create(&label(), None, false, true, None);
    assert_eq!(
        event.posted_after(),
        Some(vec![
            build_event_id_util::target_completed(
                &label(),
                &build_event_id_util::null_configuration_id()
            ),
            build_event_id_util::test_summary(
                &label().to_string(),
                &build_event_id_util::null_configuration_id()
            ),
        ])
    );
}

#[test]
fn test_as_stream_proto_for_test() {
    let event = create_for_stub_target(true, true, Some(BlazeTestStatus::Flaky));
    let proto = event.as_stream_proto(&null_context()).unwrap();
    assert_eq!(proto.id, Some(event.get_event_id()));
    assert!(target_summary_payload(&proto).overall_build_success);
    assert_eq!(
        target_summary_payload(&proto).overall_test_status,
        TestStatus::Flaky as i32
    );
}

#[test]
fn test_as_stream_proto_for_build_success() {
    let event = create_for_stub_target(true, false, None);
    let proto = event.as_stream_proto(&null_context()).unwrap();
    assert_eq!(proto.id, Some(event.get_event_id()));
    assert!(target_summary_payload(&proto).overall_build_success);
    assert_eq!(
        target_summary_payload(&proto).overall_test_status,
        TestStatus::NoStatus as i32
    );
}

#[test]
fn test_as_stream_proto_failed_build_ignores_test_result() {
    let event = create_for_stub_target(false, true, Some(BlazeTestStatus::Passed));
    let proto = event.as_stream_proto(&null_context()).unwrap();
    assert_eq!(proto.id, Some(event.get_event_id()));
    assert!(!target_summary_payload(&proto).overall_build_success);
    assert_eq!(
        target_summary_payload(&proto).overall_test_status,
        TestStatus::NoStatus as i32
    );
}

// ---------------------------------------------------------------------------------------------
// TestSummaryTest (build-event part)
// ---------------------------------------------------------------------------------------------

/// `getTemplateBuilder()`: configuration checksum "abcdef", PASSED, not cached.
fn template_summary() -> TestSummaryEvent {
    TestSummaryEvent {
        label: label(),
        configuration_checksum: "abcdef".to_string(),
        status: BlazeTestStatus::Passed,
        num_cached: 0,
        total_runs: 0,
        num_attempts: 0,
        runs: 0,
        shards: 0,
        first_start_time_millis: 0,
        last_stop_time_millis: 0,
        total_run_duration_millis: 0,
        passed_logs: Vec::new(),
        failed_logs: Vec::new(),
    }
}

#[test]
#[allow(deprecated)]
fn test_as_stream_proto() {
    let fs = file_system();
    // TestParams: getRuns() = 2, getShards() = 3.
    let summary = TestSummaryEvent {
        status: BlazeTestStatus::Failed,
        runs: 2,
        shards: 3,
        passed_logs: vec![Path::create_from_str("/apple", &fs)],
        failed_logs: vec![Path::create_from_str("/pear", &fs)],
        // mergeTiming(1000, 300)
        first_start_time_millis: 1000,
        last_stop_time_millis: 1300,
        total_run_duration_millis: 300,
        ..template_summary()
    };
    let converters = TestContext {
        path_converter: Box::new(PrefixPathConverter),
    };

    let proto = summary.as_stream_proto(&converters).unwrap();
    let test_summary = match proto.payload {
        Some(Payload::TestSummary(s)) => s,
        other => panic!("not a test summary: {other:?}"),
    };
    assert_eq!(
        test_summary,
        bes::TestSummary {
            overall_status: TestStatus::Failed as i32,
            first_start_time_millis: 1000,
            first_start_time: Some(timestamp_from_millis(1000)),
            last_stop_time_millis: 1300,
            last_stop_time: Some(timestamp_from_millis(1300)),
            total_run_duration_millis: 300,
            total_run_duration: Some(duration_from_millis(300)),
            run_count: 2,
            shard_count: 3,
            passed: vec![uri_file("/path/to/apple")],
            failed: vec![uri_file("/path/to/pear")],
            ..Default::default()
        }
    );
}

// ---------------------------------------------------------------------------------------------
// Blaze tests
// ---------------------------------------------------------------------------------------------

#[test]
fn test_summary_ids_files_and_class() {
    let fs = file_system();
    let summary = TestSummaryEvent {
        total_runs: 0,
        passed_logs: vec![Path::create_from_str("/apple", &fs)],
        failed_logs: vec![Path::create_from_str("/pear", &fs)],
        ..template_summary()
    };
    let config = build_event_id_util::configuration_id("abcdef");
    assert_eq!(
        summary.get_event_id(),
        build_event_id_util::test_summary(&label().to_string(), &config)
    );
    assert!(summary.get_children_events().is_empty());
    assert_eq!(
        summary.posted_after(),
        Some(vec![build_event_id_util::target_completed(
            &label(),
            &config
        )])
    );
    // Failed logs first, then passed logs.
    let files: Vec<(String, LocalFileType)> = summary
        .referenced_local_files()
        .into_iter()
        .map(|f| (f.path.get_path_string().to_string(), f.type_))
        .collect();
    assert_eq!(
        files,
        vec![
            ("/pear".to_string(), LocalFileType::FailedTestOutput),
            ("/apple".to_string(), LocalFileType::SuccessfulTestOutput),
        ]
    );
    // A summary without runs is vacuous for the streamer.
    assert_eq!(
        summary.event_class(),
        BuildEventClass::TestSummary { total_runs: 0 }
    );
}

/// The re2 oracle's `testSummary` event of `//:string_generator_test` (`bep.json`).
const RE2_ORACLE_TEST_SUMMARY: &str = r#"{"id":{"testSummary":{"label":"//:string_generator_test","configuration":{"id":"d2d2a0afb30f952b7fcf9b350ea1ac28b281e96df9d843b449b97ffd15360a9b"}}},"testSummary":{"totalRunCount":1,"passed":[{"uri":"file:///home/liamwarfield/.cache/blaze/oracle-out/rec-oracle-9.3.2/d4c1ff46e292915bfbb425566b16402b/execroot/_main/bazel-out/k8-fastbuild/testlogs/string_generator_test/test.log"}],"overallStatus":"PASSED","firstStartTimeMillis":"1794967259116","lastStopTimeMillis":"1794967259169","totalRunDurationMillis":"53","runCount":1,"totalRunDuration":"0.053s","firstStartTime":"2026-11-18T02:00:59.116Z","lastStopTime":"2026-11-18T02:00:59.169Z","attemptCount":1}}"#;

#[test]
fn re2_oracle_test_summary() {
    let fs = file_system();
    let summary = TestSummaryEvent {
        label: Label::parse_canonical_unchecked("//:string_generator_test"),
        configuration_checksum: "d2d2a0afb30f952b7fcf9b350ea1ac28b281e96df9d843b449b97ffd15360a9b"
            .to_string(),
        status: BlazeTestStatus::Passed,
        num_cached: 0,
        total_runs: 1,
        num_attempts: 1,
        runs: 1,
        shards: 0,
        first_start_time_millis: 1794967259116,
        last_stop_time_millis: 1794967259169,
        total_run_duration_millis: 53,
        passed_logs: vec![Path::create_from_str(
            "/home/liamwarfield/.cache/blaze/oracle-out/rec-oracle-9.3.2/d4c1ff46e292915bfbb425566b16402b/execroot/_main/bazel-out/k8-fastbuild/testlogs/string_generator_test/test.log",
            &fs,
        )],
        failed_logs: Vec::new(),
    };
    let context = TestContext {
        path_converter: Box::new(FileUriPathConverter),
    };
    let proto = summary.as_stream_proto(&context).unwrap();
    let json = JsonFormat::printer()
        .omitting_insignificant_whitespace()
        .print(&proto)
        .unwrap();
    let actual: serde_json::Value = serde_json::from_str(&json).unwrap();
    let expected: serde_json::Value = serde_json::from_str(RE2_ORACLE_TEST_SUMMARY).unwrap();
    assert_eq!(actual, expected);
    assert_eq!(json, RE2_ORACLE_TEST_SUMMARY);
}
