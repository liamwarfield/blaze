//! Scratch-workspace harness for the BUILD-loading JUnit tests (Java `PackageLoadingTestCase` and
//! the loading half of `BuildViewTestCase`): files on a temp directory, the real loading
//! SkyFunctions (PACKAGE, BZL_LOAD, ...) with Bazel's rule class provider.
//!
//! Differences from the Java test environment, which tests must respect:
//! - The rule class provider is Bazel's (`RuleClassProvider::create_bazel`), not
//!   `TestRuleClassProvider`: the prelude is `//tools/build_rules:prelude_bazel` (Java tests
//!   use `//tools/test_build_rules:test_prelude`), and only Bazel's native rules exist.
//! - Builtins injection is off by default (`--experimental_builtins_bzl_path=`), so BUILD files
//!   see the native rules directly; [`PackageLoadingTestCase::set_build_language_options`] can
//!   turn it back on (it then reads the install base's `@_builtins`, which scratch workspaces
//!   do not have).
//! - There is no analysis: `getConfiguredTarget("//pkg:x")` in a Java test whose assertion is
//!   about loading becomes [`PackageLoadingTestCase::get_package`].

use std::sync::Arc;

use blaze_cmdline::PackageIdentifier;
use blaze_events::{Event, EventKind, StoredEventHandler};
use blaze_optdefs::command::package_options::PackageOptions;
use blaze_packages::exceptions::NoSuchPackageException;
use blaze_packages::package::Package;
use blaze_packages::target::{Rule, RuleOrMacroInstance, Target};
use blaze_packages::types::AttrValue;
use blaze_skyframe::{EvaluationResult, SkyKey};

use crate::package_function::PackageKey;
use crate::test_util::{FakeRepos, TestSetup, evaluate};

/// One scratch workspace and its loading configuration.
pub(crate) struct PackageLoadingTestCase {
    pub setup: TestSetup,
}

/// A loaded package and the events of loading it (the package's own stored events, replayed by
/// PACKAGE, and Skyframe's).
pub(crate) struct LoadedPackage {
    pub package: Arc<Package>,
    pub events: Vec<Event>,
}

impl PackageLoadingTestCase {
    pub fn new() -> PackageLoadingTestCase {
        PackageLoadingTestCase {
            setup: TestSetup::with_bazel_rules(FakeRepos::default()),
        }
    }

    /// Java `scratch.file(path, lines...)`: `path` is relative to the workspace.
    pub fn scratch_file(&self, path: &str, lines: &[&str]) {
        self.setup
            .scratch
            .file(&self.setup.ws(path), &lines.join("\n"));
    }

    /// Java `setBuildLanguageOptions("--flag=value", ...)` (also `--flag` / `--noflag`).
    pub fn set_build_language_options(&mut self, options: &[&str]) {
        for option in options {
            let option = option.strip_prefix("--").expect("an option starts with --");
            let (name, value) = match option.split_once('=') {
                Some((name, value)) => (name.to_owned(), value.to_owned()),
                None => match option.strip_prefix("no") {
                    Some(name) => (name.to_owned(), "false".to_owned()),
                    None => (option.to_owned(), "true".to_owned()),
                },
            };
            self.setup
                .semantics
                .set_flag(&name, &value)
                .unwrap_or_else(|e| panic!("--{option}: {e:?}"));
        }
    }

    /// Java `setPackageOptions(...)`: edits the command's `PackageOptions` and syncs package
    /// loading with them. Call it after the scratch files of the package path exist.
    pub fn set_package_options(&mut self, edit: impl FnOnce(&mut PackageOptions)) {
        let mut package_options = self.setup.package_options.clone().unwrap_or_default();
        edit(&mut package_options);
        self.setup.set_package_options(package_options);
    }

    /// Java's absolute `/workspace/<rel>` (the scratch workspace is a temp directory here, so
    /// expected texts that name a workspace path are built with this).
    pub fn workspace_path(&self, rel: &str) -> String {
        format!("{}/{}", self.setup.workspace().get_path_string(), rel)
    }

    /// Java `PackageLoadingTestCase.parsePackageOptions`: every Java loading test runs with
    /// `--default_visibility=public`. The harness default is the flag's default (private), so a
    /// test that asserts on visibility calls this first.
    pub fn use_public_default_visibility(&mut self) {
        self.set_package_options(|o| {
            o.default_visibility = blaze_optdefs::command::rule_visibility::RuleVisibility::PUBLIC;
        });
    }

    /// Java `scratch.dir(path)`: `path` is relative to the workspace.
    pub fn scratch_dir(&self, path: &str) {
        self.setup.scratch.dir(&self.setup.ws(path));
    }

    /// Java `scratch.deleteFile(path)`: `path` is relative to the workspace.
    pub fn delete_file(&self, path: &str) {
        let file = self.setup.scratch.path(&self.setup.ws(path));
        let _ = std::fs::remove_file(crate::test_util::platform(&file));
    }

    fn key(pkg: &str) -> PackageKey {
        PackageKey::key(PackageIdentifier::create_in_main_repo_str(pkg))
    }

    /// Evaluates PACKAGE for `pkg` with a fresh evaluator (so files written since the last
    /// call are seen: Java `invalidatePackages()`).
    pub fn evaluate_package(&self, pkg: &str) -> (EvaluationResult, Vec<Event>) {
        let evaluator = self.setup.evaluator();
        let events = Arc::new(StoredEventHandler::new());
        let result = evaluate(
            &evaluator,
            vec![SkyKey::new(Self::key(pkg))],
            false,
            events.clone(),
        );
        (result, events.get_events())
    }

    /// [`PackageLoadingTestCase::evaluate_package`] in keep-going mode (Java
    /// `SkyframeExecutorTestUtils.evaluate(..., /* keepGoing= */ true, reporter)`).
    pub fn evaluate_package_keep_going(&self, pkg: &str) -> (EvaluationResult, Vec<Event>) {
        let evaluator = self.setup.evaluator();
        let events = Arc::new(StoredEventHandler::new());
        let result = evaluate(
            &evaluator,
            vec![SkyKey::new(Self::key(pkg))],
            true,
            events.clone(),
        );
        (result, events.get_events())
    }

    /// For a Java test that expects loading or analysis of a target of `pkg` to fail with an
    /// error reported while the package is loaded (`update(...)` fails, `getConfiguredTarget`
    /// returns null): evaluates PACKAGE, asserts that it threw or that the package contains
    /// errors, and returns the events.
    pub fn load_failure_events(&self, pkg: &str) -> Vec<Event> {
        let (result, events) = self.evaluate_package(pkg);
        if !result.has_error() {
            let value = result.get(&Self::key(pkg)).expect("the package value");
            assert!(
                value.get_package().contains_errors(),
                "package '{pkg}' loaded without errors; events: {:#?}",
                event_strings(&events)
            );
        }
        events
    }

    /// Java `getPackage(pkg)` / `loadPackage`: the package, which may contain errors.
    pub fn get_package(&self, pkg: &str) -> LoadedPackage {
        let (result, events) = self.evaluate_package(pkg);
        assert!(
            !result.has_error(),
            "{:?}\nevents: {:?}",
            result,
            messages(&events)
        );
        LoadedPackage {
            package: result
                .get(&Self::key(pkg))
                .expect("the package value")
                .get_package()
                .clone(),
            events,
        }
    }

    /// The package, asserting it has no errors and loading reported no error event.
    pub fn get_package_without_errors(&self, pkg: &str) -> LoadedPackage {
        let loaded = self.get_package(pkg);
        assert!(
            !loaded.package.contains_errors(),
            "package '{pkg}' contains errors: {:?}",
            messages(&loaded.events)
        );
        loaded.assert_no_error_events();
        loaded
    }

    /// The `NoSuchPackageException` PACKAGE throws for `pkg`.
    pub fn get_package_exception(&self, pkg: &str) -> NoSuchPackageException {
        let (result, _) = self.evaluate_package(pkg);
        result
            .get_error(&SkyKey::new(Self::key(pkg)))
            .expect("an error")
            .exception_as::<NoSuchPackageException>()
            .expect("a NoSuchPackageException")
            .clone()
    }
}

impl LoadedPackage {
    /// Java `pkg.getTarget(name)`; panics if there is none.
    pub fn target(&self, name: &str) -> &Target {
        self.package
            .get_target(name)
            .unwrap_or_else(|e| panic!("{e:?}"))
    }

    /// Java `pkg.getRule(name)`; panics if there is none.
    pub fn rule(&self, name: &str) -> &Arc<Rule> {
        self.package
            .get_rule(name)
            .unwrap_or_else(|| panic!("no rule '{name}' in the package"))
    }

    /// Java `assertContainsEvent(expected)`: some event's message contains `expected`.
    pub fn assert_contains_event(&self, expected: &str) {
        assert!(
            self.events
                .iter()
                .any(|e| e.get_message().contains(expected)),
            "no event contains {expected:?}; events: {:#?}",
            self.messages()
        );
    }

    /// Java `assertDoesNotContainEvent(unexpected)`.
    pub fn assert_does_not_contain_event(&self, unexpected: &str) {
        assert!(
            !self
                .events
                .iter()
                .any(|e| e.get_message().contains(unexpected)),
            "an event contains {unexpected:?}; events: {:#?}",
            self.messages()
        );
    }

    /// Java `assertNoEvents()` restricted to errors (progress and debug events do not count).
    pub fn assert_no_error_events(&self) {
        let errors: Vec<String> = self
            .events
            .iter()
            .filter(|e| e.get_kind() == EventKind::Error)
            .map(|e| e.get_message().into_owned())
            .collect();
        assert!(errors.is_empty(), "unexpected errors: {errors:#?}");
    }

    pub fn messages(&self) -> Vec<String> {
        messages(&self.events)
    }

    /// The raw value of attribute `attr` of rule `rule` (Java `RawAttributeMapper.of(rule)
    /// .getRawAttributeValue`); panics if the rule or the value is missing.
    pub fn attr(&self, rule: &str, attr: &str) -> AttrValue {
        self.rule(rule)
            .get_attr(attr)
            .unwrap_or_else(|| panic!("rule '{rule}' has no value for attribute '{attr}'"))
    }

    /// Java `attributes(rule).get(attr, BuildType.LABEL_LIST)`, as label strings in stored
    /// order; panics if the value is not a label list (for example a select).
    pub fn label_list_attr(&self, rule: &str, attr: &str) -> Vec<String> {
        match self.attr(rule, attr) {
            AttrValue::LabelList(labels) => labels.iter().map(|l| l.to_string()).collect(),
            other => panic!("attribute '{attr}' of rule '{rule}' is not a label list: {other:?}"),
        }
    }

    /// The messages of the ERROR events (Java `eventCollector.filtered(EventKind.ERROR)`).
    pub fn error_events(&self) -> Vec<String> {
        self.events
            .iter()
            .filter(|e| e.get_kind() == EventKind::Error)
            .map(|e| e.get_message().into_owned())
            .collect()
    }

    /// The messages the Java test `eventCollector` would hold: it keeps
    /// `EventKind.ERRORS_WARNINGS_AND_INFO` (FoundationTestCase), so progress events ("Loading
    /// package: ...") are not in it.
    pub fn collected_events(&self) -> Vec<String> {
        self.events
            .iter()
            .filter(|e| EventKind::ERRORS_WARNINGS_AND_INFO.contains(e.get_kind()))
            .map(|e| e.get_message().into_owned())
            .collect()
    }

    /// The messages of the DEBUG events (Starlark `print`).
    pub fn debug_events(&self) -> Vec<String> {
        self.events
            .iter()
            .filter(|e| e.get_kind() == EventKind::Debug)
            .map(|e| e.get_message().into_owned())
            .collect()
    }

    /// Java `assertNoEvents()` (see [`LoadedPackage::collected_events`]).
    pub fn assert_no_events(&self) {
        let collected = self.collected_events();
        assert!(collected.is_empty(), "unexpected events: {collected:#?}");
    }
}

pub(crate) fn messages(events: &[Event]) -> Vec<String> {
    events
        .iter()
        .map(|e| e.get_message().into_owned())
        .collect()
}

/// Java `assertContainsEvent(expected)` (`MoreAsserts.assertContainsEvent`) on the events of
/// [`PackageLoadingTestCase::evaluate_package`]: some event's `toString()`
/// (`<KIND> <location>: <message>`) contains `expected`, so an expectation may name the kind and
/// the location as well as the message.
pub(crate) fn assert_contains_event(events: &[Event], expected: &str) {
    assert!(
        events.iter().any(|e| e.to_string().contains(expected)),
        "no event contains {expected:?}; events: {:#?}",
        event_strings(events)
    );
}

/// Java `assertDoesNotContainEvent(unexpected)` on the events of
/// [`PackageLoadingTestCase::evaluate_package`] (matched like [`assert_contains_event`]).
pub(crate) fn assert_does_not_contain_event(events: &[Event], unexpected: &str) {
    assert!(
        !events.iter().any(|e| e.to_string().contains(unexpected)),
        "an event contains {unexpected:?}; events: {:#?}",
        event_strings(events)
    );
}

/// Java `Event.toString()` of each event.
pub(crate) fn event_strings(events: &[Event]) -> Vec<String> {
    events.iter().map(|e| e.to_string()).collect()
}

/// The sorted copy of `values` (for Truth's `containsExactly` without `inOrder`).
pub(crate) fn sorted(mut values: Vec<String>) -> Vec<String> {
    values.sort();
    values
}
