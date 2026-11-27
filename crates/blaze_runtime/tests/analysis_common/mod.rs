//! The parts of Java's `AnalysisTestCase`
//! (src/test/java/com/google/devtools/build/lib/analysis/util/AnalysisTestCase.java) that
//! DormantDependencyTest, MaterializerRulesTest and MaterializerRulesRealDepsTest use, on
//! [`BuildIntegrationTestCase`].
//!
//! PORT: Java's `update(labels)` runs loading and analysis on an in-process `BuildView`; here it
//! is `build --nobuild` with the fixture's options and the test's configuration
//! (`useConfiguration`) through the dispatcher, `Err` when the command fails (Java throws
//! `TargetParsingException` or `ViewCreationFailedException`). `getConfiguredTarget(label)`
//! reads the configured target from the graph the command kept.
#![allow(dead_code)]

use std::sync::Arc;

use blaze_analysis::configured_target::ConfiguredTarget;
use blaze_analysis::configured_target_key::{CONFIGURED_TARGET, ConfiguredTargetKey};
use blaze_analysis::configured_target_value::ConfiguredTargetValue;
use blaze_analysis::providers::file_provider::FileProvider;
use blaze_buildtool_testing::BuildIntegrationTestCase;
use blaze_events::{Event, EventKind};

pub struct Fixture {
    pub t: BuildIntegrationTestCase,
    configuration: Vec<String>,
    /// The `@bazel_tools` the test writes to (`TestConstants.TOOLS_REPOSITORY_SCRATCH`), once
    /// it writes there.
    tools_override: Option<std::path::PathBuf>,
}

impl Fixture {
    pub fn new() -> Fixture {
        Fixture {
            t: BuildIntegrationTestCase::new(),
            configuration: Vec::new(),
            tools_override: None,
        }
    }

    /// `scratch.overwriteFile(TestConstants.TOOLS_REPOSITORY_SCRATCH + path, lines...)`.
    ///
    /// PORT: Java's `@bazel_tools` is the mock tools repository it writes into the workspace;
    /// here it is the install base's `embedded_tools`, mirrored (one symbolic link per file) into
    /// a directory of the fixture the first time a test writes there, and used through
    /// `--override_repository=bazel_tools=<that directory>`.
    pub fn write_tools(&mut self, path: &str, lines: &[&str]) {
        let dir = match &self.tools_override {
            Some(dir) => dir.clone(),
            None => {
                let dir = self
                    .t
                    .get_output_base()
                    .parent()
                    .expect("the fixture's directory")
                    .join("bazel_tools_override");
                let source = blaze_buildtool_testing::oracle_install_base().join("embedded_tools");
                mirror(&source, &dir);
                self.t.add_options(&[&format!(
                    "--override_repository=bazel_tools={}",
                    dir.display()
                )]);
                self.tools_override = Some(dir.clone());
                dir
            }
        };
        let file = dir.join(path);
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        let _ = std::fs::remove_file(&file);
        let mut content = String::new();
        for line in lines {
            content.push_str(line);
            content.push('\n');
        }
        std::fs::write(&file, content).unwrap();
    }

    /// `scratch.file(path, lines...)`.
    pub fn write(&self, path: &str, lines: &[&str]) {
        self.t.write(path, lines);
    }

    /// `useConfiguration(args...)`: replaces the configuration of later updates.
    // Java: src/test/java/com/google/devtools/build/lib/analysis/util/AnalysisTestCase.java:282 AnalysisTestCase.useConfiguration
    pub fn use_configuration(&mut self, args: &[&str]) {
        self.configuration = args.iter().map(|a| a.to_string()).collect();
    }

    /// `update(labels...)`: loads and analyzes the labels.
    // Java: src/test/java/com/google/devtools/build/lib/analysis/util/AnalysisTestCase.java:470 AnalysisTestCase.update
    pub fn update(&mut self, labels: &[&str]) -> Result<(), ()> {
        // Java's update does not clear the event collector (tests call eventCollector.clear()).
        let mut args: Vec<String> = vec!["--nobuild".to_owned()];
        args.extend(self.configuration.iter().cloned());
        args.extend(labels.iter().map(|l| l.to_string()));
        let args: Vec<&str> = args.iter().map(|s| s.as_str()).collect();
        if self.t.run_command("build", &args).is_success() {
            return Ok(());
        }
        for e in self.t.events() {
            if matches!(e.get_kind(), EventKind::Error | EventKind::Fatal) {
                eprintln!("update error: {}", e.get_message());
            }
        }
        Err(())
    }

    /// The events `FoundationTestCase`'s collector keeps (`EventKind.ERRORS_WARNINGS_AND_INFO`).
    fn collected(&self) -> Vec<Event> {
        self.t
            .events()
            .into_iter()
            .filter(|e| {
                matches!(
                    e.get_kind(),
                    EventKind::Fatal
                        | EventKind::Error
                        | EventKind::Warning
                        | EventKind::Debug
                        | EventKind::Pass
                        | EventKind::Fail
                        | EventKind::Timeout
                        | EventKind::Info
                )
            })
            .collect()
    }

    /// Java's scratch workspace is `/workspace`; expected event texts naming it name the
    /// fixture's workspace here.
    fn scratch_text(&self, expected: &str) -> String {
        expected.replace(
            "/workspace/",
            &format!("{}/", self.t.get_workspace().display()),
        )
    }

    /// `assertContainsEvent(expectedMessage)`: an event whose `toString()` contains it.
    // Java: src/test/java/com/google/devtools/build/lib/testutil/MoreAsserts.java assertContainsEvent
    pub fn assert_contains_event(&self, expected: &str) {
        let expected = &self.scratch_text(expected);
        let events = self.collected();
        assert!(
            events
                .iter()
                .any(|e| e.to_string().contains(expected.as_str())),
            "no event containing {expected:?} among {:?}",
            events.iter().map(|e| e.get_message()).collect::<Vec<_>>()
        );
    }

    /// `assertDoesNotContainEvent(unexpectedMessage)`.
    pub fn assert_does_not_contain_event(&self, unexpected: &str) {
        let unexpected = &self.scratch_text(unexpected);
        let events = self.collected();
        assert!(
            !events
                .iter()
                .any(|e| e.to_string().contains(unexpected.as_str())),
            "an event contains {unexpected:?}: {:?}",
            events.iter().map(|e| e.get_message()).collect::<Vec<_>>()
        );
    }

    /// `assertContainsEventWithFrequency(expectedMessage, frequency)`.
    pub fn assert_contains_event_with_frequency(&self, expected: &str, frequency: usize) {
        let expected = &self.scratch_text(expected);
        let events = self.collected();
        let count = events
            .iter()
            .filter(|e| e.to_string().contains(expected.as_str()))
            .count();
        assert_eq!(
            count,
            frequency,
            "events containing {expected:?} among {:?}",
            events.iter().map(|e| e.get_message()).collect::<Vec<_>>()
        );
    }

    /// `eventCollector.clear()`.
    pub fn clear_events(&self) {
        self.t.clear_events();
    }

    /// `getConfiguredTarget(label)`: the configured target of `label` in the target
    /// configuration, from the graph the last update kept.
    ///
    /// PORT: Java asks for the key in `getTargetConfiguration()`; here the label's configured
    /// target with a configuration is looked for in the graph, and one in more than one
    /// configuration fails loudly.
    // Java: src/test/java/com/google/devtools/build/lib/analysis/util/AnalysisTestCase.java:507 AnalysisTestCase.getConfiguredTarget
    pub fn get_configured_target(&self, label: &str) -> ConfiguredTarget {
        let evaluator = self.t.get_kept_evaluator().expect("a kept graph");
        let mut found: Vec<(ConfiguredTargetKey, Arc<ConfiguredTargetValue>)> = Vec::new();
        for (key, value) in evaluator.get_done_values() {
            if key.function_name() != CONFIGURED_TARGET {
                continue;
            }
            let Some(ct_key) = key.downcast::<ConfiguredTargetKey>() else {
                continue;
            };
            if ct_key.get_label().to_string() != label
                && ct_key.get_label().get_canonical_form() != label
            {
                continue;
            }
            if ct_key.get_configuration_key().is_none() {
                continue;
            }
            if let Ok(v) = value.clone().downcast::<ConfiguredTargetValue>() {
                found.push((ct_key.clone(), v));
            }
        }
        // A key whose value was computed in another configuration (a top-level target's key
        // before its rule transition) delegates to the target's own key: keep the latter.
        if found.len() > 1 {
            found.retain(|(key, value)| {
                value.get_configured_target().get_configuration_key()
                    == key.get_configuration_key().cloned()
            });
        }
        match found.len() {
            1 => found[0].1.get_configured_target().clone(),
            0 => panic!("no configured target of {label} in the graph"),
            n => panic!(
                "{label} is configured in {n} configurations: {found:?}",
                found = found.iter().map(|f| &f.0).collect::<Vec<_>>()
            ),
        }
    }

    /// `getConfiguredTargetAndTarget(label).getTargetForTesting()`: the target, from the package
    /// in the graph the last update kept.
    pub fn get_target(&self, label: &str) -> blaze_packages::target::Target {
        let label = blaze_cmdline::Label::parse_canonical(label).expect("a canonical label");
        let evaluator = self.t.get_kept_evaluator().expect("a kept graph");
        for (key, value) in evaluator.get_done_values() {
            if key.function_name() != blaze_sky_loading::sky_functions::PACKAGE {
                continue;
            }
            let Ok(package) = value
                .clone()
                .downcast::<blaze_sky_loading::package_function::PackageValue>()
            else {
                continue;
            };
            let package = package.get_package();
            if package.get_package_identifier() != label.get_package_identifier() {
                continue;
            }
            return package
                .get_target(label.get_name())
                .expect("the target")
                .clone();
        }
        panic!("no package of {label} in the graph")
    }

    /// `getConfiguredTarget(label).getProvider(FileProvider.class).getFilesToBuild()`.
    pub fn files_to_build(&self, label: &str) -> Vec<blaze_actions::Artifact> {
        let target = self.get_configured_target(label);
        target
            .get_provider::<FileProvider>()
            .expect("FileProvider")
            .get_files_to_build()
            .to_list()
            .to_vec()
    }
}

impl Fixture {
    /// `getConfiguredTarget(label).get(OutputGroupInfo.STARLARK_CONSTRUCTOR).getOutputGroup(name)`.
    pub fn output_group(&self, label: &str, name: &str) -> Vec<blaze_actions::Artifact> {
        use blaze_analysis::providers::output_group_info::OutputGroupInfo;
        use starlark::values::ValueLike as _;
        let target = self.get_configured_target(label);
        let info = target
            .get(&blaze_packages::starlark_defs::builtin_providers::OUTPUT_GROUP_INFO.key())
            .expect("OutputGroupInfo");
        info.value()
            .downcast_ref::<OutputGroupInfo>()
            .expect("an OutputGroupInfo")
            .get_output_group(name)
            .to_list()
            .to_vec()
    }

    /// `cleanSkyframe()`: drops the graph.
    // Java: src/test/java/com/google/devtools/build/lib/analysis/util/AnalysisTestCase.java cleanSkyframe
    pub fn clean_skyframe(&self) {
        self.t.get_blaze_workspace().reset_evaluator();
    }
}

/// `assertThat(actual).containsAtLeast(expected...)`.
pub fn assert_contains_at_least(actual: &[String], expected: &[&str]) {
    for e in expected {
        assert!(
            actual.iter().any(|a| a == e),
            "{e:?} missing from {actual:?}"
        );
    }
}

/// `assertThat(actual).containsNoneOf(unexpected...)`.
pub fn assert_contains_none_of(actual: &[String], unexpected: &[&str]) {
    for u in unexpected {
        assert!(
            !actual.iter().any(|a| a == u),
            "{u:?} present in {actual:?}"
        );
    }
}

/// Mirrors the tree `source` into `target`: directories as directories, files as symbolic
/// links.
fn mirror(source: &std::path::Path, target: &std::path::Path) {
    std::fs::create_dir_all(target).unwrap();
    for entry in std::fs::read_dir(source).unwrap() {
        let entry = entry.unwrap();
        let path = entry.path();
        let to = target.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            mirror(&path, &to);
        } else {
            std::os::unix::fs::symlink(&path, &to).unwrap();
        }
    }
}

/// `ActionsTestUtil.baseArtifactNames(artifacts)`.
// Java: src/test/java/com/google/devtools/build/lib/actions/util/ActionsTestUtil.java baseArtifactNames
pub fn base_artifact_names(artifacts: &[blaze_actions::Artifact]) -> Vec<String> {
    artifacts
        .iter()
        .map(|a| a.get_exec_path().get_base_name().to_owned())
        .collect()
}

/// `assertThat(actual).containsExactly(expected...)` (any order, same multiset).
pub fn assert_contains_exactly(actual: &[String], expected: &[&str]) {
    let mut a: Vec<&str> = actual.iter().map(|s| s.as_str()).collect();
    let mut e: Vec<&str> = expected.to_vec();
    a.sort();
    e.sort();
    assert_eq!(a, e, "actual {actual:?}, expected {expected:?}");
}
