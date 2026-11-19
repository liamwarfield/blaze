//! Port of src/test/java/com/google/devtools/build/lib/worker/WorkerTest.java (tests of
//! `SingleplexWorker` over a fake process).

mod worker_common;

use std::collections::BTreeSet;
use std::sync::Arc;

use blaze_exec::actions::execution_requirements::WorkerProtocolFormat;
use blaze_exec::sandbox::sandbox_helpers::{SandboxInputs, SandboxOutputs};
use blaze_exec::worker::recording_input_stream::ByteArrayInputStream;
use blaze_exec::worker::singleplex_worker::SingleplexWorker;
use blaze_exec::worker::worker::{Worker, WorkerError};
use blaze_exec::worker::worker_protocol::parse_delimited_work_response;
use blaze_protos::JavaMessage;
use blaze_protos::blaze::worker::{Input, WorkRequest, WorkResponse};
use blaze_vfs::PathFragment;
use indexmap::IndexMap;
use prost::Message;
use worker_common::*;

struct TestWorker {
    worker: SingleplexWorker,
    fake: Arc<FakeSubprocess>,
}

impl Drop for TestWorker {
    // Java: src/test/java/com/google/devtools/build/lib/worker/WorkerTest.java:57 WorkerTest.destroyWorker
    fn drop(&mut self) {
        self.worker.destroy();
    }
}

fn serialize_response_to_proto_bytes(response: &WorkResponse) -> Vec<u8> {
    let mut buf = Vec::new();
    response.write_delimited_to(&mut buf);
    buf
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerTest.java:71 WorkerTest.createTestWorker
fn create_test_worker(
    output_stream_bytes: Vec<u8>,
    protocol_format: WorkerProtocolFormat,
) -> TestWorker {
    let fs = fs();
    // WorkerTestUtils.createWorkerKey(protocolFormat, fs): mnemonic "dummy", multiplex and
    // sandboxed requested, not dynamic, args arg1..arg3.
    let key = Arc::new(create_worker_key_full(
        protocol_format,
        &fs,
        "dummy",
        true,
        true,
        false,
        &["arg1", "arg2", "arg3"],
    ));
    let fake = FakeSubprocess::new(output_stream_bytes);
    let worker_base_dir = fs.get_path("/outputbase/bazel-workers");
    let log_file = worker_base_dir.get_relative_str("test-log-file.log");
    let worker = SingleplexWorker::new(
        key.clone(),
        1,
        key.get_exec_root().clone(),
        log_file,
        Arc::new(worker_options()),
    );
    worker.set_process_factory(Arc::new(FakeSubprocessFactory(fake.clone())));
    let worker_files: BTreeSet<PathFragment> = key
        .get_worker_files_with_digests()
        .keys()
        .cloned()
        .collect();
    worker
        .prepare_execution(
            &SandboxInputs::get_empty_inputs(),
            &SandboxOutputs::get_empty_instance(),
            &worker_files,
            &IndexMap::new(),
        )
        .map_err(|e| format!("{e:?}"))
        .unwrap();
    TestWorker { worker, fake }
}

use blaze_vfs::FileSystemExt;

fn response(w: &TestWorker, id: i32) -> Result<Option<WorkResponse>, WorkerError> {
    w.worker.get_response(id)
}

#[test]
fn test_put_request_success() {
    let request = WorkRequest::default();
    let test_worker = create_test_worker(Vec::new(), WorkerProtocolFormat::Proto);
    test_worker.worker.put_request(&request).unwrap();
    let stdout = test_worker.fake.output.lock().clone();
    let request_from_stdout = WorkRequest::decode_length_delimited(stdout.as_slice()).unwrap();
    assert_eq!(request_from_stdout, request);
}

#[test]
fn test_get_response_success() {
    let response_in = WorkResponse::default();
    let test_worker = create_test_worker(
        serialize_response_to_proto_bytes(&response_in),
        WorkerProtocolFormat::Proto,
    );
    let read_response = response(&test_worker, 0)
        .map_err(|e| format!("{e:?}"))
        .unwrap();
    assert_eq!(read_response, Some(response_in));
}

#[test]
fn test_put_request_json_success() {
    let test_worker = create_test_worker(Vec::new(), WorkerProtocolFormat::Json);
    test_worker
        .worker
        .put_request(&WorkRequest::default())
        .unwrap();
    let stdout = test_worker.fake.output.lock().clone();
    assert_eq!(String::from_utf8(stdout).unwrap(), "{}\n");
}

#[test]
fn test_get_response_json_success() {
    let test_worker = create_test_worker(b"{}\n".to_vec(), WorkerProtocolFormat::Json);
    let read_response = response(&test_worker, 0)
        .map_err(|e| format!("{e:?}"))
        .unwrap();
    assert_eq!(read_response, Some(WorkResponse::default()));
}

#[test]
fn test_put_request_json_populated_fields_success() {
    let request = WorkRequest {
        arguments: vec!["testRequest".to_string()],
        inputs: vec![Input {
            path: "testPath".to_string(),
            digest: b"testDigest".to_vec(),
        }],
        request_id: 1,
        verbosity: 11,
        ..Default::default()
    };
    let test_worker = create_test_worker(Vec::new(), WorkerProtocolFormat::Json);
    test_worker.worker.put_request(&request).unwrap();
    let stdout = test_worker.fake.output.lock().clone();
    assert_eq!(
        String::from_utf8(stdout).unwrap(),
        "{\"arguments\":[\"testRequest\"],\"inputs\":[{\"path\":\"testPath\",\"digest\":\"dGVzdERpZ2VzdA==\"}],\"requestId\":1,\"verbosity\":11}\n"
    );
}

#[test]
fn test_get_response_json_populated_fields_success() {
    let test_worker = create_test_worker(
        b"{\"exitCode\":1,\"output\":\"test output\",\"requestId\":1}".to_vec(),
        WorkerProtocolFormat::Json,
    );
    let read_response = response(&test_worker, 1)
        .map_err(|e| format!("{e:?}"))
        .unwrap();
    assert_eq!(
        read_response,
        Some(WorkResponse {
            exit_code: 1,
            output: "test output".to_string(),
            request_id: 1,
            was_cancelled: false,
        })
    );
}

#[test]
fn test_put_request_destroyed_worker_throws() {
    let test_worker = create_test_worker(Vec::new(), WorkerProtocolFormat::Proto);
    test_worker.worker.destroy();
    assert!(
        test_worker
            .worker
            .put_request(&WorkRequest::default())
            .is_err()
    );
}

#[test]
fn test_get_response_destroyed_worker_throws() {
    let test_worker = create_test_worker(Vec::new(), WorkerProtocolFormat::Proto);
    test_worker.worker.destroy();
    assert!(matches!(response(&test_worker, 0), Err(WorkerError::Io(_))));
}

// Java: src/test/java/com/google/devtools/build/lib/worker/WorkerTest.java:193 WorkerTest.verifyGetResponseFailure
fn verify_get_response_failure(response_string: &str, expected_error: &str) {
    let test_worker = create_test_worker(
        format!("{response_string}\n").into_bytes(),
        WorkerProtocolFormat::Json,
    );
    match response(&test_worker, 0) {
        Err(WorkerError::Io(e)) => assert!(
            e.to_string().contains(expected_error),
            "{e} does not contain {expected_error}"
        ),
        other => panic!("expected an IOException, got {other:?}"),
    }
}

#[test]
fn test_get_response_bad_json_throws() {
    verify_get_response_failure(
        "{ \"output\": \"I'm missing a bracket\"",
        "Could not parse json work request correctly",
    );
}

#[test]
fn test_get_response_json_multiple_exit_code_fails() {
    verify_get_response_failure(
        "{\"exitCode\":1,\"exitCode\":1}",
        "Work response cannot have more than one exit code",
    );
}

#[test]
fn test_get_response_json_multiple_output_fails() {
    verify_get_response_failure(
        "{\"output\":\"\",\"output\":\"\"}",
        "Work response cannot have more than one output",
    );
}

#[test]
fn test_get_response_json_multiple_request_id_fails() {
    verify_get_response_failure(
        "{\"requestId\":0,\"requestId\":0}",
        "Work response cannot have more than one requestId",
    );
}

#[test]
fn test_get_response_json_unknown_fields_ignored() {
    let test_worker = create_test_worker(
        b"{\"exitCode\":1,\"output\":\"test output\",\"requestId\":1,\"unknown\":{1:['a']}}"
            .to_vec(),
        WorkerProtocolFormat::Json,
    );
    let read_response = response(&test_worker, 1)
        .map_err(|e| format!("{e:?}"))
        .unwrap();
    assert_eq!(
        read_response,
        Some(WorkResponse {
            exit_code: 1,
            output: "test output".to_string(),
            request_id: 1,
            was_cancelled: false,
        })
    );
}

#[allow(dead_code)]
fn unused(_: ByteArrayInputStream) -> Option<WorkResponse> {
    parse_delimited_work_response(&mut ByteArrayInputStream::new(Vec::new()))
        .ok()
        .flatten()
}
