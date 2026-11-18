//! Port of `lib/exec/SpawnLogReconstructor.java`.

use std::collections::hash_map::Entry as HashEntry;
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::io::{self, BufReader, Read};
use std::sync::{Arc, LazyLock};

use blaze_protos::tools::protos::{ExecLogEntry, File, SpawnExec, exec_log_entry};
use indexmap::{IndexMap, IndexSet};
use regex::Regex;

use super::io::{MessageInputStream, parse_delimited_from};
use super::spawn_log_context::JavaOrderedString;

// The path of the repo mapping manifest file under the runfiles tree.
// Java: com/google/devtools/build/lib/exec/SpawnLogReconstructor.java:51 SpawnLogReconstructor.REPO_MAPPING_MANIFEST
const REPO_MAPPING_MANIFEST: &str = "_repo_mapping";

/// Java regex `.`: any char but a line terminator (no `UNIX_LINES`/`DOTALL`).
const JAVA_DOT: &str = r"[^\n\r\u{85}\u{2028}\u{2029}]";

// Examples:
// * bazel-out/k8-fastbuild/bin/pkg/file.txt (repo: null, path: "pkg/file.txt")
// * bazel-out/k8-fastbuild/bin/external/some_repo/pkg/file.txt (repo: "some_repo", path:
//   "pkg/file.txt")
// Java: com/google/devtools/build/lib/exec/SpawnLogReconstructor.java:57 SpawnLogReconstructor.DEFAULT_GENERATED_FILE_RUNFILES_PATH_PATTERN
static DEFAULT_GENERATED_FILE_RUNFILES_PATH_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"^(?:bazel|blaze)-out/[^/]+/[^/]+/(?:external/(?P<repo>[^/]+)/)?(?P<path>{JAVA_DOT}+)$"
    ))
    .unwrap()
});

// Examples:
// * pkg/file.txt (repo: null, path: "pkg/file.txt")
// * external/some_repo/pkg/file.txt (repo: "some_repo", path: "pkg/file.txt")
// Java: com/google/devtools/build/lib/exec/SpawnLogReconstructor.java:63 SpawnLogReconstructor.DEFAULT_SOURCE_FILE_RUNFILES_PATH_PATTERN
static DEFAULT_SOURCE_FILE_RUNFILES_PATH_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"^(?:external/(?P<repo>[^/]+)/)?(?P<path>{JAVA_DOT}+)$"
    ))
    .unwrap()
});

// Examples:
// * pkg/file.txt (repo: null, path: "pkg/file.txt")
// * ../some_repo/pkg/file.txt (repo: "some_repo", path: "pkg/file.txt")
// Java: com/google/devtools/build/lib/exec/SpawnLogReconstructor.java:83 SpawnLogReconstructor.SIBLING_LAYOUT_SOURCE_FILE_RUNFILES_PATH_PATTERN
static SIBLING_LAYOUT_SOURCE_FILE_RUNFILES_PATH_PATTERN: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(&format!(
        r"^(?:\.\./(?P<repo>[^/]+)/)?(?P<path>{JAVA_DOT}+)$"
    ))
    .unwrap()
});

/// Whether `s` matches Java's `.+`.
fn matches_java_dot_plus(s: &str) -> bool {
    !s.is_empty()
        && !s
            .chars()
            .any(|c| matches!(c, '\n' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}'))
}

/// Splits `[^/]+/` off the front of `s`: the (non-empty) segment and the rest after the slash.
fn split_segment(s: &str) -> Option<(&str, &str)> {
    let slash = s.find('/')?;
    if slash == 0 {
        return None;
    }
    Some((&s[..slash], &s[slash + 1..]))
}

/// `SIBLING_LAYOUT_GENERATED_FILE_RUNFILES_PATH_PATTERN`:
/// `(?:bazel|blaze)-out/(?:(?<repo>[^/]+(?=/[^/]+-[^/]+/)(?!/coverage-metadata/))/)?[^/]+/[^/]+/(?<path>.+)`
///
/// Examples:
/// * bazel-out/k8-fastbuild/bin/pkg/file.txt (repo: null, path: "pkg/file.txt")
/// * bazel-out/some_repo/k8-fastbuild/bin/pkg/file.txt (repo: "some_repo", path: "pkg/file.txt")
/// * bazel-out/k8-fastbuild/k8-fastbuild/bin/pkg/file.txt (repo: "k8-fastbuild", path:
///   "pkg/file.txt")
///
/// Repo names are distinguished from mnemonics via a positive lookahead on the following
/// segment, which in the case of a repo name is a mnemonic and thus contains a hyphen, whereas a
/// mnemonic is followed by an output directory name, which does not contain a hyphen unless it is
/// "coverage-metadata" (which in turn is not likely to be a mnemonic).
///
/// PORT: the `regex` crate has no lookaround; this matcher follows the backtracking matcher's
/// choices (the optional repo group is tried first).
// Java: com/google/devtools/build/lib/exec/SpawnLogReconstructor.java:76 SpawnLogReconstructor.SIBLING_LAYOUT_GENERATED_FILE_RUNFILES_PATH_PATTERN
fn match_sibling_layout_generated_file(exec_path: &str) -> Option<RunfilesPathMatch> {
    let rest = exec_path
        .strip_prefix("bazel-out/")
        .or_else(|| exec_path.strip_prefix("blaze-out/"))?;

    // `[^/]+/[^/]+/(?<path>.+)`
    let tail = |s: &str| -> Option<String> {
        let (_, s) = split_segment(s)?;
        let (_, path) = split_segment(s)?;
        matches_java_dot_plus(path).then(|| path.to_string())
    };

    // With the repo group.
    if let Some((repo, after_repo)) = split_segment(rest) {
        // `(?=/[^/]+-[^/]+/)`: the next segment contains an inner hyphen and is followed by '/'.
        let lookahead = split_segment(after_repo).is_some_and(|(next, _)| {
            next.char_indices()
                .any(|(i, c)| c == '-' && i > 0 && i + 1 < next.len())
        });
        // `(?!/coverage-metadata/)`
        let negative_lookahead = after_repo.starts_with("coverage-metadata/");
        if lookahead
            && !negative_lookahead
            && let Some(path) = tail(after_repo)
        {
            return Some(RunfilesPathMatch {
                repo: Some(repo.to_string()),
                path,
            });
        }
    }

    // Without the repo group.
    tail(rest).map(|path| RunfilesPathMatch { repo: None, path })
}

/// The `repo` and `path` groups of a runfiles path match.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RunfilesPathMatch {
    /// `group("repo")`.
    pub repo: Option<String>,
    /// `group("path")`.
    pub path: String,
}

/// Represents a reconstructed input file, symlink, or directory.
// Java: com/google/devtools/build/lib/exec/SpawnLogReconstructor.java:89 SpawnLogReconstructor.Input
#[derive(Clone, Debug)]
enum Input {
    File(File),
    Symlink(File),
    Directory { path: String, files: Arc<Vec<File>> },
}

impl Input {
    fn path(&self) -> &str {
        match self {
            Input::File(file) => &file.path,
            Input::Symlink(symlink) => &symlink.path,
            Input::Directory { path, .. } => path,
        }
    }
}

/// An entry of `inputMap`: an input (files and symlinks unwrapped to their `File`), an input set
/// or a set of symlink entries.
#[derive(Debug)]
enum StoredEntry {
    File(File),
    Directory { path: String, files: Arc<Vec<File>> },
    InputSet(Arc<exec_log_entry::InputSet>),
    SymlinkEntrySet(Arc<exec_log_entry::SymlinkEntrySet>),
}

/// Reconstructs an execution log in expanded format from the compact format representation.
// Java: com/google/devtools/build/lib/exec/SpawnLogReconstructor.java:49 SpawnLogReconstructor
pub struct SpawnLogReconstructor {
    input: Option<zstd::stream::read::Decoder<'static, BufReader<Box<dyn Read + Send>>>>,

    // Stores both Inputs and InputSets. Bazel uses consecutive IDs starting from 1, so we can use
    // a Vec to store them together efficiently.
    input_map: Vec<Option<StoredEntry>>,
    hash_function_name: String,
    workspace_runfiles_directory: String,
    sibling_repository_layout: bool,
}

impl SpawnLogReconstructor {
    // Java: com/google/devtools/build/lib/exec/SpawnLogReconstructor.java:116 SpawnLogReconstructor.<init>
    pub fn new(input: Box<dyn Read + Send>) -> io::Result<SpawnLogReconstructor> {
        Ok(SpawnLogReconstructor {
            input: Some(zstd::stream::read::Decoder::new(input)?),
            // Add a null entry for the 0th index as IDs are 1-based.
            input_map: vec![None],
            hash_function_name: String::new(),
            workspace_runfiles_directory: String::new(),
            sibling_repository_layout: false,
        })
    }

    // Java: com/google/devtools/build/lib/exec/SpawnLogReconstructor.java:124 SpawnLogReconstructor.read
    fn read_impl(&mut self) -> io::Result<Option<SpawnExec>> {
        loop {
            let input = self
                .input
                .as_mut()
                .ok_or_else(|| io::Error::other("Stream closed"))?;
            let Some(entry) = parse_delimited_from::<ExecLogEntry>(input)? else {
                return Ok(None);
            };
            let id = entry.id;
            match entry.r#type {
                Some(exec_log_entry::Type::Invocation(invocation)) => {
                    self.hash_function_name = invocation.hash_function_name;
                    self.workspace_runfiles_directory = invocation.workspace_runfiles_directory;
                    self.sibling_repository_layout = invocation.sibling_repository_layout;
                }
                Some(exec_log_entry::Type::File(file)) => {
                    let input = self.reconstruct_file_entry(&file);
                    self.put_input(id, input)?;
                }
                Some(exec_log_entry::Type::Directory(dir)) => {
                    let input = self.reconstruct_dir(&dir);
                    self.put_input(id, input)?;
                }
                Some(exec_log_entry::Type::UnresolvedSymlink(symlink)) => {
                    self.put_input(id, Self::reconstruct_symlink(symlink))?;
                }
                Some(exec_log_entry::Type::RunfilesTree(runfiles_tree)) => {
                    let input = self.reconstruct_runfiles_dir(&runfiles_tree)?;
                    self.put_input(id, input)?;
                }
                Some(exec_log_entry::Type::InputSet(input_set)) => {
                    self.put_input_set(id, input_set)?;
                }
                Some(exec_log_entry::Type::SymlinkEntrySet(symlink_entry_set)) => {
                    self.put_symlink_entry_set(id, symlink_entry_set)?;
                }
                Some(exec_log_entry::Type::Spawn(spawn)) => {
                    return Ok(Some(self.reconstruct_spawn_exec(spawn)?));
                }
                Some(exec_log_entry::Type::SymlinkAction(_)) => {
                    // Symlink actions are not represented in the expanded format.
                }
                None => {
                    // `TYPE_NOT_SET` has number 0.
                    return Err(io::Error::other(format!("unknown entry type {}", 0)));
                }
            }
        }
    }

    // Java: com/google/devtools/build/lib/exec/SpawnLogReconstructor.java:155 SpawnLogReconstructor.reconstructSpawnExec
    fn reconstruct_spawn_exec(&self, entry: exec_log_entry::Spawn) -> io::Result<SpawnExec> {
        let mut builder = SpawnExec {
            command_args: entry.args,
            environment_variables: entry.env_vars,
            target_label: entry.target_label,
            mnemonic: entry.mnemonic,
            exit_code: entry.exit_code,
            status: entry.status,
            runner: entry.runner,
            cache_hit: entry.cache_hit,
            remotable: entry.remotable,
            cacheable: entry.cacheable,
            remote_cacheable: entry.remote_cacheable,
            timeout_millis: entry.timeout_millis,
            ..Default::default()
        };

        if entry.metrics.is_some() {
            builder.metrics = entry.metrics;
        }

        if entry.platform.is_some() {
            builder.platform = entry.platform;
        }

        let mut inputs: BTreeMap<JavaOrderedString, File> = BTreeMap::new();
        self.visit_input_set(
            entry.input_set_id,
            &mut |file: &File| {
                inputs.insert(JavaOrderedString(file.path.clone()), file.clone());
            },
            &mut |_input: &Input| {},
        )?;
        let mut tool_inputs: HashSet<String> = HashSet::new();
        self.visit_input_set(
            entry.tool_set_id,
            &mut |file: &File| {
                tool_inputs.insert(file.path.clone());
            },
            &mut |_input: &Input| {},
        )?;

        for (path, mut file) in inputs {
            if tool_inputs.contains(&path.0) {
                file.is_tool = true;
            }
            builder.inputs.push(file);
        }

        let mut listed_outputs: BTreeSet<JavaOrderedString> = BTreeSet::new();

        for output in &entry.outputs {
            match &output.r#type {
                Some(exec_log_entry::output::Type::OutputId(output_id)) => {
                    let input = self.get_input(*output_id)?;
                    listed_outputs.insert(JavaOrderedString(input.path().to_string()));
                    match input {
                        Input::File(file) => builder.actual_outputs.push(file),
                        Input::Symlink(symlink) => builder.actual_outputs.push(symlink),
                        Input::Directory { files, .. } => {
                            builder.actual_outputs.extend(files.iter().cloned())
                        }
                    }
                }
                Some(exec_log_entry::output::Type::InvalidOutputPath(invalid_output_path)) => {
                    listed_outputs.insert(JavaOrderedString(invalid_output_path.clone()));
                }
                None => {
                    // `TYPE_NOT_SET` has number 0.
                    return Err(io::Error::other(format!("unknown output type {}", 0)));
                }
            }
        }

        builder
            .listed_outputs
            .extend(listed_outputs.into_iter().map(|s| s.0));

        if let Some(mut digest) = entry.digest {
            digest.hash_function_name = self.hash_function_name.clone();
            builder.digest = Some(digest);
        }

        Ok(builder)
    }

    /// `HashMap.merge(key, 0, (oldValue, newValue) -> 1)`.
    fn merge_visit_count(previous_visit_count: &mut HashMap<u32, i32>, key: u32) -> i32 {
        match previous_visit_count.entry(key) {
            HashEntry::Vacant(v) => {
                v.insert(0);
                0
            }
            HashEntry::Occupied(mut o) => {
                o.insert(1);
                1
            }
        }
    }

    // Java: com/google/devtools/build/lib/exec/SpawnLogReconstructor.java:222 SpawnLogReconstructor.visitInputSet
    fn visit_input_set(
        &self,
        input_set_id: u32,
        visit_file: &mut dyn FnMut(&File),
        visit_input: &mut dyn FnMut(&Input),
    ) -> io::Result<()> {
        if input_set_id == 0 {
            return Ok(());
        }
        let mut sets_to_visit: Vec<u32> = Vec::new();
        let mut previous_visit_count: HashMap<u32, i32> = HashMap::new();
        sets_to_visit.push(input_set_id);
        while let Some(current_set_id) = sets_to_visit.pop() {
            // In case order matters (it does for runfiles, but not for inputs), we visit the set
            // in post-order (corresponds to Order#COMPILE_ORDER). Transitive sets are visited
            // before direct children; both are visited in left-to-right order.
            match Self::merge_visit_count(&mut previous_visit_count, current_set_id) {
                0 => {
                    // First visit, queue transitive sets for visit before revisiting the current
                    // set.
                    sets_to_visit.push(current_set_id);
                    for &transitive_set_id in self
                        .get_input_set(current_set_id)?
                        .transitive_set_ids
                        .iter()
                        .rev()
                    {
                        if !previous_visit_count.contains_key(&transitive_set_id) {
                            sets_to_visit.push(transitive_set_id);
                        }
                    }
                }
                1 => {
                    // Second visit, visit the direct inputs only.
                    for &input_id in &self.get_input_set(current_set_id)?.input_ids {
                        if previous_visit_count.insert(input_id, 1).is_some() {
                            continue;
                        }
                        let input = self.get_input(input_id)?;
                        visit_input(&input);
                        match &input {
                            Input::File(file) => visit_file(file),
                            Input::Symlink(symlink) => visit_file(symlink),
                            Input::Directory { files, .. } => {
                                files.iter().for_each(&mut *visit_file)
                            }
                        }
                    }
                }
                _ => panic!(
                    "IllegalStateException: expected visit count to be 0 or 1, was {}",
                    previous_visit_count[&current_set_id]
                ),
            }
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/exec/SpawnLogReconstructor.java:269 SpawnLogReconstructor.visitSymlinkEntries
    fn visit_symlink_entries(
        &self,
        runfiles_tree: &exec_log_entry::RunfilesTree,
        root_symlinks: bool,
        entry_consumer: &mut dyn FnMut(&str, Vec<File>),
    ) -> io::Result<()> {
        let symlink_entry_set_id = if root_symlinks {
            runfiles_tree.root_symlinks_id
        } else {
            runfiles_tree.symlinks_id
        };
        if symlink_entry_set_id == 0 {
            return Ok(());
        }
        let mut sets_to_visit: Vec<u32> = Vec::new();
        let mut previous_visit_count: HashMap<u32, i32> = HashMap::new();
        sets_to_visit.push(symlink_entry_set_id);
        while let Some(current_set_id) = sets_to_visit.pop() {
            // As order matters, we visit the set in post-order (corresponds to
            // Order#COMPILE_ORDER). Transitive sets are visited before direct children; both are
            // visited in left-to-right order.
            match Self::merge_visit_count(&mut previous_visit_count, current_set_id) {
                0 => {
                    // First visit, queue transitive sets for visit before revisiting the current
                    // set.
                    sets_to_visit.push(current_set_id);
                    for &transitive_set_id in self
                        .get_symlink_entry_set(current_set_id)?
                        .transitive_set_ids
                        .iter()
                        .rev()
                    {
                        if !previous_visit_count.contains_key(&transitive_set_id) {
                            sets_to_visit.push(transitive_set_id);
                        }
                    }
                }
                1 => {
                    // Second visit, visit the direct entries only.
                    for path_and_input_id in
                        &self.get_symlink_entry_set(current_set_id)?.direct_entries
                    {
                        let key = &path_and_input_id.key;
                        let runfiles_tree_relative_path = if root_symlinks {
                            key.clone()
                        } else if let Some(stripped) = key.strip_prefix("../") {
                            stripped.to_string()
                        } else {
                            format!("{}/{}", self.workspace_runfiles_directory, key)
                        };
                        let path =
                            format!("{}/{}", runfiles_tree.path, runfiles_tree_relative_path);
                        entry_consumer(
                            &runfiles_tree_relative_path,
                            self.reconstruct_runfiles_symlink_target(
                                &path,
                                path_and_input_id.value,
                            )?,
                        );
                    }
                }
                _ => panic!(
                    "IllegalStateException: expected visit count to be 0 or 1, was {}",
                    previous_visit_count[&current_set_id]
                ),
            }
        }
        Ok(())
    }

    // Java: com/google/devtools/build/lib/exec/SpawnLogReconstructor.java:323 SpawnLogReconstructor.reconstructDir
    fn reconstruct_dir(&self, dir: &exec_log_entry::Directory) -> Input {
        let files = dir
            .files
            .iter()
            .map(|dir_file| self.reconstruct_file(Some(dir), dir_file))
            .collect();
        Input::Directory {
            path: dir.path.clone(),
            files: Arc::new(files),
        }
    }

    // Java: com/google/devtools/build/lib/exec/SpawnLogReconstructor.java:332 SpawnLogReconstructor.reconstructFile
    fn reconstruct_file_entry(&self, entry: &exec_log_entry::File) -> Input {
        Input::File(self.reconstruct_file(None, entry))
    }

    // Java: com/google/devtools/build/lib/exec/SpawnLogReconstructor.java:336 SpawnLogReconstructor.reconstructFile
    fn reconstruct_file(
        &self,
        parent_dir: Option<&exec_log_entry::Directory>,
        entry: &exec_log_entry::File,
    ) -> File {
        let mut builder = File {
            path: match parent_dir {
                Some(parent_dir) => format!("{}/{}", parent_dir.path, entry.path),
                None => entry.path.clone(),
            },
            ..Default::default()
        };
        if let Some(digest) = &entry.digest {
            let mut digest = digest.clone();
            digest.hash_function_name = self.hash_function_name.clone();
            builder.digest = Some(digest);
        }
        builder
    }

    // Java: com/google/devtools/build/lib/exec/SpawnLogReconstructor.java:347 SpawnLogReconstructor.reconstructSymlink
    fn reconstruct_symlink(entry: exec_log_entry::UnresolvedSymlink) -> Input {
        Input::Symlink(File {
            path: entry.path,
            symlink_target_path: entry.target_path,
            ..Default::default()
        })
    }

    // Java: com/google/devtools/build/lib/exec/SpawnLogReconstructor.java:355 SpawnLogReconstructor.reconstructRunfilesDir
    fn reconstruct_runfiles_dir(
        &self,
        runfiles_tree: &exec_log_entry::RunfilesTree,
    ) -> io::Result<Input> {
        // In case of path collisions, runfiles should be collected in the following order, with
        // later sources overriding earlier ones (see
        // com.google.devtools.build.lib.analysis.Runfiles#getRunfilesInputs):
        //
        // 1. symlinks
        // 2. artifacts at canonical locations
        // 3. empty files
        // 4. root symlinks
        // 5. the _repo_mapping file with the repo mapping manifest
        // 6. the <workspace runfiles directory>/.runfile file (if the workspace runfiles directory
        //    wouldn't exist otherwise)
        //
        // Within each group represented by a nested set, the entries are traversed in postorder
        // (i.e. the transitive sets are visited before the direct children). This is important to
        // resolve conflicts in the same order as the real Runfiles implementation.
        let mut runfiles: IndexMap<String, File> = IndexMap::new();
        let mut has_workspace_runfiles_directory = false;
        let workspace_prefix = format!("{}/", self.workspace_runfiles_directory);

        self.visit_symlink_entries(
            runfiles_tree,
            /* root_symlinks= */ false,
            &mut |root_relative_path: &str, files: Vec<File>| {
                has_workspace_runfiles_directory |=
                    root_relative_path.starts_with(&workspace_prefix);
                for file in files {
                    runfiles.insert(file.path.clone(), file);
                }
            },
        )?;

        let mut flattened_artifacts: IndexSet<File> = IndexSet::new();
        self.visit_input_set(
            runfiles_tree.input_set_id,
            &mut |file: &File| {
                flattened_artifacts.insert(file.clone());
            },
            // This is bug-for-bug compatible with the implementation in Runfiles by considering
            // an empty non-external directory as a runfiles entry under the workspace runfiles
            // directory even though it won't be materialized as one.
            &mut |input: &Input| {
                has_workspace_runfiles_directory |=
                    self.has_workspace_runfiles_directory(input.path());
            },
        )?;
        for file in &flattened_artifacts {
            for relative_path in self.get_runfiles_paths(&file.path) {
                let mut file = file.clone();
                file.path = format!("{}/{}", runfiles_tree.path, relative_path);
                runfiles.insert(file.path.clone(), file);
            }
        }

        for empty_file in &runfiles_tree.empty_files {
            // Empty files are only created as siblings or parents of existing files, so they
            // can't by themselves create a workspace runfiles directory if it wouldn't exist
            // otherwise.
            let new_path = match empty_file.strip_prefix("../") {
                Some(stripped) => format!("{}/{}", runfiles_tree.path, stripped),
                None => format!(
                    "{}/{}/{}",
                    runfiles_tree.path, self.workspace_runfiles_directory, empty_file
                ),
            };
            runfiles.insert(
                new_path.clone(),
                File {
                    path: new_path,
                    ..Default::default()
                },
            );
        }

        self.visit_symlink_entries(
            runfiles_tree,
            /* root_symlinks= */ true,
            &mut |root_relative_path: &str, files: Vec<File>| {
                has_workspace_runfiles_directory |=
                    root_relative_path.starts_with(&workspace_prefix);
                for file in files {
                    runfiles.insert(file.path.clone(), file);
                }
            },
        )?;

        if let Some(repo_mapping_manifest) = &runfiles_tree.repo_mapping_manifest {
            runfiles.insert(
                REPO_MAPPING_MANIFEST.to_string(),
                File {
                    path: format!("{}/{}", runfiles_tree.path, REPO_MAPPING_MANIFEST),
                    digest: Some(repo_mapping_manifest.digest.clone().unwrap_or_default()),
                    ..Default::default()
                },
            );
        }

        if !has_workspace_runfiles_directory {
            let dot_runfile_path = format!(
                "{}/{}/.runfile",
                runfiles_tree.path, self.workspace_runfiles_directory
            );
            runfiles.insert(
                dot_runfile_path.clone(),
                File {
                    path: dot_runfile_path,
                    ..Default::default()
                },
            );
        }
        // Copy to avoid retaining the entire runfiles map.
        Ok(Input::Directory {
            path: runfiles_tree.path.clone(),
            files: Arc::new(runfiles.into_values().collect()),
        })
    }

    /// The `repo` and `path` groups of the runfiles path patterns for `exec_path`.
    // Java: com/google/devtools/build/lib/exec/SpawnLogReconstructor.java:447 SpawnLogReconstructor.extractRunfilesPath
    pub fn extract_runfiles_path(
        exec_path: &str,
        sibling_repository_layout: bool,
    ) -> RunfilesPathMatch {
        let generated = if sibling_repository_layout {
            match_sibling_layout_generated_file(exec_path)
        } else {
            regex_match(&DEFAULT_GENERATED_FILE_RUNFILES_PATH_PATTERN, exec_path)
        };
        if let Some(m) = generated {
            return m;
        }
        let source = regex_match(
            if sibling_repository_layout {
                &SIBLING_LAYOUT_SOURCE_FILE_RUNFILES_PATH_PATTERN
            } else {
                &DEFAULT_SOURCE_FILE_RUNFILES_PATH_PATTERN
            },
            exec_path,
        );
        match source {
            Some(m) => m,
            None => panic!("IllegalStateException"),
        }
    }

    // Java: com/google/devtools/build/lib/exec/SpawnLogReconstructor.java:465 SpawnLogReconstructor.hasWorkspaceRunfilesDirectory
    fn has_workspace_runfiles_directory(&self, path: &str) -> bool {
        Self::extract_runfiles_path(path, self.sibling_repository_layout)
            .repo
            .is_none()
    }

    // Java: com/google/devtools/build/lib/exec/SpawnLogReconstructor.java:469 SpawnLogReconstructor.getRunfilesPaths
    fn get_runfiles_paths(&self, exec_path: &str) -> Vec<String> {
        let match_result = Self::extract_runfiles_path(exec_path, self.sibling_repository_layout);
        match match_result.repo {
            None => vec![format!(
                "{}/{}",
                self.workspace_runfiles_directory, match_result.path
            )],
            Some(repo) => vec![format!("{}/{}", repo, match_result.path)],
        }
    }

    // Java: com/google/devtools/build/lib/exec/SpawnLogReconstructor.java:482 SpawnLogReconstructor.reconstructRunfilesSymlinkTarget
    fn reconstruct_runfiles_symlink_target(
        &self,
        new_path: &str,
        target_id: u32,
    ) -> io::Result<Vec<File>> {
        if target_id == 0 {
            return Ok(vec![File {
                path: new_path.to_string(),
                ..Default::default()
            }]);
        }
        Ok(match self.get_input(target_id)? {
            Input::File(mut file) => {
                file.path = new_path.to_string();
                vec![file]
            }
            Input::Symlink(mut symlink) => {
                symlink.path = new_path.to_string();
                vec![symlink]
            }
            Input::Directory { path, files } => files
                .iter()
                .map(|file| {
                    let mut file = file.clone();
                    file.path = format!("{}{}", new_path, &file.path[path.len()..]);
                    file
                })
                .collect(),
        })
    }

    // Java: com/google/devtools/build/lib/exec/SpawnLogReconstructor.java:502 SpawnLogReconstructor.putInput
    fn put_input(&mut self, id: u32, input: Input) -> io::Result<()> {
        // Unwrap trivial wrappers to reduce retained memory usage.
        let entry = match input {
            Input::File(file) => StoredEntry::File(file),
            Input::Symlink(symlink) => StoredEntry::File(symlink),
            Input::Directory { path, files } => StoredEntry::Directory { path, files },
        };
        self.put_entry(id, entry)
    }

    // Java: com/google/devtools/build/lib/exec/SpawnLogReconstructor.java:506 SpawnLogReconstructor.putInputSet
    fn put_input_set(&mut self, id: u32, input_set: exec_log_entry::InputSet) -> io::Result<()> {
        self.put_entry(id, StoredEntry::InputSet(Arc::new(input_set)))
    }

    // Java: com/google/devtools/build/lib/exec/SpawnLogReconstructor.java:510 SpawnLogReconstructor.putSymlinkEntrySet
    fn put_symlink_entry_set(
        &mut self,
        id: u32,
        symlink_entries: exec_log_entry::SymlinkEntrySet,
    ) -> io::Result<()> {
        self.put_entry(id, StoredEntry::SymlinkEntrySet(Arc::new(symlink_entries)))
    }

    // Java: com/google/devtools/build/lib/exec/SpawnLogReconstructor.java:515 SpawnLogReconstructor.putEntry
    fn put_entry(&mut self, id: u32, entry: StoredEntry) -> io::Result<()> {
        if id == 0 {
            // The entry won't be referenced, so we don't need to store it.
            return Ok(());
        }
        // Bazel emits consecutive non-zero IDs.
        if id as usize != self.input_map.len() {
            return Err(io::Error::other(format!(
                "ids must be consecutive, got {} after {}",
                id,
                self.input_map.len()
            )));
        }
        self.input_map.push(Some(entry));
        Ok(())
    }

    /// `ArrayList.get(id)`.
    fn get_entry(&self, id: u32) -> Option<&StoredEntry> {
        match self.input_map.get(id as usize) {
            Some(entry) => entry.as_ref(),
            None => panic!(
                "IndexOutOfBoundsException: Index {} out of bounds for length {}",
                id,
                self.input_map.len()
            ),
        }
    }

    // Java: com/google/devtools/build/lib/exec/SpawnLogReconstructor.java:534 SpawnLogReconstructor.getInput
    fn get_input(&self, id: u32) -> io::Result<Input> {
        // PORT: `%s` of a non-input entry is the proto's text format in Java; Debug here.
        match self.get_entry(id) {
            Some(StoredEntry::File(file)) => Ok(if file.symlink_target_path.is_empty() {
                Input::File(file.clone())
            } else {
                Input::Symlink(file.clone())
            }),
            Some(StoredEntry::Directory { path, files }) => Ok(Input::Directory {
                path: path.clone(),
                files: Arc::clone(files),
            }),
            None => Err(io::Error::other(format!(
                "referenced input {id} is missing"
            ))),
            Some(value) => Err(io::Error::other(format!(
                "entry {id} is not an input: {value:?}"
            ))),
        }
    }

    // Java: com/google/devtools/build/lib/exec/SpawnLogReconstructor.java:545 SpawnLogReconstructor.getInputSet
    fn get_input_set(&self, id: u32) -> io::Result<Arc<exec_log_entry::InputSet>> {
        match self.get_entry(id) {
            Some(StoredEntry::InputSet(input_set)) => Ok(Arc::clone(input_set)),
            None => Err(io::Error::other(format!(
                "referenced input set {id} is missing"
            ))),
            Some(value) => Err(io::Error::other(format!(
                "entry {id} is not an input set: {value:?}"
            ))),
        }
    }

    // Java: com/google/devtools/build/lib/exec/SpawnLogReconstructor.java:554 SpawnLogReconstructor.getSymlinkEntrySet
    fn get_symlink_entry_set(&self, id: u32) -> io::Result<Arc<exec_log_entry::SymlinkEntrySet>> {
        match self.get_entry(id) {
            Some(StoredEntry::SymlinkEntrySet(symlink_entries)) => Ok(Arc::clone(symlink_entries)),
            None => Err(io::Error::other(format!(
                "referenced set of symlink entries {id} is missing"
            ))),
            Some(value) => Err(io::Error::other(format!(
                "entry {id} is not a set of symlink entries: {value:?}"
            ))),
        }
    }
}

fn regex_match(pattern: &Regex, exec_path: &str) -> Option<RunfilesPathMatch> {
    let captures = pattern.captures(exec_path)?;
    Some(RunfilesPathMatch {
        repo: captures.name("repo").map(|m| m.as_str().to_string()),
        path: captures
            .name("path")
            .map(|m| m.as_str().to_string())
            .unwrap_or_default(),
    })
}

impl MessageInputStream<SpawnExec> for SpawnLogReconstructor {
    fn read(&mut self) -> io::Result<Option<SpawnExec>> {
        self.read_impl()
    }

    // Java: com/google/devtools/build/lib/exec/SpawnLogReconstructor.java:567 SpawnLogReconstructor.close
    fn close(&mut self) -> io::Result<()> {
        self.input = None;
        Ok(())
    }
}
