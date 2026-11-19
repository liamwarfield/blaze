//! Ports of Java analysis JUnit tests that run on the analysis test harness
//! (`crate::testing`, Java `BuildViewTestCase` / `AnalysisTestCase`).

mod alias_test;
mod aspect_aware_attribute_mapper_test;
mod aspect_auto_exec_groups_test;
mod build_view_test;
mod filegroup_configured_target_test;
mod starlark_aspect_attr_aspects_function_test;
mod starlark_aspects_propagation_predicate_test;
mod starlark_aspects_toolchain_propagation_test;
mod starlark_defined_aspects_test_1;
mod starlark_defined_aspects_test_2;
mod starlark_defined_aspects_test_3;
mod visibility_test;
