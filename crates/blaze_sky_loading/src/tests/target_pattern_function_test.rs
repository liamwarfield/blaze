//! Target pattern resolution: TARGET_PATTERN and `SkyframeTargetPatternEvaluator`.
//!
//! Java: src/test/java/com/google/devtools/build/lib/skyframe/TargetPatternFunctionTest.java and
//! SkyframeTargetPatternEvaluatorTest.java (subset; the rest needs incremental invalidation and
//! test rules).

use std::sync::Arc;

use blaze_cmdline::TargetPattern;
use blaze_events::StoredEventHandler;

use crate::package_function::SkyframePackageLocator;
use crate::package_manager::SkyframePackageManager;
use crate::test_util::{FakeRepos, TestSetup};

fn setup() -> TestSetup {
    TestSetup::with_bazel_rules(FakeRepos::default())
}

fn file(setup: &TestSetup, rel: &str, contents: &str) {
    setup.scratch.file(&setup.ws(rel), contents);
}

fn manager(setup: &TestSetup) -> SkyframePackageManager {
    SkyframePackageManager::new(
        Arc::new(setup.evaluator()),
        SkyframePackageLocator {
            pkg_locator: setup.config.pkg_locator.clone(),
            deleted_packages: setup.config.deleted_packages.clone(),
        },
    )
}

fn preload(setup: &TestSetup, pattern: &str) -> (Vec<String>, Arc<StoredEventHandler>) {
    let m = manager(setup);
    let events = Arc::new(StoredEventHandler::new());
    let result = m
        .preload_target_patterns(
            events.clone(),
            TargetPattern::default_parser(),
            &[pattern.to_owned()],
            true,
        )
        .expect("no error");
    let mut labels: Vec<String> = result[pattern]
        .iter()
        .map(|t| t.get_label().to_string())
        .collect();
    labels.sort();
    (labels, events)
}

#[test]
fn recursive_pattern_finds_targets_in_all_packages() {
    let s = setup();
    file(&s, "a/BUILD", "filegroup(name = 'x')");
    file(&s, "a/b/BUILD", "filegroup(name = 'y')");
    file(&s, "c/d.txt", "");
    let (labels, _) = preload(&s, "//...");
    assert_eq!(labels, vec!["//a/b:y", "//a:x"]);
}

#[test]
fn all_in_package_lists_rules_only() {
    let s = setup();
    file(&s, "a/BUILD", "filegroup(name = 'x', srcs = ['f.txt'])");
    file(&s, "a/f.txt", "");
    let (labels, _) = preload(&s, "//a:all");
    assert_eq!(labels, vec!["//a:x"]);
    let (labels, _) = preload(&s, "//a:*");
    assert_eq!(labels, vec!["//a:BUILD", "//a:f.txt", "//a:x"]);
}

#[test]
fn missing_package_is_skipped_with_error() {
    let s = setup();
    file(&s, "a/BUILD", "");
    let (labels, events) = preload(&s, "//nope:all");
    assert!(labels.is_empty());
    let messages: Vec<String> = events
        .get_events()
        .iter()
        .map(|e| e.get_message().into_owned())
        .collect();
    assert!(
        messages
            .iter()
            .any(|m| m.starts_with("Skipping '//nope:all': ")),
        "{:?}",
        messages
    );
}
