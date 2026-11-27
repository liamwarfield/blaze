//! Port of `lib/bazel/BazelWorkspaceStatusModule.java`: `BazelWorkspaceStatusAction` (the
//! contents of `stable-status.txt` and `volatile-status.txt`) and `BazelStatusActionFactory`.

use std::any::Any;
use std::collections::BTreeMap;
use std::sync::Arc;

use blaze_collect::{NestedSetBuilder, Order};
use blaze_util::Fingerprint;
use indexmap::IndexMap;

use crate::actions::action::{AbstractAction, Action, ComputeKeyError, InputMetadataProvider};
use crate::actions::action_key_context::ActionKeyContext;
use crate::actions::artifact::Artifact;
use crate::analysis::workspace_status_action::{
    WorkspaceStatusAction, WorkspaceStatusActionEnvironment, WorkspaceStatusActionFactory,
    new_workspace_status_action, system_action_owner,
};

// Java: com/google/devtools/build/lib/analysis/BuildInfo.java:25 BuildInfo.BUILD_EMBED_LABEL
pub const BUILD_EMBED_LABEL: &str = "BUILD_EMBED_LABEL";
// Java: com/google/devtools/build/lib/analysis/BuildInfo.java:30 BuildInfo.BUILD_USER
pub const BUILD_USER: &str = "BUILD_USER";
// Java: com/google/devtools/build/lib/analysis/BuildInfo.java:35 BuildInfo.BUILD_HOST
pub const BUILD_HOST: &str = "BUILD_HOST";
// Java: com/google/devtools/build/lib/analysis/BuildInfo.java:40 BuildInfo.BUILD_TIMESTAMP
pub const BUILD_TIMESTAMP: &str = "BUILD_TIMESTAMP";

// Java: com/google/devtools/build/lib/bazel/BazelWorkspaceStatusModule.java:132 BazelWorkspaceStatusAction.SPECIAL_STABLE_KEYS
const SPECIAL_STABLE_KEYS: [&str; 3] = [BUILD_EMBED_LABEL, BUILD_HOST, BUILD_USER];

/// The inputs of `BazelWorkspaceStatusAction.execute` that come from the environment.
///
/// PORT: Java reads these from the action context (`WorkspaceStatusAction.Context`) and the
/// system clock; they are parameters here so that the file contents can be computed (and tested)
/// without the execution layer.
#[derive(Clone, Debug, Default)]
pub struct WorkspaceStatusInputs {
    /// `--embed_label`.
    pub embed_label: String,
    /// The client environment (`Context.getClientEnv()`), read for `SOURCE_DATE_EPOCH`.
    pub client_env: IndexMap<String, String>,
    /// `System.currentTimeMillis()`.
    pub current_time_millis: i64,
    /// The standard output of `--workspace_status_command`, or `""` when no command is set. A
    /// Unicode string, not an internal one: Java decodes it with `stdoutStream.toString(UTF_8)`
    /// (`blaze_util::string_encoding::java_new_string_utf8`), and the status files write it back
    /// with `getBytes(UTF_8)`, so its non-ASCII text round-trips unchanged.
    pub workspace_status_command_output: String,
}

/// The workspace status action of Bazel.
// Java: com/google/devtools/build/lib/bazel/BazelWorkspaceStatusModule.java:74 BazelWorkspaceStatusModule.BazelWorkspaceStatusAction
#[derive(Debug)]
pub struct BazelWorkspaceStatusAction {
    base: AbstractAction,
    stable_status: Artifact,
    volatile_status: Artifact,
    username: String,
    hostname: String,
}

impl BazelWorkspaceStatusAction {
    // Java: com/google/devtools/build/lib/bazel/BazelWorkspaceStatusModule.java:87 BazelWorkspaceStatusAction.<init>
    pub fn new(
        stable_status: Artifact,
        volatile_status: Artifact,
        username: impl Into<String>,
        hostname: impl Into<String>,
    ) -> BazelWorkspaceStatusAction {
        BazelWorkspaceStatusAction {
            base: new_workspace_status_action(
                system_action_owner(),
                NestedSetBuilder::empty_set(Order::STABLE_ORDER),
                vec![stable_status.clone(), volatile_status.clone()],
            ),
            stable_status,
            volatile_status,
            username: username.into(),
            hostname: hostname.into(),
        }
    }

    /// The parts of `execute` that compute the two files: returns the contents of
    /// `stable-status.txt` and `volatile-status.txt`, and the overall map posted as the
    /// `BuildInfoEvent`.
    ///
    /// PORT: running `--workspace_status_command` (`getAdditionalWorkspaceStatus`), posting the
    /// event and writing the files are execution (wave W6); the command's output comes in
    /// through `inputs`.
    // Java: com/google/devtools/build/lib/bazel/BazelWorkspaceStatusModule.java:174 BazelWorkspaceStatusAction.execute
    pub fn compute_status_files(&self, inputs: &WorkspaceStatusInputs) -> WorkspaceStatusFiles {
        let mut volatile_map: StatusMap = BTreeMap::new();
        let mut stable_map: StatusMap = BTreeMap::new();

        stable_map.insert(
            JavaStringKey(BUILD_EMBED_LABEL.to_string()),
            inputs.embed_label.clone(),
        );
        stable_map.insert(JavaStringKey(BUILD_HOST.to_string()), self.hostname.clone());
        stable_map.insert(JavaStringKey(BUILD_USER.to_string()), self.username.clone());
        let current_time_millis =
            get_current_time_millis(&inputs.client_env, inputs.current_time_millis);
        volatile_map.insert(
            JavaStringKey(BUILD_TIMESTAMP.to_string()),
            (current_time_millis / 1000).to_string(),
        );
        volatile_map.insert(
            JavaStringKey("FORMATTED_DATE".to_string()),
            format(current_time_millis / 1000 * 1000),
        );
        let status_map = parse_workspace_status(&inputs.workspace_status_command_output);
        for (key, value) in status_map {
            if is_stable_key(key.as_str()) {
                stable_map.insert(key, value);
            } else {
                volatile_map.insert(key, value);
            }
        }

        let mut overall_map = BTreeMap::new();
        overall_map.extend(volatile_map.iter().map(|(k, v)| (k.0.clone(), v.clone())));
        overall_map.extend(stable_map.iter().map(|(k, v)| (k.0.clone(), v.clone())));

        WorkspaceStatusFiles {
            stable_status: print_status_map(&stable_map),
            volatile_status: print_status_map(&volatile_map),
            overall_map,
        }
    }

    // `execute` (running `--workspace_status_command` and writing the files) is ported with the
    // execution: blaze_exec `analysis::actions::bazel_workspace_status_module`.
}

/// What `BazelWorkspaceStatusAction.execute` writes and posts.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkspaceStatusFiles {
    /// The bytes of `stable-status.txt`.
    pub stable_status: Vec<u8>,
    /// The bytes of `volatile-status.txt`.
    pub volatile_status: Vec<u8>,
    /// The `BuildInfoEvent` map (volatile entries overridden by stable ones).
    // PORT: keyed by `String` for `BuildInfoEvent` (blaze_exec); its iteration order is code
    // point order, Java's TreeMap's UTF-16 order differs only for keys with chars above U+FFFF.
    pub overall_map: BTreeMap<String, String>,
}

/// `DateTimeFormatter.ofPattern("yyyy MMM dd HH mm ss EEE")` applied to the UTC time.
// Java: com/google/devtools/build/lib/bazel/BazelWorkspaceStatusModule.java:83 BazelWorkspaceStatusAction.format
// PORT: Java formats with the JVM's default locale; the Bazel server runs with an English locale
// (month and weekday abbreviations below).
pub fn format(timestamp: i64) -> String {
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    // 1970-01-01 was a Thursday.
    const WEEKDAYS: [&str; 7] = ["Thu", "Fri", "Sat", "Sun", "Mon", "Tue", "Wed"];
    let secs = timestamp.div_euclid(1000);
    let days = secs.div_euclid(86400);
    let sod = secs.rem_euclid(86400);
    let (year, month, day) = civil_from_days(days);
    let weekday = WEEKDAYS[days.rem_euclid(7) as usize];
    format!(
        "{:04} {} {:02} {:02} {:02} {:02} {}",
        year,
        MONTHS[(month - 1) as usize],
        day,
        sod / 3600,
        (sod % 3600) / 60,
        sod % 60,
        weekday
    )
}

/// Days since 1970-01-01 to (year, month, day) in the proleptic Gregorian calendar.
fn civil_from_days(z: i64) -> (i64, u32, u32) {
    let z = z + 719468;
    let era = z.div_euclid(146097);
    let doe = z.rem_euclid(146097);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    (if m <= 2 { y + 1 } else { y }, m, d)
}

// Java: com/google/devtools/build/lib/bazel/BazelWorkspaceStatusModule.java:135 BazelWorkspaceStatusAction.isStableKey
fn is_stable_key(key: &str) -> bool {
    key.starts_with("STABLE_") || SPECIAL_STABLE_KEYS.contains(&key)
}

/// Java `String.trim()`: strips leading and trailing chars `<= ' '`.
fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

/// A key of Java's `TreeMap<String, String>`: ordered by `String.compareTo` (UTF-16 code units).
/// `str` order (code points) differs between a char above U+FFFF and one in U+E000..U+FFFF, and
/// the command's output is a Unicode string (`stdoutStream.toString(UTF_8)`).
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct JavaStringKey(pub String);

impl JavaStringKey {
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl Ord for JavaStringKey {
    // Java: java/lang/String.java String.compareTo
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        self.0.encode_utf16().cmp(other.0.encode_utf16())
    }
}

impl PartialOrd for JavaStringKey {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
}

/// Java `TreeMap<String, String>` of a status file.
pub type StatusMap = BTreeMap<JavaStringKey, String>;

/// Parses the output of `--workspace_status_command`.
// Java: com/google/devtools/build/lib/bazel/BazelWorkspaceStatusModule.java:139 BazelWorkspaceStatusAction.parseWorkspaceStatus
pub fn parse_workspace_status(input: &str) -> StatusMap {
    let mut result = BTreeMap::new();
    // `input.trim().split("\n")`: Java's split drops trailing empty strings, which have fewer
    // than two parts and are skipped below anyway.
    for line in java_trim(input).split('\n') {
        // `line.split(" ", 2)`
        let split_line: Vec<&str> = line.splitn(2, ' ').collect();
        if split_line.len() >= 2 {
            result.insert(
                JavaStringKey(split_line[0].to_string()),
                java_trim(split_line[1]).to_string(),
            );
        }
    }
    result
}

// Java: com/google/devtools/build/lib/bazel/BazelWorkspaceStatusModule.java:151 BazelWorkspaceStatusAction.printStatusMap
fn print_status_map(map: &StatusMap) -> Vec<u8> {
    let mut s = map
        .iter()
        .map(|(k, v)| format!("{} {}", k.as_str(), v))
        .collect::<Vec<_>>()
        .join("\n");
    s.push('\n');
    // Java: `s.getBytes(UTF_8)`. The keys and values from the command are Unicode; the host and
    // user names are internal strings (platform strings), so their non-ASCII bytes come out
    // encoded twice, as in Java.
    s.into_bytes()
}

/// This method returns the current time for stamping, using SOURCE_DATE_EPOCH
/// (https://reproducible-builds.org/specs/source-date-epoch/) if provided.
// Java: com/google/devtools/build/lib/bazel/BazelWorkspaceStatusModule.java:231 BazelWorkspaceStatusAction.getCurrentTimeMillis
// PORT: `System.currentTimeMillis()` is the `current_time_millis` parameter.
pub fn get_current_time_millis(
    client_env: &IndexMap<String, String>,
    current_time_millis: i64,
) -> i64 {
    if let Some(value) = client_env.get("SOURCE_DATE_EPOCH") {
        let value = java_trim(value);
        if !value.is_empty() {
            // `Long.parseLong`: an optional sign and decimal digits.
            if let Some(v) = parse_java_long(value) {
                return v.wrapping_mul(1000);
            }
            // Fall-back to use the current time if SOURCE_DATE_EPOCH is not a long.
        }
    }
    current_time_millis
}

/// `Long.parseLong(s)`, `None` for `NumberFormatException`.
fn parse_java_long(s: &str) -> Option<i64> {
    let digits = s.strip_prefix(['+', '-']).unwrap_or(s);
    if digits.is_empty() || !digits.bytes().all(|b| b.is_ascii_digit()) {
        return None;
    }
    s.parse::<i64>().ok()
}

impl Action for BazelWorkspaceStatusAction {
    fn abstract_action(&self) -> &AbstractAction {
        &self.base
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    // Java: com/google/devtools/build/lib/bazel/BazelWorkspaceStatusModule.java:246 BazelWorkspaceStatusAction.getMnemonic
    fn get_mnemonic(&self) -> String {
        "BazelWorkspaceStatusAction".to_string()
    }

    // Java: com/google/devtools/build/lib/analysis/WorkspaceStatusAction.java:197 WorkspaceStatusAction.computeKey
    fn compute_key(
        &self,
        _action_key_context: &ActionKeyContext,
        _input_metadata_provider: Option<&dyn InputMetadataProvider>,
        _fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        // Since executeUnconditionally() is true (and this action is special-cased anyway),
        // there is no point in calculating a fingerprint.
        Ok(())
    }

    // Java: com/google/devtools/build/lib/analysis/WorkspaceStatusAction.java:187 WorkspaceStatusAction.executeUnconditionally
    fn execute_unconditionally(&self) -> bool {
        true
    }

    // Java: com/google/devtools/build/lib/analysis/WorkspaceStatusAction.java:192 WorkspaceStatusAction.isVolatile
    fn is_volatile(&self) -> bool {
        true
    }
}

impl WorkspaceStatusAction for BazelWorkspaceStatusAction {
    // Java: com/google/devtools/build/lib/bazel/BazelWorkspaceStatusModule.java:251 BazelWorkspaceStatusAction.getVolatileStatus
    fn get_volatile_status(&self) -> Artifact {
        self.volatile_status.clone()
    }

    // Java: com/google/devtools/build/lib/bazel/BazelWorkspaceStatusModule.java:256 BazelWorkspaceStatusAction.getStableStatus
    fn get_stable_status(&self) -> Artifact {
        self.stable_status.clone()
    }

    fn get_workspace_status_description(&self) -> &str {
        "workspace status"
    }
}

/// Bazel's `WorkspaceStatusAction.Factory`.
// Java: com/google/devtools/build/lib/bazel/BazelWorkspaceStatusModule.java:269 BazelWorkspaceStatusModule.BazelStatusActionFactory
#[derive(Clone, Debug)]
pub struct BazelStatusActionFactory {
    /// `StandardSystemProperty.USER_NAME.value()`.
    pub username: String,
    /// `NetUtil.getCachedShortHostName()`.
    pub hostname: String,
}

impl WorkspaceStatusActionFactory for BazelStatusActionFactory {
    // Java: com/google/devtools/build/lib/bazel/BazelWorkspaceStatusModule.java:278 BazelStatusActionFactory.createWorkspaceStatusAction
    fn create_workspace_status_action(
        &self,
        env: &dyn WorkspaceStatusActionEnvironment,
    ) -> Arc<dyn WorkspaceStatusAction> {
        let stable_artifact = env.create_stable_artifact("stable-status.txt");
        let volatile_artifact = env.create_volatile_artifact("volatile-status.txt");
        Arc::new(BazelWorkspaceStatusAction::new(
            stable_artifact,
            volatile_artifact,
            self.username.clone(),
            self.hostname.clone(),
        ))
    }

    // Java: com/google/devtools/build/lib/bazel/BazelWorkspaceStatusModule.java:272 BazelStatusActionFactory.createDummyWorkspaceStatus
    fn create_dummy_workspace_status(&self) -> BTreeMap<String, String> {
        BTreeMap::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn format_matches_java_pattern() {
        // 2024-02-29T13:05:09Z was a Thursday.
        assert_eq!(format(1_709_211_909_000), "2024 Feb 29 13 05 09 Thu");
        assert_eq!(format(0), "1970 Jan 01 00 00 00 Thu");
    }

    #[test]
    fn parse_workspace_status_trims_and_sorts() {
        let m = parse_workspace_status("  STABLE_B  two words \nA 1\nnospace\n\n");
        let v: Vec<(&str, &str)> = m.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
        assert_eq!(v, vec![("A", "1"), ("STABLE_B", "two words")]);
    }

    // Byte-strings review 2026-11-20 S2-5. Java's TreeMap orders keys by UTF-16 units:
    // `STABLE_🐷` (D83D DC37) sorts before `STABLE_Ａ` (FF21); by code point it would be after.
    #[test]
    fn parse_workspace_status_orders_by_utf16_units() {
        let m = parse_workspace_status("STABLE_Ａ 1\nSTABLE_🐷 2\n");
        let v: Vec<&str> = m.keys().map(|k| k.as_str()).collect();
        assert_eq!(v, vec!["STABLE_🐷", "STABLE_Ａ"]);
    }

    #[test]
    fn source_date_epoch() {
        let mut env = IndexMap::new();
        env.insert("SOURCE_DATE_EPOCH".to_string(), " 1700000000 ".to_string());
        assert_eq!(get_current_time_millis(&env, 5), 1_700_000_000_000);
        env.insert("SOURCE_DATE_EPOCH".to_string(), "abc".to_string());
        assert_eq!(get_current_time_millis(&env, 5), 5);
    }
}
