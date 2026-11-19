//! `BlazeVersionInfo` (the version information of the current process) and the build data that
//! Bazel's `main` reads from `build-data.properties` (`Bazel.tryGetBuildInfo`).

use std::collections::BTreeMap;
use std::sync::Arc;

use parking_lot::Mutex;

/// The content of `build-data.properties` as embedded in the oracle's server jar
/// (`<install_base>/A-server.jar`), decoded the way `java.util.Properties.load` decodes it
/// (`\:` -> `:`), in file order.
///
/// PORT: Blaze impersonates Bazel 9.3.2 (an explicit decision of the crate lead), so these are
/// the oracle's values, not data about the Blaze build. Bazel's build stamps them into the jar.
pub const BUILD_DATA_PROPERTIES: &[(&str, &str)] = &[
    (
        "build.target",
        "@@//src/main/java/com/google/devtools/build/lib/bazel:BazelServer",
    ),
    ("main.class", "com.google.devtools.build.lib.bazel.Bazel"),
    ("build.label", "9.3.2"),
    ("build.time", "Wed Nov 04 22:18:38 2026 (1793830718)"),
    ("build.timestamp", "1793830718"),
    ("build.timestamp.as.int", "1793830718"),
];

/// Builds the standard build info map from the loaded properties. The returned value is the list
/// of "build.*" properties from the build-data.properties file. The final key is the original one
/// striped, dot replaced with a space and with first letter capitalized.
///
/// PORT: the properties come from [`BUILD_DATA_PROPERTIES`] instead of a jar resource, so the
/// "file fails to load" branch (empty map) cannot happen. Java's `ImmutableMap` iteration order
/// (the `Properties` hash order) is not observable: `BlazeVersionInfo` sorts the keys.
// Java: com/google/devtools/build/lib/bazel/Bazel.java:119 Bazel.tryGetBuildInfo
pub fn try_get_build_info() -> BTreeMap<String, String> {
    let mut build_data = BTreeMap::new();
    for (string_key, value) in BUILD_DATA_PROPERTIES {
        if string_key.starts_with("build.") {
            // build.label -> Build label, build.timestamp.as.int -> Build timestamp as int
            let build_data_key = format!("B{}", string_key[1..].replace('.', " "));
            // ImmutableMap.Builder.buildOrThrow fails on duplicate keys.
            if build_data
                .insert(build_data_key.clone(), value.to_string())
                .is_some()
            {
                panic!(
                    "IllegalArgumentException: Multiple entries with same key: {}",
                    build_data_key
                );
            }
        }
    }
    build_data
}

/// Determines the version information of the current process.
///
/// The version information is a dictionary mapping from string keys to string values. For build
/// stamping, it should have the key "Build label", which contains among others a
/// XXXXXXX-YYYY.MM.DD string to indicate the version of the release. If no data is available
/// (eg. when running non-released version), [`BlazeVersionInfo::is_available`] returns false.
// Java: com/google/devtools/build/lib/analysis/BlazeVersionInfo.java:33 BlazeVersionInfo
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BlazeVersionInfo {
    build_data: BTreeMap<String, String>,
}

static INSTANCE: Mutex<Option<Arc<BlazeVersionInfo>>> = Mutex::new(None);

impl BlazeVersionInfo {
    // Java: com/google/devtools/build/lib/analysis/BlazeVersionInfo.java:34 BlazeVersionInfo.BUILD_LABEL
    pub const BUILD_LABEL: &'static str = "Build label";

    /// Key for the release timestamp is seconds.
    // Java: com/google/devtools/build/lib/analysis/BlazeVersionInfo.java:36 BlazeVersionInfo.BUILD_TIMESTAMP
    pub const BUILD_TIMESTAMP: &'static str = "Build timestamp as int";

    /// If the current version is a development version, this environment variable can be used to
    /// override the version string (e.g. to deal with version-based feature detection during a
    /// bisect).
    // Java: com/google/devtools/build/lib/analysis/BlazeVersionInfo.java:41 BlazeVersionInfo.BAZEL_DEV_VERSION_OVERRIDE_ENV_VAR
    pub const BAZEL_DEV_VERSION_OVERRIDE_ENV_VAR: &'static str = "BAZEL_DEV_VERSION_OVERRIDE";

    // Java: com/google/devtools/build/lib/analysis/BlazeVersionInfo.java:49 BlazeVersionInfo.<init>
    pub fn new<K: AsRef<str>, V: AsRef<str>>(
        info: impl IntoIterator<Item = (K, V)>,
    ) -> BlazeVersionInfo {
        let mut build_data = BTreeMap::new();
        for (k, v) in info {
            build_data.insert(k.as_ref().to_string(), v.as_ref().to_string());
        }
        BlazeVersionInfo { build_data }
    }

    /// Accessor method for BlazeVersionInfo singleton.
    ///
    /// If setBuildInfo was not called, returns an empty BlazeVersionInfo instance, which should
    /// not be persisted.
    // Java: com/google/devtools/build/lib/analysis/BlazeVersionInfo.java:59 BlazeVersionInfo.instance
    pub fn instance() -> Arc<BlazeVersionInfo> {
        let instance = INSTANCE.lock();
        match &*instance {
            None => Arc::new(BlazeVersionInfo::new(Vec::<(String, String)>::new())),
            Some(i) => i.clone(),
        }
    }

    /// PORT: Java logs the summary to the server log (`logVersionInfo`); Blaze has no server
    /// logger, so nothing is logged.
    // Java: com/google/devtools/build/lib/analysis/BlazeVersionInfo.java:66 BlazeVersionInfo.logVersionInfo
    fn log_version_info(_info: &BlazeVersionInfo) {}

    /// Sets build info.
    ///
    /// This should be called once in the program execution, as early soon as possible, so we can
    /// have the version information even before modules are initialized.
    // Java: com/google/devtools/build/lib/analysis/BlazeVersionInfo.java:80 BlazeVersionInfo.setBuildInfo
    pub fn set_build_info(info: BTreeMap<String, String>) {
        let mut instance = INSTANCE.lock();
        if instance.is_some() {
            panic!("IllegalStateException: setBuildInfo called twice.");
        }
        let new_instance = Arc::new(BlazeVersionInfo::new(info));
        Self::log_version_info(&new_instance);
        *instance = Some(new_instance);
    }

    /// Indicates whether version information is available.
    // Java: com/google/devtools/build/lib/analysis/BlazeVersionInfo.java:91 BlazeVersionInfo.isAvailable
    pub fn is_available(&self) -> bool {
        !self.build_data.is_empty()
    }

    /// Returns the summary which gets displayed in the 'version' command. The summary is a list of
    /// formatted key / value pairs.
    // Java: com/google/devtools/build/lib/analysis/BlazeVersionInfo.java:100 BlazeVersionInfo.getSummary
    pub fn get_summary(&self) -> Option<String> {
        if self.build_data.is_empty() {
            return None;
        }
        Some(
            self.build_data
                .iter()
                .map(|(k, v)| format!("{}: {}", k, v))
                .collect::<Vec<_>>()
                .join("\n"),
        )
    }

    /// Returns true iff this binary is released--that is, a binary built with a release label.
    // Java: com/google/devtools/build/lib/analysis/BlazeVersionInfo.java:113 BlazeVersionInfo.isReleasedBlaze
    pub fn is_released_blaze(&self) -> bool {
        self.build_data
            .get(Self::BUILD_LABEL)
            .is_some_and(|l| !l.is_empty())
    }

    /// Returns the release label, if any, or "development version".
    // Java: com/google/devtools/build/lib/analysis/BlazeVersionInfo.java:121 BlazeVersionInfo.getReleaseName
    pub fn get_release_name(&self) -> String {
        match self.build_data.get(Self::BUILD_LABEL) {
            Some(build_label) if !build_label.is_empty() => format!("release {}", build_label),
            _ => "development version".to_string(),
        }
    }

    /// Returns the version, if any, or `""`. The returned version number is easier to process
    /// than the version returned by `get_release_name()`.
    // Java: com/google/devtools/build/lib/analysis/BlazeVersionInfo.java:132 BlazeVersionInfo.getVersion
    pub fn get_version(&self) -> String {
        if let Some(build_label) = self.build_data.get(Self::BUILD_LABEL) {
            return build_label.clone();
        }
        // PORT: System.getenv reads the server's environment; Blaze's is the process environment.
        if let Some(over) = std::env::var_os(Self::BAZEL_DEV_VERSION_OVERRIDE_ENV_VAR) {
            return blaze_util::string_encoding::platform_to_internal(&over).into_owned();
        }
        String::new()
    }

    /// Returns the release timestamp in seconds.
    ///
    /// (Java returns `new Date().getTime()`, i.e. milliseconds, when the timestamp is missing or
    /// "0"; kept as is.)
    // Java: com/google/devtools/build/lib/analysis/BlazeVersionInfo.java:147 BlazeVersionInfo.getTimestamp
    pub fn get_timestamp(&self) -> i64 {
        match self.build_data.get(Self::BUILD_TIMESTAMP) {
            None => now_millis(),
            Some(t) if t == "0" => now_millis(),
            Some(t) => t
                .parse::<i64>()
                .unwrap_or_else(|_| panic!("NumberFormatException: For input string: \"{}\"", t)),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/BlazeVersionInfo.java:156 BlazeVersionInfo.getBuildData
    pub fn get_build_data(&self) -> &BTreeMap<String, String> {
        &self.build_data
    }
}

fn now_millis() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_millis() as i64)
        .unwrap_or(0)
}

#[cfg(test)]
mod tests {
    use super::*;

    // Java: src/test/java/com/google/devtools/build/lib/runtime/BlazeVersionInfoTest.java
    #[test]
    fn test_empty_version_info_means_not_available() {
        let info = BlazeVersionInfo::new(Vec::<(String, String)>::new());
        assert!(!info.is_available());
        assert_eq!(info.get_summary(), None);
        assert_eq!(info.get_release_name(), "development version");
    }

    #[test]
    fn test_release_name_is_development_if_build_label_is_null() {
        let info = BlazeVersionInfo::new([("Build label", "")]);
        assert_eq!(info.get_release_name(), "development version");
    }

    #[test]
    fn test_release_name_if_build_label_is_present() {
        let info = BlazeVersionInfo::new([("Build label", "3/4/2009 (gold)")]);
        assert_eq!(info.get_release_name(), "release 3/4/2009 (gold)");
    }

    #[test]
    fn test_get_summary_returns_ordered_tablified_data() {
        let info = BlazeVersionInfo::new([("key3", "foo"), ("key2", "bar"), ("key1", "baz")]);
        assert_eq!(
            info.get_summary().as_deref(),
            Some("key1: baz\nkey2: bar\nkey3: foo")
        );
    }

    #[test]
    fn test_version_is_head_if_build_label_is_null() {
        let info = BlazeVersionInfo::new(Vec::<(String, String)>::new());
        // PORT: Java's test assumes BAZEL_DEV_VERSION_OVERRIDE is unset.
        if std::env::var(BlazeVersionInfo::BAZEL_DEV_VERSION_OVERRIDE_ENV_VAR).is_err() {
            assert_eq!(info.get_version(), "");
        }
    }

    #[test]
    fn test_versions_i_if_build_label_is_present() {
        let info = BlazeVersionInfo::new([("Build label", "123.4")]);
        assert_eq!(info.get_version(), "123.4");
    }

    /// Expected output of the oracle's `bazel version` (Bazel 9.3.2).
    #[test]
    fn oracle_build_info_summary() {
        let info = BlazeVersionInfo::new(try_get_build_info());
        assert_eq!(
            info.get_summary().as_deref(),
            Some(
                "Build label: 9.3.2\n\
                 Build target: @@//src/main/java/com/google/devtools/build/lib/bazel:BazelServer\n\
                 Build time: Wed Nov 04 22:18:38 2026 (1793830718)\n\
                 Build timestamp: 1793830718\n\
                 Build timestamp as int: 1793830718"
            )
        );
        assert_eq!(info.get_release_name(), "release 9.3.2");
        assert!(info.is_released_blaze());
        assert_eq!(info.get_timestamp(), 1793830718);
    }
}
