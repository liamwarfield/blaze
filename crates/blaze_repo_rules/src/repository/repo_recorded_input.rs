//! Recorded inputs of a repo fetch (or module extension evaluation) and their marker-file form.
//!
//! Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java.
//!
//! Represents a "recorded input" of a repo fetch. We define the "input" of a repo fetch as any
//! entity that could affect the output of the repo fetch (i.e. the repo contents). A "recorded
//! input" is thus any input we can record during the fetch and thus know about only after the
//! fetch. This contrasts with "predeclared inputs", which are known before fetching the repo, and
//! "undiscoverable inputs", which are used during the fetch but is not recorded or recordable.
//!
//! Recorded inputs are of particular interest, since in order to determine whether a fetched repo
//! is still up-to-date, the identity of all recorded inputs need to be stored in addition to their
//! values. This contrasts with predeclared inputs; the whole set of predeclared inputs are known
//! before the fetch, so we can simply hash all predeclared input values.
//!
//! Recorded inputs and their values are stored in *marker files* for repos. Each recorded input
//! is stored as a string, with a prefix denoting its type, followed by a colon, and then the
//! information identifying that specific input.
//!
//! PORT: Java's sealed class hierarchy (`File`, `Dirents`, `DirTree`, `EnvVar`,
//! `RecordedRepoMapping`, `NeverUpToDateRepoRecordedInput`) is the enum [`RepoRecordedInput`];
//! the per-type `Parser` objects are the prefix match in [`RepoRecordedInput::parse`].

use std::collections::BTreeMap;
use std::fmt;

use blaze_cmdline::LabelSyntaxException;
use blaze_cmdline::RepositoryName;
use blaze_cmdline::label_constants::EXTERNAL_REPOSITORY_LOCATION;
use blaze_cmdline::label_validator;
use blaze_sky_loading::actions::FileValue;
use blaze_sky_loading::directories::LoadingDirectories;
use blaze_sky_loading::directory_listing::DirectoryListingKey;
use blaze_sky_loading::directory_listing::DirectoryListingValue;
use blaze_sky_loading::file::FileKey;
use blaze_sky_loading::precomputed::STARLARK_SEMANTICS;
use blaze_sky_loading::repos::RepositoryMappingKey;
use blaze_sky_loading::repos::RepositoryMappingValue;
use blaze_skyframe::Environment;
use blaze_skyframe::Interrupted;
use blaze_skyframe::SkyKey;
use blaze_util::fingerprint::Fingerprint;
use blaze_vfs::PathFragment;
use blaze_vfs::Root;
use blaze_vfs::RootedPath;
use indexmap::IndexMap;
use indexmap::IndexSet;

use super::directory_tree_digest::DirectoryTreeDigestKey;
use super::repo_environment_function::RepoEnvironmentKey;

/// The prefixes of the recorded input types (Java `Parser.getPrefix()`).
const FILE_PREFIX: &str = "FILE";
const DIRENTS_PREFIX: &str = "DIRENTS";
const DIRTREE_PREFIX: &str = "DIRTREE";
const ENV_PREFIX: &str = "ENV";
const REPO_MAPPING_PREFIX: &str = "REPO_MAPPING";

/// The reason of [`RepoRecordedInput::parse_failure`].
// Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:889 NeverUpToDateRepoRecordedInput.PARSE_FAILURE
const PARSE_FAILURE_REASON: &str = "malformed marker file entry encountered";

// Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:323 RepoRecordedInput.UNDECIDED
const UNDECIDED: &str = "values missing";

/// A recorded input of a repo fetch.
///
/// PORT: Java's `NeverUpToDateRepoRecordedInput.equals` is identity; here two sentinels with the
/// same reason are equal. The only sentinel Java creates is `PARSE_FAILURE`, a singleton, so the
/// two agree.
// Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:75 RepoRecordedInput
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum RepoRecordedInput {
    /// Represents a file input accessed during the repo fetch. Despite being named just "file",
    /// this can represent a file or a directory on the filesystem, and it does not need to exist.
    /// The value of the input contains whether this is a file or a directory or nonexistent, and
    /// if it's a file, the digest of its contents.
    // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:413 RepoRecordedInput.File
    File(RepoCacheFriendlyPath),
    /// Represents the list of entries under a directory accessed during the fetch.
    ///
    /// May only be requested for paths that are known to represent existing directories, so
    /// consumers usually have to request a [`RepoRecordedInput::File`] first.
    // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:527 RepoRecordedInput.Dirents
    Dirents(RepoCacheFriendlyPath),
    /// Represents an entire directory tree accessed during the fetch. Anything under the tree
    /// changing (including adding/removing/renaming files or directories and changing file
    /// contents) will cause it to go out of date.
    ///
    /// May only be requested for paths that are known to represent existing directories, so
    /// consumers usually have to request a [`RepoRecordedInput::File`] first.
    // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:625 RepoRecordedInput.DirTree
    DirTree(RepoCacheFriendlyPath),
    /// Represents an environment variable accessed during the repo fetch.
    // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:706 RepoRecordedInput.EnvVar
    EnvVar(String),
    /// Represents a repo mapping entry that was used during the repo fetch.
    // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:792 RepoRecordedInput.RecordedRepoMapping
    RecordedRepoMapping {
        source_repo: RepositoryName,
        apparent_name: String,
    },
    /// A sentinel "input" that's always out-of-date for a given reason.
    // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:887 RepoRecordedInput.NeverUpToDateRepoRecordedInput
    NeverUpToDate(String),
}

/// Represents the possible values returned by [`RepoRecordedInput::get_value`]: either a valid
/// value (which may be null), or an invalid value with a reason (e.g. due to I/O failure).
// Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:280 RepoRecordedInput.MaybeValue
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum MaybeValue {
    /// Represents a valid value, which may be null.
    Valid(Option<String>),
    /// Represents an invalid value with a reason (e.g. due to I/O failure).
    Invalid(String),
}

impl MaybeValue {
    // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:281 MaybeValue.VALUES_MISSING
    pub fn values_missing() -> MaybeValue {
        MaybeValue::Invalid("values missing".to_owned())
    }
}

/// A recorded input along with its recorded value.
// Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:117 RepoRecordedInput.WithValue
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct RepoRecordedInputWithValue {
    pub input: RepoRecordedInput,
    pub value: Option<String>,
}

impl RepoRecordedInputWithValue {
    pub fn new(input: RepoRecordedInput, value: Option<String>) -> Self {
        RepoRecordedInputWithValue { input, value }
    }

    /// Parses a [`RepoRecordedInputWithValue`] from its string representation.
    // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:119 RepoRecordedInput.WithValue.parse
    pub fn parse(s: &str) -> Option<Self> {
        let s_char = s.find(' ');
        if let Some(s_char) = s_char
            && s_char > 0
        {
            let Some(unescaped) = unescape(&s[..s_char]) else {
                // Java: RepoRecordedInput.parse(null) fails in Splitter.splitToList.
                panic!("java.lang.NullPointerException");
            };
            let input = RepoRecordedInput::parse(&unescaped);
            if input != RepoRecordedInput::parse_failure() {
                return Some(RepoRecordedInputWithValue::new(
                    input,
                    unescape(&s[s_char + 1..]),
                ));
            }
        }
        None
    }

    /// Splits the given list of recorded input values into batches such that within each batch,
    /// all recorded inputs's `SkyKey`s can be requested together.
    // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:134 RepoRecordedInput.WithValue.splitIntoBatches
    pub fn split_into_batches(recorded_input_values: &[Self]) -> Vec<Vec<Self>> {
        let mut batches = Vec::new();
        let mut current_batch: Vec<Self> = Vec::new();
        for recorded_input_value in recorded_input_values {
            if !recorded_input_value
                .input
                .can_be_requested_unconditionally()
                && !current_batch.is_empty()
            {
                batches.push(std::mem::take(&mut current_batch));
            }
            current_batch.push(recorded_input_value.clone());
        }
        if !current_batch.is_empty() {
            batches.push(current_batch);
        }
        batches
    }
}

impl fmt::Display for RepoRecordedInputWithValue {
    /// Converts this value to a string in a format compatible with
    /// [`RepoRecordedInputWithValue::parse`].
    // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:154 RepoRecordedInput.WithValue.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.input, escape(self.value.as_deref()))
    }
}

/// Escapes a (nullable) string for the marker file: `null` is `\0`; backslash, newline and space
/// are escaped.
// Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:233 RepoRecordedInput.escape
pub fn escape(str: Option<&str>) -> String {
    match str {
        None => "\\0".to_owned(),
        Some(str) => str
            .replace('\\', "\\\\")
            .replace('\n', "\\n")
            .replace(' ', "\\s"),
    }
}

/// Inverse of [`escape`]; `\0` is the null string.
// Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:239 RepoRecordedInput.unescape
pub fn unescape(str: &str) -> Option<String> {
    if str == "\\0" {
        return None; // \0 == null string
    }
    let mut result = String::new();
    let mut escaped = false;
    for c in str.chars() {
        if escaped {
            if c == 'n' {
                // n means new line
                result.push('\n');
            } else if c == 's' {
                // s means space
                result.push(' ');
            } else {
                // Any other escaped characters are just un-escaped
                result.push(c);
            }
            escaped = false;
        } else if c == '\\' {
            escaped = true;
        } else {
            result.push(c);
        }
    }
    Some(result)
}

/// Java `EnvVar.wrap`: the environment variables as recorded inputs, sorted by name.
// Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:722 RepoRecordedInput.EnvVar.wrap
pub fn wrap_env_vars(
    env_vars: &BTreeMap<String, Option<String>>,
) -> IndexMap<RepoRecordedInput, Option<String>> {
    // PORT: Java sorts with String.compareTo (UTF-16 order); a BTreeMap is in byte order, which
    // agrees for the names of environment variables (no supplementary characters).
    env_vars
        .iter()
        .map(|(k, v)| (RepoRecordedInput::EnvVar(k.clone()), v.clone()))
        .collect()
}

impl RepoRecordedInput {
    /// A sentinel "input" that's always out-of-date to signify parse failure.
    // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:889 NeverUpToDateRepoRecordedInput.PARSE_FAILURE
    pub fn parse_failure() -> RepoRecordedInput {
        RepoRecordedInput::NeverUpToDate(PARSE_FAILURE_REASON.to_owned())
    }

    /// Parses a recorded input from its string representation. Returns
    /// [`RepoRecordedInput::parse_failure`] if the string representation is invalid.
    // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:99 RepoRecordedInput.parse
    pub fn parse(s: &str) -> RepoRecordedInput {
        let Some((prefix, rest)) = s.split_once(':') else {
            return RepoRecordedInput::parse_failure();
        };
        match prefix {
            FILE_PREFIX => Self::parse_path(rest, RepoRecordedInput::File),
            DIRENTS_PREFIX => Self::parse_path(rest, RepoRecordedInput::Dirents),
            DIRTREE_PREFIX => Self::parse_path(rest, RepoRecordedInput::DirTree),
            // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:715 EnvVar.PARSER.parse
            ENV_PREFIX => RepoRecordedInput::EnvVar(rest.to_owned()),
            REPO_MAPPING_PREFIX => Self::parse_repo_mapping(rest),
            _ => RepoRecordedInput::parse_failure(),
        }
    }

    // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:422 File.PARSER.parse
    // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:536 Dirents.PARSER.parse
    // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:634 DirTree.PARSER.parse
    fn parse_path(
        s: &str,
        make: impl FnOnce(RepoCacheFriendlyPath) -> RepoRecordedInput,
    ) -> RepoRecordedInput {
        match RepoCacheFriendlyPath::parse(s) {
            Ok(path) => make(path),
            // malformed inputs cause refetch
            Err(_) => RepoRecordedInput::parse_failure(),
        }
    }

    // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:801 RecordedRepoMapping.PARSER.parse
    fn parse_repo_mapping(s: &str) -> RepoRecordedInput {
        // Splitter.on(',').limit(2): a missing second part is an IndexOutOfBoundsException.
        let mut parts = s.splitn(2, ',');
        let first = parts.next().unwrap_or("");
        let Some(second) = parts.next() else {
            // malformed inputs cause refetch
            return RepoRecordedInput::parse_failure();
        };
        match RepositoryName::create(first) {
            Ok(source_repo) => RepoRecordedInput::RecordedRepoMapping {
                source_repo,
                apparent_name: second.to_owned(),
            },
            // malformed inputs cause refetch
            Err(_) => RepoRecordedInput::parse_failure(),
        }
    }

    /// Java `getParser().getPrefix()`.
    fn get_prefix(&self) -> &'static str {
        match self {
            RepoRecordedInput::File(_) => FILE_PREFIX,
            RepoRecordedInput::Dirents(_) => DIRENTS_PREFIX,
            RepoRecordedInput::DirTree(_) => DIRTREE_PREFIX,
            RepoRecordedInput::EnvVar(_) => ENV_PREFIX,
            RepoRecordedInput::RecordedRepoMapping { .. } => REPO_MAPPING_PREFIX,
            // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:914 NeverUpToDateRepoRecordedInput.getParser
            RepoRecordedInput::NeverUpToDate(_) => panic!(
                "java.lang.UnsupportedOperationException: this sentinel input should never be parsed"
            ),
        }
    }

    /// Returns the post-colon substring that identifies the specific input: for example, the
    /// `MY_ENV_VAR` part of `ENV:MY_ENV_VAR`.
    // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:309 RepoRecordedInput.toStringInternal
    fn to_string_internal(&self) -> String {
        match self {
            RepoRecordedInput::File(path)
            | RepoRecordedInput::Dirents(path)
            | RepoRecordedInput::DirTree(path) => path.to_string(),
            RepoRecordedInput::EnvVar(name) => name.clone(),
            // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:847 RecordedRepoMapping.toStringInternal
            RepoRecordedInput::RecordedRepoMapping {
                source_repo,
                apparent_name,
            } => format!("{},{}", source_repo.get_name(), apparent_name),
            // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:909 NeverUpToDateRepoRecordedInput.toStringInternal
            RepoRecordedInput::NeverUpToDate(_) => panic!(
                "java.lang.UnsupportedOperationException: this sentinel input should never be serialized"
            ),
        }
    }

    /// Returns the `SkyKey` that is necessary to determine whether the input is outdated.
    // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:315 RepoRecordedInput.getSkyKey
    pub fn get_sky_key(&self, directories: &LoadingDirectories) -> SkyKey {
        match self {
            // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:488 File.getSkyKey
            RepoRecordedInput::File(path) => {
                SkyKey::new(FileKey::key(path.get_rooted_path(directories)))
            }
            // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:589 Dirents.getSkyKey
            RepoRecordedInput::Dirents(path) => {
                SkyKey::new(DirectoryListingKey::key(path.get_rooted_path(directories)))
            }
            // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:677 DirTree.getSkyKey
            RepoRecordedInput::DirTree(path) => {
                SkyKey::new(DirectoryTreeDigestKey(path.get_rooted_path(directories)))
            }
            // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:760 EnvVar.getSkyKey
            RepoRecordedInput::EnvVar(name) => SkyKey::new(RepoEnvironmentKey::key(name)),
            // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:852 RecordedRepoMapping.getSkyKey
            RepoRecordedInput::RecordedRepoMapping { source_repo, .. } => {
                SkyKey::new(RepositoryMappingKey::key(source_repo.clone()))
            }
            // Return a random SkyKey to satisfy the contract.
            // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:919 NeverUpToDateRepoRecordedInput.getSkyKey
            RepoRecordedInput::NeverUpToDate(_) => SkyKey::new(STARLARK_SEMANTICS.get_key()),
        }
    }

    /// Returns true if [`RepoRecordedInput::get_value`] can be requested even if previous
    /// recorded inputs have not been verified to be up to date.
    // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:321 RepoRecordedInput.canBeRequestedUnconditionally
    pub fn can_be_requested_unconditionally(&self) -> bool {
        match self {
            // Requesting files in external repositories can result in cycles if the external repo
            // now transitively depends on the requesting repo.
            // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:493 File.canBeRequestedUnconditionally
            RepoRecordedInput::File(path) => !path.in_external_repo(),
            // If the File input requested before this input has changed its type to
            // non-directory, this input's DirectoryListingValue must not be requested as it would
            // result in a build error.
            // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:594 Dirents.canBeRequestedUnconditionally
            RepoRecordedInput::Dirents(_) => false,
            // If the File input requested before this input has changed its type to
            // non-directory, this input's DirectoryTreeDigestValue must not be requested as it
            // would result in a build error.
            // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:682 DirTree.canBeRequestedUnconditionally
            RepoRecordedInput::DirTree(_) => false,
            // Environment variables are static data injected into Skyframe, so there is no risk
            // of cycles.
            // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:765 EnvVar.canBeRequestedUnconditionally
            RepoRecordedInput::EnvVar(_) => true,
            // Starlark can only request the mapping of the repo it is currently executing from,
            // which means that the repo has already been fetched (either to execute the code or
            // to verify the transitive .bzl hash). Further cycles aren't possible.
            // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:857 RecordedRepoMapping.canBeRequestedUnconditionally
            RepoRecordedInput::RecordedRepoMapping { .. } => true,
            // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:925 NeverUpToDateRepoRecordedInput.canBeRequestedUnconditionally
            RepoRecordedInput::NeverUpToDate(_) => true,
        }
    }

    /// Returns the current value of this input, which may be null, wrapped in a
    /// [`MaybeValue::Valid`], or a [`MaybeValue::Invalid`] if the value is known to be invalid.
    ///
    /// The method can request Skyframe evaluations, and if any values are missing, this method
    /// can return any value and will be reinvoked after a Skyframe restart.
    // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:297 RepoRecordedInput.getValue
    pub fn get_value(
        &self,
        env: &mut Environment<'_>,
        directories: &LoadingDirectories,
    ) -> Result<MaybeValue, Interrupted> {
        match self {
            // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:500 File.getValue
            RepoRecordedInput::File(path) => {
                let rooted_path = path.get_rooted_path(directories);
                let Some(file_value) = env.get_value(FileKey::key(rooted_path.clone()))? else {
                    return Ok(MaybeValue::values_missing());
                };
                match file_value_to_marker_value(&rooted_path, &file_value) {
                    Ok(v) => Ok(MaybeValue::Valid(Some(v))),
                    Err(e) => Ok(MaybeValue::Invalid(format!(
                        "failed to stat {}: {}",
                        path, e
                    ))),
                }
            }
            // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:601 Dirents.getValue
            RepoRecordedInput::Dirents(path) => {
                let Some(directory_listing_value) =
                    env.get_value(DirectoryListingKey::key(path.get_rooted_path(directories)))?
                else {
                    return Ok(MaybeValue::values_missing());
                };
                Ok(MaybeValue::Valid(Some(
                    directory_listing_value_to_marker_value(&directory_listing_value),
                )))
            }
            // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:689 DirTree.getValue
            RepoRecordedInput::DirTree(path) => {
                let Some(directory_tree_digest_value) =
                    env.get_value(DirectoryTreeDigestKey(path.get_rooted_path(directories)))?
                else {
                    return Ok(MaybeValue::values_missing());
                };
                Ok(MaybeValue::Valid(Some(
                    directory_tree_digest_value.hex_digest.clone(),
                )))
            }
            // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:772 EnvVar.getValue
            RepoRecordedInput::EnvVar(name) => {
                let Some(value) = env.get_value(RepoEnvironmentKey::key(name))? else {
                    return Ok(MaybeValue::values_missing());
                };
                Ok(MaybeValue::Valid(value.value().map(str::to_owned)))
            }
            // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:865 RecordedRepoMapping.getValue
            RepoRecordedInput::RecordedRepoMapping {
                source_repo,
                apparent_name,
            } => {
                let Some(repo_mapping_value) =
                    env.get_value(RepositoryMappingKey::key(source_repo.clone()))?
                else {
                    // PORT: Java dereferences the null value (NullPointerException); callers only
                    // get here after `prefetch` found no missing values, which Blaze's blocking
                    // evaluator only breaks when the dependency failed.
                    return Ok(MaybeValue::values_missing());
                };
                if *repo_mapping_value == RepositoryMappingValue::not_found_value() {
                    return Ok(MaybeValue::Invalid(format!(
                        "source repo {} doesn't exist anymore",
                        source_repo
                    )));
                }
                let canonical_name = repo_mapping_value
                    .repository_mapping()
                    .expect("not NOT_FOUND_VALUE")
                    .get(apparent_name);
                // RepositoryMapping.get never returns null.
                Ok(MaybeValue::Valid(Some(
                    canonical_name.get_name().to_owned(),
                )))
            }
            // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:930 NeverUpToDateRepoRecordedInput.getValue
            RepoRecordedInput::NeverUpToDate(reason) => Ok(MaybeValue::Invalid(reason.clone())),
        }
    }

    /// Returns a human-readable description of the change from `old_value` to `new_value`.
    // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:303 RepoRecordedInput.describeChange
    pub fn describe_change(&self, old_value: Option<&str>, new_value: Option<&str>) -> String {
        match self {
            // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:516 File.describeChange
            RepoRecordedInput::File(path) => {
                format!("file info or contents of {} changed", path)
            }
            // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:612 Dirents.describeChange
            RepoRecordedInput::Dirents(path) => {
                format!("directory entries of {} changed", path)
            }
            // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:700 DirTree.describeChange
            RepoRecordedInput::DirTree(path) => format!("directory tree at {} changed", path),
            // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:782 EnvVar.describeChange
            RepoRecordedInput::EnvVar(name) => format!(
                "environment variable {} changed: {} -> {}",
                name,
                old_value.map_or_else(|| "<unset>".to_owned(), |v| format!("'{}'", v)),
                new_value.map_or_else(|| "<unset>".to_owned(), |v| format!("'{}'", v)),
            ),
            // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:876 RecordedRepoMapping.describeChange
            RepoRecordedInput::RecordedRepoMapping {
                source_repo,
                apparent_name,
            } => format!(
                "canonical name for @{} in {} changed: {} -> {}",
                apparent_name,
                source_repo,
                old_value.unwrap_or("<doesn't exist>"),
                new_value.unwrap_or("<doesn't exist>"),
            ),
            // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:935 NeverUpToDateRepoRecordedInput.describeChange
            RepoRecordedInput::NeverUpToDate(_) => panic!(
                "java.lang.UnsupportedOperationException: the value for this sentinel input is always invalid"
            ),
        }
    }

    /// Returns a human-readable reason for why the given `old_value` is no longer up-to-date for
    /// this recorded input, or `None` if it is still up-to-date.
    ///
    /// The method can request Skyframe evaluations, and if any values are missing, this method
    /// can return any value (`UNDECIDED`) and will be reinvoked after a Skyframe restart.
    // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:210 RepoRecordedInput.isOutdated
    fn is_outdated(
        &self,
        env: &mut Environment<'_>,
        directories: &LoadingDirectories,
        old_value: Option<&str>,
    ) -> Result<Option<String>, Interrupted> {
        let wrapped_new_value = self.get_value(env, directories)?;
        if env.values_missing() {
            return Ok(Some(UNDECIDED.to_owned()));
        }
        Ok(match wrapped_new_value {
            MaybeValue::Invalid(reason) => Some(reason),
            MaybeValue::Valid(new_value) => {
                if old_value == new_value.as_deref() {
                    None
                } else {
                    Some(self.describe_change(old_value, new_value.as_deref()))
                }
            }
        })
    }

    /// Returns whether all values are still up-to-date for each recorded input (`None`) or a
    /// human-readable reason for why that's not the case. If Skyframe values are missing, the
    /// return value should be ignored; callers are responsible for checking
    /// `env.values_missing()` and triggering a Skyframe restart if needed.
    // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:165 RepoRecordedInput.isAnyValueOutdated
    pub fn is_any_value_outdated(
        env: &mut Environment<'_>,
        directories: &LoadingDirectories,
        recorded_input_values: &[RepoRecordedInputWithValue],
    ) -> Result<Option<String>, Interrupted> {
        let inputs: Vec<RepoRecordedInput> = recorded_input_values
            .iter()
            .map(|v| v.input.clone())
            .collect();
        Self::prefetch(env, directories, &inputs)?;
        if env.values_missing() {
            return Ok(Some(UNDECIDED.to_owned()));
        }
        for recorded_input in recorded_input_values {
            let reason = recorded_input.input.is_outdated(
                env,
                directories,
                recorded_input.value.as_deref(),
            )?;
            if reason.is_some() {
                return Ok(reason);
            }
        }
        Ok(None)
    }

    /// Requests the information from Skyframe that is required by future calls to
    /// [`RepoRecordedInput::is_any_value_outdated`] for the given set of inputs.
    // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:186 RepoRecordedInput.prefetch
    pub fn prefetch(
        env: &mut Environment<'_>,
        directories: &LoadingDirectories,
        recorded_inputs: &[RepoRecordedInput],
    ) -> Result<(), Interrupted> {
        // toImmutableSet: first occurrence order, duplicates dropped.
        let keys: IndexSet<SkyKey> = recorded_inputs
            .iter()
            .map(|rri| rri.get_sky_key(directories))
            .collect();
        if env.values_missing() {
            return Ok(());
        }
        let keys: Vec<SkyKey> = keys.into_iter().collect();
        let results = env.get_values_and_exceptions(&keys)?;
        // Per the contract of Environment.getValuesAndExceptions, we need to access the results to
        // actually find all missing values.
        for key in &keys {
            let _unused = results.get(key);
        }
        Ok(())
    }
}

impl fmt::Display for RepoRecordedInput {
    /// Returns the string representation of this recorded input, in the format suitable for
    /// parsing back via [`RepoRecordedInput::parse`].
    ///
    /// The returned string never contains spaces or newlines; those characters are escaped.
    // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:272 RepoRecordedInput.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let prefix = self.get_prefix();
        write!(f, "{}:{}", prefix, escape(Some(&self.to_string_internal())))
    }
}

/// Convert a [`FileValue`] to a String appropriate for placing in a repository marker file. The
/// file need not exist, and can be a file or a directory.
///
/// The error is Java's `IOException` from `Path.getDigest`; its `Display` is `getMessage()`.
// Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:470 RepoRecordedInput.File.fileValueToMarkerValue
pub fn file_value_to_marker_value(
    rooted_path: &RootedPath,
    file_value: &FileValue,
) -> std::io::Result<String> {
    if file_value.is_directory() {
        return Ok("DIR".to_owned());
    }
    if !file_value.exists() {
        return Ok("ENOENT".to_owned());
    }
    // Return the file content digest in hex. fileValue may or may not have the digest available.
    let digest = match file_value.real_file_state_value().get_digest() {
        Some(digest) => digest.to_vec(),
        // Fast digest not available, or it would have been in the FileValue.
        None => file_value
            .real_rooted_path(rooted_path)
            .as_path()
            .get_digest()?,
    };
    Ok(hex::encode(digest))
}

/// The marker value of a directory listing: a fingerprint of the sorted entry names.
// Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:578 RepoRecordedInput.Dirents.directoryListingValueToMarkerValue
fn directory_listing_value_to_marker_value(
    directory_listing_value: &DirectoryListingValue,
) -> String {
    let mut fp = Fingerprint::new();
    let mut names: Vec<&str> = directory_listing_value
        .get_dirents()
        .iter()
        .map(|d| d.get_name())
        .collect();
    // Java sorts the internal strings by UTF-16 code unit: `str` order for chars <= U+00FF.
    names.sort();
    fp.add_strings(&names);
    fp.hex_digest_and_reset()
}

/// Represents a filesystem path stored in a way that is repo-cache-friendly. That is, if the path
/// happens to point inside the current Bazel workspace (in either the main repo or an external
/// repo), we store the appropriate repo name and the path fragment relative to the repo root,
/// instead of the entire absolute path.
///
/// This is *almost* like storing a label, but includes the extra corner case of files inside a
/// repo but not within any package due to missing BUILD files. For example, the file
/// `@@foo//:abc.bzl` is addressable by a label if the file `@@foo//:BUILD` exists. But if the
/// BUILD file doesn't exist, the `abc.bzl` file should still be "watchable"; it's just that
/// `@@foo//:abc.bzl` is technically not a valid label.
///
/// Of course, when the path is outside the current Bazel workspace, we just store the absolute
/// path.
// Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:341 RepoRecordedInput.RepoCacheFriendlyPath
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub struct RepoCacheFriendlyPath {
    pub repo_name: Option<RepositoryName>,
    pub path: PathFragment,
}

impl RepoCacheFriendlyPath {
    // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:347 RepoCacheFriendlyPath.createInsideWorkspace
    pub fn create_inside_workspace(repo_name: RepositoryName, path: PathFragment) -> Self {
        blaze_util::check_argument!(
            !path.is_absolute(),
            "the provided path should be relative to the repo root: {}",
            path
        );
        RepoCacheFriendlyPath {
            repo_name: Some(repo_name),
            path,
        }
    }

    // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:354 RepoCacheFriendlyPath.createOutsideWorkspace
    pub fn create_outside_workspace(path: PathFragment) -> Self {
        blaze_util::check_argument!(
            path.is_absolute(),
            "the provided path should be absolute in the filesystem: {}",
            path
        );
        RepoCacheFriendlyPath {
            repo_name: None,
            path,
        }
    }

    // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:367 RepoCacheFriendlyPath.parse
    pub fn parse(s: &str) -> Result<Self, LabelSyntaxException> {
        if label_validator::is_absolute(s) {
            let skip_ats = if s.starts_with("@@") {
                2
            } else if s.starts_with('@') {
                1
            } else {
                0
            };
            let Some(double_slash) = s.find("//").filter(|&i| i >= skip_ats) else {
                // Java: String.substring with an end before the begin (or -1).
                let end = s.find("//").map_or(-1, |i| i as i64);
                panic!(
                    "java.lang.StringIndexOutOfBoundsException: begin {}, end {}, length {}",
                    skip_ats,
                    end,
                    s.len()
                );
            };
            return Ok(Self::create_inside_workspace(
                RepositoryName::create(&s[skip_ats..double_slash])?,
                PathFragment::create(&s[double_slash + 2..]),
            ));
        }
        Ok(Self::create_outside_workspace(PathFragment::create(s)))
    }

    /// Returns the rooted path corresponding to this "repo-friendly path".
    // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:379 RepoCacheFriendlyPath.getRootedPath
    pub fn get_rooted_path(&self, directories: &LoadingDirectories) -> RootedPath {
        let root = match &self.repo_name {
            None => Root::absolute_root(directories.get_output_base().get_file_system()),
            Some(repo_name) if repo_name.is_main() => {
                Root::from_path(directories.get_workspace().clone())
            }
            // This path is from an external repo. We just directly fabricate the path here
            // instead of requesting the appropriate RepositoryDirectoryValue, since we can rely on
            // the various other SkyFunctions (such as FileStateFunction and
            // DirectoryListingStateFunction) to do that for us instead. This also sidesteps an
            // awkward situation when the external repo in question is not defined.
            Some(repo_name) => Root::from_path(
                directories
                    .get_output_base()
                    .get_relative(&EXTERNAL_REPOSITORY_LOCATION)
                    .get_relative_str(repo_name.get_name()),
            ),
        };
        RootedPath::to_rooted_path(root, &self.path)
    }

    /// Returns true if the path points into an external repository.
    // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:402 RepoCacheFriendlyPath.inExternalRepo
    pub fn in_external_repo(&self) -> bool {
        self.repo_name.as_ref().is_some_and(|r| !r.is_main())
    }
}

impl fmt::Display for RepoCacheFriendlyPath {
    // Java: com/google/devtools/build/lib/rules/repository/RepoRecordedInput.java:361 RepoCacheFriendlyPath.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        // We store `@@foo//abc/def:ghi.bzl` as just `@@foo//abc/def/ghi.bzl`. See class javadoc
        // for more context.
        match &self.repo_name {
            Some(repo_name) => write!(f, "{}//{}", repo_name, self.path),
            None => write!(f, "{}", self.path),
        }
    }
}
