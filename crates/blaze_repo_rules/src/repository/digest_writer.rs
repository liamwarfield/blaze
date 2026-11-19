//! Handles writing and reading of repo marker files.
//!
//! Java: com/google/devtools/build/lib/bazel/repository/DigestWriter.java.

use blaze_bzlmod_model::AttributeValue;
use blaze_bzlmod_model::AttributeValues;
use blaze_bzlmod_model::gson::JsonWriter;
use blaze_cmdline::RepositoryName;
use blaze_repo_fetch::fs_utils;
use blaze_sky_loading::directories::LoadingDirectories;
use blaze_skyframe::Environment;
use blaze_skyframe::Interrupted;
use blaze_skyframe::Transience;
use blaze_starlark::StarlarkSemantics;
use blaze_util::fingerprint::Fingerprint;
use blaze_util::string_encoding::internal_to_unicode;
use blaze_util::string_encoding::latin1_decode;
use blaze_util::string_encoding::latin1_encode_lossy;
use blaze_vfs::Path;

use super::exceptions::RepositoryFunctionException;
use super::repo_definition::RepoDefinition;
use super::repo_environment_function::get_environment_view;
use super::repo_recorded_input::RepoRecordedInput;
use super::repo_recorded_input::RepoRecordedInputWithValue;
use super::repo_recorded_input::wrap_env_vars;
use super::repository_utils::get_external_repository_directory;
use crate::semantics_fingerprint::stable_fingerprint;
use crate::starlark::os::java_os_arch_lower_case;
use crate::starlark::os::java_os_name_lower_case;

/// The marker file version is inject in the rule key digest so the rule key is always different
/// when we decide to update the format.
// Java: com/google/devtools/build/lib/bazel/repository/DigestWriter.java:46 DigestWriter.MARKER_FILE_VERSION
const MARKER_FILE_VERSION: i32 = 7;

/// Either an error the caller rethrows (Java `RepositoryFunctionException`) or an interruption.
#[derive(Debug)]
pub enum DigestWriterError {
    Repository(RepositoryFunctionException),
    Interrupted(Interrupted),
}

impl From<Interrupted> for DigestWriterError {
    fn from(e: Interrupted) -> Self {
        DigestWriterError::Interrupted(e)
    }
}

impl From<RepositoryFunctionException> for DigestWriterError {
    fn from(e: RepositoryFunctionException) -> Self {
        DigestWriterError::Repository(e)
    }
}

/// Handles writing and reading of repo marker files.
// Java: com/google/devtools/build/lib/bazel/repository/DigestWriter.java:42 DigestWriter
#[derive(Debug)]
pub struct DigestWriter {
    directories: LoadingDirectories,
    pub(crate) predeclared_input_hash: String,
    pub(crate) marker_path: Path,
}

impl DigestWriter {
    // Java: com/google/devtools/build/lib/bazel/repository/DigestWriter.java:52 DigestWriter.<init>
    fn new(
        directories: &LoadingDirectories,
        repository_name: &RepositoryName,
        predeclared_input_hash: String,
    ) -> Self {
        DigestWriter {
            directories: directories.clone(),
            predeclared_input_hash,
            marker_path: Self::get_marker_path(directories, repository_name),
        }
    }

    /// Returns `None` if and only if a Skyframe restart is needed.
    // Java: com/google/devtools/build/lib/bazel/repository/DigestWriter.java:61 DigestWriter.create
    pub fn create(
        env: &mut Environment<'_>,
        directories: &LoadingDirectories,
        repository_name: &RepositoryName,
        repo_definition: &RepoDefinition,
        starlark_semantics: &StarlarkSemantics,
    ) -> Result<Option<DigestWriter>, Interrupted> {
        let Some(predeclared_input_hash) =
            Self::compute_predeclared_input_hash(env, repo_definition, starlark_semantics)?
        else {
            return Ok(None);
        };
        Ok(Some(DigestWriter::new(
            directories,
            repository_name,
            predeclared_input_hash,
        )))
    }

    pub fn predeclared_input_hash(&self) -> &str {
        &self.predeclared_input_hash
    }

    pub fn marker_path(&self) -> &Path {
        &self.marker_path
    }

    // Java: com/google/devtools/build/lib/bazel/repository/DigestWriter.java:76 DigestWriter.writeMarkerFile
    pub fn write_marker_file(
        &self,
        recorded_input_values: &[RepoRecordedInputWithValue],
    ) -> Result<(), RepositoryFunctionException> {
        let mut builder = String::new();
        builder.push_str(&self.predeclared_input_hash);
        builder.push('\n');
        for recorded_input_value in recorded_input_values {
            builder.push_str(&recorded_input_value.to_string());
            builder.push('\n');
        }
        // Java: FileSystemUtils.writeContent(markerPath, ISO_8859_1, content)
        fs_utils::write_content(&self.marker_path, &latin1_encode_lossy(&builder))
            .map_err(|e| RepositoryFunctionException::from_io_error(&e, Transience::Transient))
    }

    // Java: com/google/devtools/build/lib/bazel/repository/DigestWriter.java:91 DigestWriter.areRepositoryAndMarkerFileConsistent
    pub fn are_repository_and_marker_file_consistent(
        &self,
        env: &mut Environment<'_>,
    ) -> Result<Option<String>, DigestWriterError> {
        self.are_repository_and_marker_file_consistent_at(env, &self.marker_path)
    }

    /// Checks if the state of the repo in the filesystem is consistent with its current
    /// definition. Returns `None` if they are consistent; otherwise, returns a description of why
    /// they are not.
    ///
    /// This method treats a missing Skyframe dependency as if the repo is not up to date. The
    /// caller is responsible for checking `env.values_missing()`.
    // Java: com/google/devtools/build/lib/bazel/repository/DigestWriter.java:104 DigestWriter.areRepositoryAndMarkerFileConsistent
    pub fn are_repository_and_marker_file_consistent_at(
        &self,
        env: &mut Environment<'_>,
        marker_path: &Path,
    ) -> Result<Option<String>, DigestWriterError> {
        if !marker_path.exists() {
            return Ok(Some("repo hasn't been fetched yet".to_owned()));
        }

        let content = match fs_utils::read_content(marker_path) {
            Ok(bytes) => bytes,
            Err(e) => {
                return Err(
                    RepositoryFunctionException::from_io_error(&e, Transience::Transient).into(),
                );
            }
        };
        // Java: FileSystemUtils.readContent(markerPath, ISO_8859_1)
        let content = latin1_decode(&content);
        let Some(recorded_input_values) =
            Self::read_marker_file(&content, &self.predeclared_input_hash)
        else {
            return Ok(Some(
                "Bazel version, flags, repo rule definition or attributes changed".to_owned(),
            ));
        };
        // Check inputs in batches to prevent Skyframe cycles caused by outdated dependencies.
        for batch in RepoRecordedInputWithValue::split_into_batches(&recorded_input_values) {
            let outdated_reason =
                RepoRecordedInput::is_any_value_outdated(env, &self.directories, &batch)?;
            if outdated_reason.is_some() {
                return Ok(outdated_reason);
            }
        }
        Ok(None)
    }

    /// Returns a list of recorded inputs with their values parsed from the given marker file if
    /// the predeclared input hash matches, or `None` if the hash doesn't match or any error occurs
    /// during parsing.
    // Java: com/google/devtools/build/lib/bazel/repository/DigestWriter.java:137 DigestWriter.readMarkerFile
    pub fn read_marker_file(
        content: &str,
        predeclared_input_hash: &str,
    ) -> Option<Vec<RepoRecordedInputWithValue>> {
        let mut recorded_input_values = Vec::new();
        let mut first_line_verified = false;
        for line in content.split('\n') {
            if line.is_empty() {
                continue;
            }
            if !first_line_verified {
                if line != predeclared_input_hash {
                    // Break early, need to reload anyway. This also detects marker file version
                    // changes so that unknown formats are not parsed.
                    return None;
                }
                first_line_verified = true;
            } else {
                let Some(input_and_value) = RepoRecordedInputWithValue::parse(line) else {
                    // On parse failure, just forget everything else and mark the whole input out
                    // of date.
                    return None;
                };
                recorded_input_values.push(input_and_value);
            }
        }
        if !first_line_verified {
            return None;
        }
        Some(recorded_input_values)
    }

    // Java: com/google/devtools/build/lib/bazel/repository/DigestWriter.java:170 DigestWriter.computePredeclaredInputHash
    pub(crate) fn compute_predeclared_input_hash(
        env: &mut Environment<'_>,
        repo_definition: &RepoDefinition,
        starlark_semantics: &StarlarkSemantics,
    ) -> Result<Option<String>, Interrupted> {
        let Some(environ) = get_environment_view(env, &repo_definition.repo_rule.environ)? else {
            return Ok(None);
        };
        let environ_inputs = wrap_env_vars(&environ);
        let repo_rule = &repo_definition.repo_rule;
        let bzl_file_label = repo_rule
            .id
            .bzl_file_label()
            .expect("NullPointerException: RepoRuleId.bzlFileLabel is null");
        let attr_values_json = attribute_values_to_gson_json(&repo_definition.attr_values);
        let mut fp = Fingerprint::new();
        fp.add_int(MARKER_FILE_VERSION)
            .add_bytes(&stable_fingerprint(starlark_semantics))
            .add_string(&bzl_file_label.to_string())
            .add_string(repo_rule.id.rule_name())
            .add_bytes(&repo_rule.transitive_bzl_digest)
            .add_string(&repo_definition.name)
            .add_string(&attr_values_json)
            // This info is accessible via rctx.os.{name,arch} and can also influence the result
            // of a repo rule in subtle ways (e.g. behavior of host tools, line breaks, etc).
            // PORT: both values come from the functions `repository_os` uses, so that
            // `rctx.os.{name,arch}` and this hash cannot disagree (Blaze is Linux x86-64 only).
            .add_string(&java_os_name_lower_case())
            .add_string(&java_os_arch_lower_case());
        fp.add_int(environ_inputs.len() as i32);
        for (key, value) in &environ_inputs {
            fp.add_string(&key.to_string());
            match value {
                None => {
                    fp.add_boolean(false);
                }
                Some(v) => {
                    fp.add_boolean(true).add_string(v);
                }
            }
        }
        fp.add_int(repo_rule.recorded_repo_mapping_entries.len() as i32);
        for (row, column, value) in &repo_rule.recorded_repo_mapping_entries {
            fp.add_string(row.get_name());
            fp.add_string(column);
            fp.add_string(value.get_name());
        }
        Ok(Some(fp.hex_digest_and_reset()))
    }

    // Java: com/google/devtools/build/lib/bazel/repository/DigestWriter.java:212 DigestWriter.getMarkerPath
    fn get_marker_path(directories: &LoadingDirectories, repo: &RepositoryName) -> Path {
        get_external_repository_directory(directories).get_child(&repo.get_marker_file_name())
    }

    // Java: com/google/devtools/build/lib/bazel/repository/DigestWriter.java:217 DigestWriter.clearMarkerFile
    pub fn clear_marker_file(
        directories: &LoadingDirectories,
        repo: &RepositoryName,
    ) -> Result<(), RepositoryFunctionException> {
        Self::get_marker_path(directories, repo)
            .delete()
            .map(|_| ())
            .map_err(|e| RepositoryFunctionException::from_io_error(&e, Transience::Transient))
    }
}

/// `GsonTypeAdapterUtil.SINGLE_EXTENSION_USAGES_VALUE_GSON.toJson(attrValues)`.
///
/// `toJson(Object)` serializes the runtime class `AutoValue_AttributeValues`, for which no adapter
/// is registered (`AttributeValuesAdapter` is registered for the exact type `AttributeValues`
/// only), so Gson's reflective adapter writes its one field: `{"attributes":{...}}`. The values
/// are then serialized by their runtime types: strings and labels (Bazel's adapters) as strings,
/// booleans, lists and dicts as JSON, `StarlarkInt` reflectively as `{"v":N}` and `None`
/// (`NoneType`, no fields) as `{}`. Verified against Bazel with `scripts/w5r/java-probe/run
/// attr-json`.
// Java: com/google/devtools/build/lib/bazel/repository/DigestWriter.java:165 DigestWriter.computePredeclaredInputHash
pub fn attribute_values_to_gson_json(attr_values: &AttributeValues) -> String {
    fn value(w: &mut JsonWriter, v: &AttributeValue) {
        match v {
            AttributeValue::None => {
                w.begin_object().end_object();
            }
            AttributeValue::Bool(b) => {
                w.value_bool(*b);
            }
            AttributeValue::Int(i) => {
                w.begin_object()
                    .name("v")
                    .value_number(&i.to_string())
                    .end_object();
            }
            // GsonTypeAdapterUtil.STRING_ADAPTER: internalToUnicode(s).
            AttributeValue::String(s) => {
                w.value_str(&internal_to_unicode(s));
            }
            // GsonTypeAdapterUtil.LABEL_TYPE_ADAPTER:
            // internalToUnicode(label.getUnambiguousCanonicalForm()).
            AttributeValue::Label(l) => {
                w.value_str(&internal_to_unicode(&l.get_unambiguous_canonical_form()));
            }
            AttributeValue::List(items) => {
                w.begin_array();
                for i in items {
                    value(w, i);
                }
                w.end_array();
            }
            AttributeValue::Dict(entries) => {
                w.begin_object();
                for (k, v) in entries {
                    let key = match k {
                        // Keys go through the same adapters (complex map key serialization
                        // writes a key's JSON primitive as its string).
                        AttributeValue::String(s) => internal_to_unicode(s).into_owned(),
                        AttributeValue::Label(l) => {
                            internal_to_unicode(&l.get_unambiguous_canonical_form()).into_owned()
                        }
                        AttributeValue::Bool(b) => b.to_string(),
                        AttributeValue::Int(i) => i.to_string(),
                        other => blaze_util::unported!(
                            "Gson complex map key serialization",
                            "dict key {other:?}"
                        ),
                    };
                    w.name(&key);
                    value(w, v);
                }
                w.end_object();
            }
        }
    }
    let mut w = JsonWriter::new(false);
    w.begin_object().name("attributes").begin_object();
    for (name, v) in attr_values.attributes() {
        w.name(name);
        value(&mut w, v);
    }
    w.end_object().end_object();
    w.into_string()
}

#[cfg(test)]
mod tests {
    use super::*;

    // Blaze-only: Java has no DigestWriterTest; the expectations follow readMarkerFile's contract.
    #[test]
    fn read_marker_file_checks_the_first_line() {
        assert!(DigestWriter::read_marker_file("", "abc").is_none());
        assert!(DigestWriter::read_marker_file("xyz\n", "abc").is_none());
        assert_eq!(
            DigestWriter::read_marker_file("\nabc\n\n", "abc").map(|v| v.len()),
            Some(0)
        );
    }
}
