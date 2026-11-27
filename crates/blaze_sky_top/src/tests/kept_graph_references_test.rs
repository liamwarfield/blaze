//! Not a Java test: the references that would keep a command's evaluator alive after its
//! executor is gone. A server keeps the graph between commands only when nothing but the
//! executor holds the evaluator (blaze_sky_top `incremental`); an `Arc` cycle through a graph
//! value (the regression of 2026-11-26: the artifact factory's package roots held the
//! evaluator, so every daemon command after a build that executed rebuilt the graph) both
//! leaks the evaluator and discards the kept graph.

use std::sync::Arc;

use blaze_cmdline::PackageIdentifier;

use crate::testing::BuildViewTestCase;

// The lookup `ExecutionTool.prepareForExecution` hands to `IncrementalPackageRoots` (and through
// it to the artifact factory, which graph values reach) must not hold the evaluator.
#[test]
fn root_for_done_package_lookup_does_not_keep_the_evaluator_alive() {
    let mut t = BuildViewTestCase::new();
    t.scratch.file("pkg/BUILD", &["filegroup(name = 'g')\n"]);
    t.get_configured_target("//pkg:g").unwrap();
    let executor = t.skyframe_executor();
    let lookup = executor.get_root_for_done_package();
    let evaluator = Arc::downgrade(executor.get_evaluator());
    assert!(
        lookup
            .get_root_for_package(&PackageIdentifier::create_in_main_repo_str("pkg"))
            .is_some(),
        "the lookup reads the graph while the executor lives"
    );
    t.invalidate_packages();
    assert!(
        evaluator.upgrade().is_none(),
        "the package root lookup keeps the evaluator of a dropped executor alive"
    );
    // A lookup that outlived its executor is a bug of its holder: it fails loudly.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        lookup.get_root_for_package(&PackageIdentifier::create_in_main_repo_str("pkg"))
    }));
    assert!(result.is_err());
}
