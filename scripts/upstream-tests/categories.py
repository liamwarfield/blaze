"""Why upstream tests do not count on this machine (pin 9.3.2, Fedora 44 x86-64, 16 cores).

summarize uses OVERRIDES (per target, set after reading the logs of both baseline runs and a
--test-jobs 2 retry) first, then the first matching pattern over the failing shards' test.log.
VACUOUS targets pass without running a test function on Linux; they are kept out of the
passing list, since any binary passes them.
"""

CATEGORY_DOC = {
    "host-gcc-gcov": "gcov output of the host GCC 16.2 differs from the expected coverage text "
                     "(upstream CI also skips these on Rocky Linux 8 for its gcov version)",
    "host-no-clang": "test builds with --repo_env=CC=clang (or a module that does); clang is "
                     "not installed",
    "host-jdk": "@local_jdk resolves to the host javac's JDK 11 (alternatives: java is 25, "
                "javac is temurin-11); VanillaJavaBuilder needs Java 17+",
    "release-binary": "expects 'release: development version' from a from-source //src:bazel; "
                      "the binary under test is a release build",
    "memory-cap": "OOM-killed under the job's MemoryMax=14G",
    "network": "needs network the sandbox blocks or an external host",
    "timeout": "hit the size timeout on this machine",
    "uncategorized": "not yet classified",
}

# (category, regex), first match wins.
PATTERNS = [
    ("host-no-clang", r"Cannot find gcc or CC \(clang\)"),
    ("network", r"UnknownHostException|Network is unreachable|Temporary failure in name resolution"
                r"|Error downloading|connect timed out"),
]

# label -> category, from reading the logs.
OVERRIDES = {
    "//src/test/shell/bazel:bazel_cc_code_coverage_test": "host-gcc-gcov",
    "//src/test/shell/bazel:bazel_coverage_cc_head_test_gcc": "host-gcc-gcov",
    "//src/test/shell/bazel:bazel_coverage_cc_released_test_gcc": "host-gcc-gcov",
    "//src/test/shell/bazel:cc_integration_test": "host-no-clang",
    "//src/test/shell/bazel:path_mapping_test": "host-no-clang",
    "//src/test/shell/integration:bazel_java_test": "host-jdk",
    "//src/test/shell/integration:info_test": "release-binary",
    "//src/test/shell/bazel:bazel_proto_library_test": "memory-cap",  # if it fails every run
}

# Flaky on the oracle here (pass in some baseline runs, not in others), with the cause.
FLAKY_DOC = {
    "//src/test/shell/bazel:bazel_proto_library_test":
        "shard 2's inner Bazel compiles protobuf with 16 parallel C++ jobs and is OOM-killed when "
        "the job's MemoryMax=14G is reached (also at --test-jobs 2); passes when memory allows",
    "//src/test/py/bazel:bazel_module_test":
        "testReverseDependencyDirection: 'JUST FETCHED: bar' missing (refetch timing); failed in "
        "run 2 and in the retry",
    "//src/test/shell/bazel:external_integration_test":
        "test_deferred_download_* hang on their fifo-gated HTTP server until the 900s timeout "
        "(run 2: shard 1; retry: shards 1 and 15)",
    "//src/test/shell/integration:bazel_worker_test":
        "test_worker_restarts_when_worker_runfiles_change: worker restart count 1, expected 2",
    "//src/test/shell/integration:build_event_stream_test":
        "test_glob_filesystem_operation_cost: cost differs; a large failure log then overflows "
        "unittest.bash's perl XML writer (Argument list too long), losing later testcases",
}

# Pass without running any test function on Linux.
VACUOUS = {
    "//src/test/py/bazel:bazel_windows_cpp_test": "windows-only",
    "//src/test/py/bazel:bazel_windows_symlinks_test": "windows-only",
    "//src/test/py/bazel:bazel_windows_test": "windows-only",
    "//src/test/py/bazel:test_wrapper_test": "windows-only",
    "//src/test/shell/bazel:bazel_windows_example_test": "windows-only",
    "//src/test/shell/bazel:bazel_windows_java8_cross_drive_test": "windows-only",
    "//src/test/shell/bazel:bazel_windows_java11_cross_drive_test": "windows-only",
    "//src/test/shell/bazel:bazel_windows_java17_cross_drive_test": "windows-only",
    "//src/test/shell/bazel:bazel_windows_java21_cross_drive_test": "windows-only",
    "//src/test/shell/bazel/apple:bazel_apple_test": "macos-only",
    "//src/test/shell/bazel/apple:bazel_objc_test": "macos-only",
    "//src/test/shell/bazel:cpp_darwin_integration_test": "macos-only",
    "//src/test/shell/bazel:bazel_java_test_no_windows": "never calls run_suite (upstream)",
    "//src/test/shell/bazel:cc_import_starlark_test": "run_suite commented out (upstream)",
}
