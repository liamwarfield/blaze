//! The JUnit tests of the Java rule classes that analyse BUILD files with rules_java through
//! `BuildViewTestCase` run on the analysis test harness, which lives in `blaze_sky_top` (a crate
//! that depends on this one), so they are ported there:
//!
//! - `JavaCompileActionBuilderTest`: `blaze_sky_top/src/tests/java_compile_action_builder_test.rs`
//! - `JavaRuntimeTest`: `blaze_sky_top/src/tests/java_runtime_test.rs`
//! - `JavaStarlarkApiTest`: `blaze_sky_top/src/tests/java_starlark_api_test.rs`
