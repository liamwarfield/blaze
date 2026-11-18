//! Starlark-free helpers behind `repository_ctx` / `module_ctx`
//! (`StarlarkBaseExternalContext`, `StarlarkRepositoryContext`), for the Starlark layer to call.

use std::collections::HashMap;
use std::io::Write as _;

use indexmap::{IndexMap, IndexSet};

use blaze_vfs::{FileSystemRef, Path, PathFragment};

use crate::errors::IoException;

/// Errors `which()` raises as Starlark `EvalException`s.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WhichError(pub String);

/// `repository_ctx.which(program)`: the path of `program` on the repo environment's `PATH`, or
/// `None`.
///
/// PORT: `OsUtils.executableExtension()` is empty on Linux, so the second lookup with the
/// extension never runs.
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:2336 StarlarkBaseExternalContext.which
pub fn which(
    program: &str,
    repo_env: &HashMap<String, String>,
    file_system: &FileSystemRef,
) -> Result<Option<Path>, WhichError> {
    if program.contains('/') || program.contains('\\') {
        return Err(WhichError(format!(
            "Program argument of which() may not contain a / or a \\ ('{}' given)",
            program
        )));
    }
    if program.is_empty() {
        return Err(WhichError(
            "Program argument of which() may not be empty".to_string(),
        ));
    }
    // IOException when checking executable file means we cannot read the file data so we cannot
    // execute it, swallow the exception.
    Ok(find_command_on_path(program, repo_env, file_system).unwrap_or(None))
}

// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:2366 StarlarkBaseExternalContext.findCommandOnPath
fn find_command_on_path(
    program: &str,
    repo_env: &HashMap<String, String>,
    file_system: &FileSystemRef,
) -> Result<Option<Path>, IoException> {
    let Some(path_env_variable) = repo_env.get("PATH") else {
        return Ok(None);
    };
    // Java String.split(":") drops trailing empty strings; empty entries are relative anyway.
    for p in path_env_variable.split(':') {
        let fragment = PathFragment::create(p);
        if fragment.is_absolute() {
            // We ignore relative path as they don't mean much here.
            let path = Path::create(fragment, file_system).get_child(java_trim(program));
            if path.exists() && path.is_file() && path.is_executable()? {
                return Ok(Some(path));
            }
        }
    }
    Ok(None)
}

fn java_trim(s: &str) -> &str {
    s.trim_matches(|c: char| c <= ' ')
}

/// Create parent directories for the given path.
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1792 StarlarkBaseExternalContext.makeDirectories
pub fn make_directories(path: &Path) -> Result<(), IoException> {
    if let Some(parent) = path.get_parent_directory() {
        parent.create_directory_and_parents()?;
    }
    Ok(())
}

/// The file-system part of `repository_ctx.file(path, content, executable)`: creates parent
/// directories, replaces the file with `content` (Bazel's internal string bytes) and sets the
/// executable bit if requested. The caller does the output-directory check.
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:1501 StarlarkBaseExternalContext.createFile
pub fn create_file(path: &Path, content: &[u8], executable: bool) -> Result<(), IoException> {
    make_directories(path)?;
    path.delete()?;
    {
        let mut stream = path.get_output_stream()?;
        stream.write_all(content)?;
        stream.flush()?;
    }
    if executable {
        path.set_executable(true)?;
    }
    Ok(())
}

/// `StringUtilities.replaceAllLiteral`.
// Java: com/google/devtools/build/lib/util/StringUtilities.java:62 StringUtilities.replaceAllLiteral
pub fn replace_all_literal(s: &str, literal: &str, replacement: &str) -> String {
    if literal.is_empty() {
        // Java String.replace("", r) inserts r between every char and at both ends.
        let mut out = String::from(replacement);
        for c in s.chars() {
            out.push(c);
            out.push_str(replacement);
        }
        return out;
    }
    s.replace(literal, replacement)
}

/// The file-system part of `repository_ctx.template(path, template, substitutions,
/// executable)`: reads the template as raw bytes, applies the substitutions in order and writes
/// the result.
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryContext.java:336 StarlarkRepositoryContext.createFileFromTemplate
pub fn create_file_from_template(
    path: &Path,
    template: &Path,
    substitutions: &IndexMap<String, String>,
    executable: bool,
) -> Result<(), IoException> {
    make_directories(path)?;
    // PORT: Java reads and writes as Latin-1 (raw bytes); Rust uses UTF-8 strings, identical for
    // valid UTF-8 content.
    let raw = crate::fs_utils::read_content(template)?;
    // PORT: byte-strings (Java reads the template as ISO-8859-1, keeping the raw bytes)
    let mut tpl = String::from_utf8_lossy(&raw).into_owned();
    for (k, v) in substitutions {
        tpl = replace_all_literal(&tpl, k, v);
    }
    path.delete()?;
    {
        let mut stream = path.get_output_stream()?;
        stream.write_all(tpl.as_bytes())?;
        stream.flush()?;
    }
    if executable {
        path.set_executable(true)?;
    }
    Ok(())
}

/// The `environment` argument of `repository_ctx.execute`: `None` values remove a variable,
/// strings set it, anything else is an error (`environment values must be strings or None,
/// got <value>`; the caller formats the value).
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum EnvValue {
    Set(String),
    Remove,
}

/// Splits `execute`'s environment into variables to force and variables to remove.
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkBaseExternalContext.java:2030 StarlarkBaseExternalContext.execute
pub fn split_execute_environment(
    environment: &IndexMap<String, EnvValue>,
) -> (IndexMap<String, String>, IndexSet<String>) {
    let mut force = IndexMap::new();
    let mut remove = IndexSet::new();
    for (k, v) in environment {
        match v {
            EnvValue::Remove => {
                remove.insert(k.clone());
            }
            EnvValue::Set(s) => {
                force.insert(k.clone(), s.clone());
            }
        }
    }
    (force, remove)
}

/// The environment a local `execute` runs with: the repo environment minus removed variables,
/// then the forced ones (`Maps.filterKeys(repoEnv, ...)` + `addEnvironmentVariables`).
pub fn execute_environment(
    repo_env: &IndexMap<String, String>,
    force: &IndexMap<String, String>,
    remove: &IndexSet<String>,
) -> IndexMap<String, String> {
    let mut env: IndexMap<String, String> = repo_env
        .iter()
        .filter(|(k, _)| !remove.contains(*k))
        .map(|(k, v)| (k.clone(), v.clone()))
        .collect();
    for (k, v) in force {
        env.insert(k.clone(), v.clone());
    }
    env
}
