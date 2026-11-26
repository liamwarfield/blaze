//! The JUnit tests of lib/worker that are not ported yet: one ignored test per Java @Test
//! method (names kept), so that the counts show them. Ported test classes have their own
//! files (worker_*_test.rs).

mod work_request_handler_test {
    // Java tests: src/test/java/com/google/devtools/build/lib/worker/WorkRequestHandlerTest.java
    #[test]
    #[ignore = "WorkRequestHandler (the Java library for writing workers) is not ported"]
    fn test_normal_work_request() {}
    #[test]
    #[ignore = "WorkRequestHandler (the Java library for writing workers) is not ported"]
    fn test_multiplex_work_request() {}
    #[test]
    #[ignore = "WorkRequestHandler (the Java library for writing workers) is not ported"]
    fn test_multiplex_work_request_stops_threads_on_shutdown() {}
    #[test]
    #[ignore = "WorkRequestHandler (the Java library for writing workers) is not ported"]
    fn test_multiplex_work_request_stops_worker_on_exception() {}
    #[test]
    #[ignore = "WorkRequestHandler (the Java library for writing workers) is not ported"]
    fn test_output() {}
    #[test]
    #[ignore = "WorkRequestHandler (the Java library for writing workers) is not ported"]
    fn test_exception() {}
    #[test]
    #[ignore = "WorkRequestHandler (the Java library for writing workers) is not ported"]
    fn test_cancel_request_exactly_one_response_sent() {}
    #[test]
    #[ignore = "WorkRequestHandler (the Java library for writing workers) is not ported"]
    fn test_cancel_request_sends_response_when_done() {}
    #[test]
    #[ignore = "WorkRequestHandler (the Java library for writing workers) is not ported"]
    fn test_cancel_request_no_double_cancel_response() {}
    #[test]
    #[ignore = "WorkRequestHandler (the Java library for writing workers) is not ported"]
    fn test_cancel_request_sends_no_response_when_already_sent() {}
    #[test]
    #[ignore = "WorkRequestHandler (the Java library for writing workers) is not ported"]
    fn test_work_request_handler_with_work_request_callback() {}
    #[test]
    #[ignore = "WorkRequestHandler (the Java library for writing workers) is not ported"]
    fn test_worker_io_does_wrap_system_streams() {}
    #[test]
    #[ignore = "WorkRequestHandler (the Java library for writing workers) is not ported"]
    fn test_worker_io_does_capture_standard_out_and_error_streams() {}
}
