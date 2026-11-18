//! Test doubles shared by the ported runtime tests: Java's tests mock `ConfiguredTarget`,
//! `BuildConfigurationValue` and `AnsiTerminalPrinter` with Mockito.

#![allow(dead_code)]

use std::any::Any;
use std::collections::hash_map::DefaultHasher;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use blaze_actions::{ActionLookupKey, ActionLookupKeyRef, Artifact};
use blaze_buildtool::runtime::test_summary::ConfigurationView;
use blaze_buildtool::runtime::test_summary_printer::TerminalPrinter;
use blaze_cmdline::Label;
use blaze_collect::NestedSet;
use blaze_events::{ByteArrayOutputStream, OutputStream};
use blaze_optdefs::common::test_timeout::TestTimeout;
use blaze_sky_exec::analysis_seam::{ConfiguredTargetView, TestParams};
use indexmap::IndexMap;
use parking_lot::Mutex;

/// A `ConfiguredTargetKey` stand-in: label plus configuration checksum.
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct StubKey {
    pub label: Label,
    pub checksum: String,
}

impl fmt::Debug for StubKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "StubKey({}, {})", self.label, self.checksum)
    }
}

impl fmt::Display for StubKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl ActionLookupKey for StubKey {
    fn get_label(&self) -> Option<Label> {
        Some(self.label.clone())
    }
    fn get_configuration_checksum(&self) -> Option<String> {
        Some(self.checksum.clone())
    }
    fn function_name(&self) -> &'static str {
        "CONFIGURED_TARGET"
    }
    fn key_equals(&self, other: &dyn ActionLookupKey) -> bool {
        other
            .as_any()
            .downcast_ref::<StubKey>()
            .is_some_and(|o| o == self)
    }
    fn key_hash(&self) -> u64 {
        let mut h = DefaultHasher::new();
        self.hash(&mut h);
        h.finish()
    }
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// A mocked test `ConfiguredTarget` with a `TestProvider`.
#[derive(Debug)]
pub struct StubTarget {
    pub label: Label,
    pub checksum: String,
    pub shards: i32,
    pub runs: i32,
    pub runs_detects_flakes: bool,
    pub timeout: TestTimeout,
    pub test_status_artifacts: Vec<Artifact>,
}

impl ConfiguredTargetView for StubTarget {
    // PORT (harness/test): the target has no BUILD file; a fixed location.
    fn get_location(&self) -> blaze_events::location::Location {
        blaze_events::location::Location::from_file_line_column("/BUILD", 1, 1)
    }

    fn get_label(&self) -> Label {
        self.label.clone()
    }
    fn get_configuration_checksum(&self) -> Option<String> {
        Some(self.checksum.clone())
    }
    fn get_lookup_key(&self) -> ActionLookupKeyRef {
        ActionLookupKeyRef::new(StubKey {
            label: self.label.clone(),
            checksum: self.checksum.clone(),
        })
    }
    fn get_rule_class_or_kind(&self) -> String {
        "cc_test".to_string()
    }
    fn get_output_groups(&self) -> IndexMap<String, NestedSet<Artifact>> {
        IndexMap::new()
    }
    fn get_executable(&self) -> Option<Artifact> {
        None
    }
    fn get_test_params(&self) -> Option<TestParams> {
        Some(TestParams {
            runs: self.runs,
            shards: self.shards,
            runs_detects_flakes: self.runs_detects_flakes,
            timeout: self.timeout,
            test_rule_class: "cc_test".to_string(),
            test_status_artifacts: self.test_status_artifacts.clone(),
        })
    }
}

/// `target(path, targetName)` of the Java tests: a mock whose `TestParams.getShards()` is 1 and
/// whose other `TestParams` getters return Mockito's defaults.
pub fn target(path: &str, target_name: &str) -> Arc<dyn ConfiguredTargetView> {
    Arc::new(StubTarget {
        label: Label::create(path, target_name).unwrap(),
        checksum: "abcdef".to_string(),
        shards: 1,
        runs: 0,
        runs_detects_flakes: false,
        timeout: TestTimeout::SHORT,
        test_status_artifacts: Vec::new(),
    })
}

/// A mocked `BuildConfigurationValue`.
#[derive(Debug)]
pub struct StubConfiguration {
    pub checksum: String,
    pub mnemonic: String,
}

impl ConfigurationView for StubConfiguration {
    fn checksum(&self) -> String {
        self.checksum.clone()
    }
    fn get_mnemonic(&self) -> String {
        self.mnemonic.clone()
    }
}

pub fn configuration() -> Arc<dyn ConfigurationView> {
    Arc::new(StubConfiguration {
        checksum: "abcdef".to_string(),
        mnemonic: "k8-fastbuild".to_string(),
    })
}

/// One call on the mocked printer.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum PrinterCall {
    Print(String),
    PrintLn(String),
}

/// A Mockito mock of `AnsiTerminalPrinter`: records the raw strings.
pub struct RecordingPrinter {
    pub calls: Mutex<Vec<PrinterCall>>,
    pub stream: Arc<ByteArrayOutputStream>,
}

impl RecordingPrinter {
    pub fn new() -> Arc<RecordingPrinter> {
        Arc::new(RecordingPrinter {
            calls: Mutex::new(Vec::new()),
            stream: Arc::new(ByteArrayOutputStream::new()),
        })
    }

    pub fn prints(&self) -> Vec<String> {
        self.calls
            .lock()
            .iter()
            .filter_map(|c| match c {
                PrinterCall::Print(s) => Some(s.clone()),
                _ => None,
            })
            .collect()
    }

    pub fn print_lns(&self) -> Vec<String> {
        self.calls
            .lock()
            .iter()
            .filter_map(|c| match c {
                PrinterCall::PrintLn(s) => Some(s.clone()),
                _ => None,
            })
            .collect()
    }

    /// `verify(printer).print(argThat(matcher))`: exactly one `print` call matches.
    pub fn verify_print(&self, matcher: impl Fn(&str) -> bool) {
        let prints = self.prints();
        let n = prints.iter().filter(|s| matcher(s)).count();
        assert_eq!(n, 1, "expected exactly one matching print in {:?}", prints);
    }

    /// `verify(printer, never()).print(anyString())`.
    pub fn verify_never_print(&self) {
        assert!(self.prints().is_empty(), "{:?}", self.prints());
    }
}

impl TerminalPrinter for RecordingPrinter {
    fn print(&self, str: &str) {
        self.calls.lock().push(PrinterCall::Print(str.to_string()));
    }
    fn print_ln(&self, str: &str) {
        self.calls
            .lock()
            .push(PrinterCall::PrintLn(str.to_string()));
    }
    fn get_output_stream(&self) -> Arc<dyn OutputStream> {
        self.stream.clone()
    }
}

/// Mockito's `find(regex)` matcher.
pub fn find(regex: &str) -> impl Fn(&str) -> bool {
    let re = regex::Regex::new(regex).unwrap();
    move |s| re.is_match(s)
}

/// Mockito's `contains(substring)` matcher.
pub fn contains(substring: &str) -> impl Fn(&str) -> bool {
    let substring = substring.to_string();
    move |s| s.contains(&substring)
}
