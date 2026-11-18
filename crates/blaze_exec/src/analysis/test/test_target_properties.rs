//! Port of `analysis/test/TestTargetProperties.java`.
//!
//! PORT: the Java constructor reads the test rule (`TestSize.getTestSize(rule)`, the `tags` and
//! `flaky` attributes, `TargetUtils.getExecutionInfo`, ...) during analysis. The analysis crates
//! own rules, so this struct is built from those already-computed values; the execution-info and
//! remotability logic of the constructor that does not read the rule is ported in
//! [`TestTargetProperties::compute_execution_info`] and [`TestTargetProperties::new`].

use blaze_cmdline::Label;
use blaze_optdefs::common::test_size::TestSize;
use blaze_optdefs::common::test_timeout::TestTimeout;
use indexmap::IndexMap;

use crate::actions::ResourceSet;
use crate::actions::execution_requirements as er;

/// Container for test target properties available to the `TestRunnerAction` instance.
// Java: com/google/devtools/build/lib/analysis/test/TestTargetProperties.java:38 TestTargetProperties
#[derive(Clone, Debug, PartialEq)]
pub struct TestTargetProperties {
    size: TestSize,
    timeout: TestTimeout,
    tags: Vec<String>,
    is_remotable: bool,
    is_flaky: bool,
    is_external: bool,
    language: String,
    execution_info: IndexMap<String, String>,
    /// `testConfiguration.getTestResources(size)`, or `None` when the configuration has no
    /// `TestConfiguration` fragment (`testConfiguration == null`).
    test_resources: Option<IndexMap<String, f64>>,
}

/// Resources used by local tests of various sizes.
///
/// When changing these values, remember to update the documentation at
/// attributes/test/size.html.
// Java: com/google/devtools/build/lib/analysis/test/TestTargetProperties.java:46 TestTargetProperties.SMALL_RESOURCES
fn get_resource_set_from_size(size: TestSize) -> ResourceSet {
    // Java: com/google/devtools/build/lib/analysis/test/TestTargetProperties.java:53 TestTargetProperties.getResourceSetFromSize
    match size {
        TestSize::SMALL => ResourceSet::create(20.0, 1.0, 1),
        TestSize::MEDIUM => ResourceSet::create(100.0, 1.0, 1),
        TestSize::LARGE => ResourceSet::create(300.0, 1.0, 1),
        _ => ResourceSet::create(800.0, 1.0, 1),
    }
}

impl TestTargetProperties {
    /// The execution-info part of the Java constructor: `TargetUtils.getExecutionInfo(rule)`
    /// (`rule_execution_info`), then `local` / `no-remote-exec` / `no-testloasd` for local,
    /// exclusive and no-testloasd tests, then the rule's `ExecutionInfo` provider
    /// (`execution_requirements`, overwriting). The caller applies
    /// `BuildConfigurationValue.modifyExecutionInfo(map, "TestRunner")` afterwards.
    // Java: com/google/devtools/build/lib/analysis/test/TestTargetProperties.java:76 TestTargetProperties.<init>
    pub fn compute_execution_info(
        rule_execution_info: &IndexMap<String, String>,
        incompatible_exclusive_test_sandboxed: bool,
        is_local_test_rule: bool,
        is_exclusive_test_rule: bool,
        is_no_testloasd_test_rule: bool,
        execution_requirements: Option<&IndexMap<String, String>>,
    ) -> IndexMap<String, String> {
        let mut execution_info: IndexMap<String, String> = IndexMap::new();
        execution_info.extend(
            rule_execution_info
                .iter()
                .map(|(k, v)| (k.clone(), v.clone())),
        );

        if incompatible_exclusive_test_sandboxed {
            if is_local_test_rule {
                execution_info.insert(er::LOCAL.to_string(), String::new());
            } else if is_exclusive_test_rule {
                execution_info.insert(er::NO_REMOTE_EXEC.to_string(), String::new());
            }
        } else if is_local_test_rule || is_exclusive_test_rule {
            execution_info.insert(er::LOCAL.to_string(), String::new());
        }

        if is_no_testloasd_test_rule {
            execution_info.insert(er::LOCAL.to_string(), String::new());
            execution_info.insert(er::NO_TESTLOASD.to_string(), String::new());
        }

        if let Some(execution_requirements) = execution_requirements {
            // This will overwrite whatever TargetUtils put there, which might be confusing.
            for (k, v) in execution_requirements {
                execution_info.insert(k.clone(), v.clone());
            }
        }
        execution_info
    }

    /// Creates test target properties instance from the values the Java constructor reads off the
    /// rule. `execution_info` is the final map (see [`Self::compute_execution_info`], after
    /// `modifyExecutionInfo`); `test_resources` is `testConfiguration.getTestResources(size)`.
    // Java: com/google/devtools/build/lib/analysis/test/TestTargetProperties.java:76 TestTargetProperties.<init>
    pub fn new(
        size: TestSize,
        timeout: TestTimeout,
        tags: Vec<String>,
        is_flaky: bool,
        is_external: bool,
        language: String,
        execution_info: IndexMap<String, String>,
        test_resources: Option<IndexMap<String, f64>>,
    ) -> TestTargetProperties {
        let is_remotable = !execution_info.contains_key(er::LOCAL)
            && !execution_info.contains_key(er::NO_REMOTE)
            && !execution_info.contains_key(er::NO_REMOTE_EXEC);
        TestTargetProperties {
            size,
            timeout,
            tags,
            is_remotable,
            is_flaky,
            is_external,
            language,
            execution_info,
            test_resources,
        }
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetProperties.java:130 TestTargetProperties.getSize
    pub fn get_size(&self) -> TestSize {
        self.size
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetProperties.java:134 TestTargetProperties.getTimeout
    pub fn get_timeout(&self) -> TestTimeout {
        self.timeout
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetProperties.java:138 TestTargetProperties.getTags
    pub fn get_tags(&self) -> &[String] {
        &self.tags
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetProperties.java:142 TestTargetProperties.isRemotable
    pub fn is_remotable(&self) -> bool {
        self.is_remotable
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetProperties.java:146 TestTargetProperties.isFlaky
    pub fn is_flaky(&self) -> bool {
        self.is_flaky
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetProperties.java:150 TestTargetProperties.isExternal
    pub fn is_external(&self) -> bool {
        self.is_external
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetProperties.java:154 TestTargetProperties.getLocalResourceUsage
    pub fn get_local_resource_usage(
        &self,
        _label: &Label,
        using_local_test_jobs: bool,
    ) -> ResourceSet {
        if using_local_test_jobs {
            // Java: TestTargetProperties.LOCAL_TEST_JOBS_BASED_RESOURCES
            return ResourceSet::create_with_local_test_count(1);
        }

        let default_resources = get_resource_set_from_size(self.size);
        let config_resources = self.test_resources.clone().unwrap_or_default();
        default_resources.with_resource_overrides(&[config_resources])
    }

    /// Returns a map of execution info. See `Spawn.getExecutionInfo`.
    // Java: com/google/devtools/build/lib/analysis/test/TestTargetProperties.java:170 TestTargetProperties.getExecutionInfo
    pub fn get_execution_info(&self) -> &IndexMap<String, String> {
        &self.execution_info
    }

    // Java: com/google/devtools/build/lib/analysis/test/TestTargetProperties.java:174 TestTargetProperties.getLanguage
    pub fn get_language(&self) -> &str {
        &self.language
    }
}
