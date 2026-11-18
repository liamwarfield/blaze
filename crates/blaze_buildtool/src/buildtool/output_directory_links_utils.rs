//! Port of `buildtool/OutputDirectoryLinksUtils.java` (the convenience symlinks `bazel-bin`,
//! `bazel-testlogs`, `bazel-genfiles`, `bazel-out` and `bazel-<workspace>`).

use std::sync::{Arc, LazyLock};

use blaze_build_config::build_configuration_value::BuildConfigurationValue;
use blaze_cmdline::RepositoryName;
use blaze_cmdline::repository_name::MAIN;
use blaze_events::{Event, EventHandler};
use blaze_optdefs::command::build_request_options::{BuildRequestOptions, ConvenienceSymlinksMode};
use blaze_protos::build_event_stream::ConvenienceSymlink;
use blaze_protos::build_event_stream::convenience_symlink::Action;
use blaze_vfs::{Path, PathFragment};
use indexmap::{IndexMap, IndexSet};

use super::config_symlink::{ConfigSymlink, SymlinkDefinition};

/// The parts of `BlazeDirectories` that `OutputDirectoryLinksUtils` reads.
///
/// PORT: `BlazeDirectories` is ported in `blaze_runtime`, which sits above this crate;
/// `blaze_runtime` implements this trait for it.
// Java: com/google/devtools/build/lib/analysis/BlazeDirectories.java:71 BlazeDirectories
pub trait BlazeDirectoriesView {
    // Java: com/google/devtools/build/lib/analysis/BlazeDirectories.java:186 BlazeDirectories.getExecRoot
    fn get_exec_root(&self, workspace_name: &str) -> Path;
    // Java: com/google/devtools/build/lib/analysis/BlazeDirectories.java:213 BlazeDirectories.getOutputPath
    fn get_output_path(&self, workspace_name: &str) -> Path;
    // Java: com/google/devtools/build/lib/analysis/BlazeDirectories.java:163 BlazeDirectories.getOutputBase
    fn get_output_base(&self) -> Path;
    // Java: com/google/devtools/build/lib/analysis/BlazeDirectories.java:177 BlazeDirectories.getBlazeExecRoot
    fn get_blaze_exec_root(&self) -> Path;
}

/// `BuildRequestOptions.getSymlinkPrefix`.
///
/// PORT: `BuildRequestOptions` is transcribed in `blaze_optdefs`, which holds no hand-written
/// methods of it.
// Java: com/google/devtools/build/lib/buildtool/BuildRequestOptions.java:357 BuildRequestOptions.getSymlinkPrefix
pub fn get_symlink_prefix(options: &BuildRequestOptions, product_name: &str) -> String {
    match &options.symlink_prefix {
        None => format!("{}-", product_name),
        Some(prefix) => prefix.clone(),
    }
}

/// Returns all (types of) convenience symlinks that may be created.
///
/// Note that this is independent of which symlinks are actually requested by the build options;
/// that's controlled by returning no candidates in `SymlinkDefinition.getLinkPaths`.
///
/// The order of the result indicates precedence for `PathPrettyPrinter`.
// Java: com/google/devtools/build/lib/buildtool/OutputDirectoryLinksUtils.java:56 OutputDirectoryLinksUtils.getAllLinkDefinitions
fn get_all_link_definitions(
    symlink_definitions: &[Arc<dyn SymlinkDefinition>],
) -> Vec<Arc<dyn SymlinkDefinition>> {
    let mut builder: Vec<Arc<dyn SymlinkDefinition>> = Vec::new();
    builder.extend(STANDARD_LINK_DEFINITIONS.iter().cloned());
    builder.extend(symlink_definitions.iter().cloned());
    builder
}

// Java: com/google/devtools/build/lib/buildtool/OutputDirectoryLinksUtils.java:64 OutputDirectoryLinksUtils.NO_CREATE_SYMLINKS_PREFIX
const NO_CREATE_SYMLINKS_PREFIX: &str = "/";

/// Attempts to create or delete convenience symlinks in the workspace to the various output
/// directories, and generates associated log events.
///
/// If `--symlink_prefix` is `NO_CREATE_SYMLINKS_PREFIX`, or `--experimental_convenience_symlinks`
/// is `IGNORE`, this method is a no-op.
///
/// Otherwise, for each symlink type, we decide whether the symlink should exist or not. If it
/// should exist, it is created with the appropriate destination path; if not, it is deleted if
/// already present on the file system. In either case, the decision of whether to create or delete
/// the symlink is logged. (Note that deleting pre-existing symlinks helps ensure the user's
/// workspace is in a consistent state after the build. However, if the `--symlink_prefix` has
/// changed, we have no way to cleanup old symlink names leftover from a previous invocation.)
///
/// If `--experimental_convenience_symlinks` is set to `CLEAN`, all symlinks are set to be
/// deleted. If it's set to `NORMAL`, each symlink type decides whether it should be created or
/// deleted. (A symlink may decide to be deleted if e.g. it is disabled by a flag, or would want to
/// point to more than one destination.) If it's set to `LOG_ONLY`, the same logic is run as in the
/// `NORMAL` case, but the result is only emitting log messages, with no actual filesystem
/// mutations.
///
/// A warning is emitted if a symlink would resolve to multiple destinations, or if a filesystem
/// mutation operation fails.
///
/// PORT: Java's `Set<BuildConfigurationValue> targetConfigs` is a slice of distinct
/// configurations in the caller's order.
// Java: com/google/devtools/build/lib/buildtool/OutputDirectoryLinksUtils.java:92 OutputDirectoryLinksUtils.createOutputDirectoryLinks
pub fn create_output_directory_links(
    symlink_definitions: &[Arc<dyn SymlinkDefinition>],
    build_request_options: &BuildRequestOptions,
    workspace_name: &str,
    workspace: &Path,
    directories: &dyn BlazeDirectoriesView,
    event_handler: &dyn EventHandler,
    target_configs: &[Arc<BuildConfigurationValue>],
    product_name: &str,
) -> SymlinkCreationResult {
    let exec_root = directories.get_exec_root(workspace_name);
    let output_path = directories.get_output_path(workspace_name);
    let symlink_prefix = get_symlink_prefix(build_request_options, product_name);
    let mode = build_request_options.experimental_convenience_symlinks;
    if NO_CREATE_SYMLINKS_PREFIX == symlink_prefix {
        return SymlinkCreationResult::empty();
    }

    let mut convenience_symlinks_builder: Vec<ConvenienceSymlink> = Vec::new();
    let mut created_convenience_symlinks_builder: IndexMap<PathFragment, PathFragment> =
        IndexMap::new();
    let mut failures: Vec<String> = Vec::new();
    let mut ambiguous_links: Vec<String> = Vec::new();
    let mut created_links: IndexSet<String> = IndexSet::new();
    let workspace_base_name = workspace.get_base_name();
    let repository_name: &RepositoryName = &MAIN;
    let log_only = mode == ConvenienceSymlinksMode::LOG_ONLY;

    for symlink in get_all_link_definitions(symlink_definitions) {
        let link_name = symlink.get_link_name(&symlink_prefix, workspace_base_name);
        if !created_links.insert(link_name.clone()) {
            // already created a link by this name
            continue;
        }
        if mode == ConvenienceSymlinksMode::CLEAN {
            remove_link(
                workspace,
                &link_name,
                &mut failures,
                &mut convenience_symlinks_builder,
                log_only,
            );
        } else {
            let candidate_paths = symlink.get_link_paths(
                build_request_options,
                target_configs,
                repository_name,
                &output_path,
                &exec_root,
            );
            if candidate_paths.len() == 1 {
                create_link(
                    workspace,
                    &link_name,
                    &exec_root,
                    directories,
                    candidate_paths.get_index(0).unwrap(),
                    &mut failures,
                    &mut convenience_symlinks_builder,
                    &mut created_convenience_symlinks_builder,
                    log_only,
                );
            } else {
                remove_link(
                    workspace,
                    &link_name,
                    &mut failures,
                    &mut convenience_symlinks_builder,
                    log_only,
                );
                // candidatePaths can be empty if the symlink decided not to be created. This can
                // happen if the symlink is disabled by a flag, or it intercepts an error while
                // computing its target path. In that case, don't trigger a warning about an
                // ambiguous link.
                if candidate_paths.len() > 1 {
                    ambiguous_links.push(link_name);
                }
            }
        }
    }

    if !failures.is_empty() {
        event_handler.handle(&Event::warn(format!(
            "failed to create one or more convenience symlinks for prefix '{}':\n  {}",
            symlink_prefix,
            failures.join("\n  ")
        )));
    }
    if !ambiguous_links.is_empty() {
        event_handler.handle(&Event::warn(format!(
            "cleared convenience symlink(s) {} because they wouldn't contain \
             requested targets' outputs. Those targets self-transition to multiple \
             distinct configurations",
            ambiguous_links.join(", ")
        )));
    }
    // PORT: `buildKeepingLast` keeps the last value at the first key's position, which is what
    // `IndexMap::insert` does.
    SymlinkCreationResult {
        convenience_symlink_protos: convenience_symlinks_builder,
        created_symlinks: created_convenience_symlinks_builder,
    }
}

/// Attempts to remove the convenience symlinks in the workspace directory.
///
/// Issues a warning if it fails, e.g. because workspaceDirectory is readonly. Also cleans up any
/// child directories created by a custom prefix.
///
/// `symlink_definitions` are the extra symlink types added by the `ConfiguredRuleClassProvider`.
// Java: com/google/devtools/build/lib/buildtool/OutputDirectoryLinksUtils.java:183 OutputDirectoryLinksUtils.removeOutputDirectoryLinks
pub fn remove_output_directory_links(
    symlink_definitions: &[Arc<dyn SymlinkDefinition>],
    workspace: &Path,
    event_handler: &dyn EventHandler,
    symlink_prefix: &str,
) {
    if NO_CREATE_SYMLINKS_PREFIX == symlink_prefix {
        return;
    }
    let mut failures: Vec<String> = Vec::new();

    let workspace_base_name = workspace.get_base_name();

    for link in get_all_link_definitions(symlink_definitions) {
        remove_link(
            workspace,
            &link.get_link_name(symlink_prefix, workspace_base_name),
            &mut failures,
            &mut Vec::new(),
            false,
        );
    }

    remove_directory_and_parents(workspace, PathFragment::create(symlink_prefix));
    if !failures.is_empty() {
        event_handler.handle(&Event::warn(format!(
            "failed to remove one or more convenience symlinks for prefix '{}':\n  {}",
            symlink_prefix,
            failures.join("\n  ")
        )));
    }
}

/// Creates a symlink and outputs a `ConvenienceSymlink` entry.
///
/// The symlink is created at path `name`, relative to `base`, creating directories as needed; it
/// points to `target`. Any filesystem errors are appended to `failures`.
///
/// A `ConvenienceSymlink` entry is added to `symlinks_builder` describing the symlink. `exec_root`
/// and `directories` are used to determine the relative target path for this entry.
///
/// If `log_only` is true, the `ConvenienceSymlink` entry is added but no actual filesystem
/// operations are performed.
// Java: com/google/devtools/build/lib/buildtool/OutputDirectoryLinksUtils.java:225 OutputDirectoryLinksUtils.createLink
fn create_link(
    base: &Path,
    name: &str,
    exec_root: &Path,
    directories: &dyn BlazeDirectoriesView,
    target: &Path,
    failures: &mut Vec<String>,
    symlinks_builder: &mut Vec<ConvenienceSymlink>,
    created_symlinks_builder: &mut IndexMap<PathFragment, PathFragment>,
    log_only: bool,
) {
    // The BEP event needs to report a target path relative to the output base. Usually the target
    // is already under the output base, but if the execroot is virtual (only happens in internal
    // blaze, see ModuleFileSystem), we need to rewrite the path using the real execroot.
    let output_base = directories.get_output_base();
    let target_for_event = if target.starts_with(&output_base) {
        target.clone()
    } else {
        directories
            .get_blaze_exec_root()
            .get_relative(&target.relative_to(exec_root))
    };
    symlinks_builder.push(ConvenienceSymlink {
        path: name.to_string(),
        target: target_for_event
            .relative_to(&output_base)
            .get_path_string()
            .to_string(),
        action: Action::Create as i32,
    });

    let name_fragment = PathFragment::create(name);
    if log_only {
        // Still report as created - log-only implies we want to pretend it exists.
        created_symlinks_builder.insert(name_fragment, target.as_fragment().clone());
        return;
    }
    let link = base.get_relative_str(name);
    if let Err(e) = target.create_directory_and_parents() {
        failures.push(format!(
            "cannot create directory {}: {}",
            target.get_path_string(),
            java_io_message(&e)
        ));
        return;
    }
    match ensure_symbolic_link(&link, target.as_fragment()) {
        Ok(()) => {
            created_symlinks_builder.insert(name_fragment, target.as_fragment().clone());
        }
        Err(e) => {
            failures.push(format!(
                "cannot create symbolic link {} -> {}:  {}",
                name,
                target.get_path_string(),
                java_io_message(&e)
            ));
        }
    }
}

/// Deletes a symlink and outputs a `ConvenienceSymlink` entry.
///
/// The symlink to be deleted is at path `name`, relative to `base`. Any filesystem errors are
/// appended to `failures`.
///
/// A `ConvenienceSymlink` entry is added to `symlinks_builder` describing the symlink to be
/// deleted.
///
/// If `log_only` is true, the `ConvenienceSymlink` entry is added but no actual filesystem
/// operations are performed.
// Java: com/google/devtools/build/lib/buildtool/OutputDirectoryLinksUtils.java:285 OutputDirectoryLinksUtils.removeLink
fn remove_link(
    base: &Path,
    name: &str,
    failures: &mut Vec<String>,
    symlinks_builder: &mut Vec<ConvenienceSymlink>,
    log_only: bool,
) {
    symlinks_builder.push(ConvenienceSymlink {
        path: name.to_string(),
        target: String::new(),
        action: Action::Delete as i32,
    });
    if log_only {
        return;
    }
    let link = base.get_relative_str(name);
    if link.is_symbolic_link() {
        // TODO(b/146885821): Consider also removing empty ancestor directories, to allow for
        //  cleaning up directories generated by --symlink_prefix=dir1/dir2/...
        //  Might be undesireable since it could also remove manually-created directories.
        // PORT: skipped, logging (`logger.atFinest().log("Removing %s", link)`).
        if let Err(e) = link.delete() {
            failures.push(format!("{}: {}", name, java_io_message(&e)));
        }
    }
}

/// `STANDARD_LINK_DEFINITIONS`: bin, testlogs, genfiles, the output directory (`bazel-out`) and
/// the execroot (`bazel-<workspace>`).
// Java: com/google/devtools/build/lib/buildtool/OutputDirectoryLinksUtils.java:311 OutputDirectoryLinksUtils.STANDARD_LINK_DEFINITIONS
static STANDARD_LINK_DEFINITIONS: LazyLock<Vec<Arc<dyn SymlinkDefinition>>> = LazyLock::new(|| {
    vec![
        Arc::new(ConfigSymlink::new(
            "bin",
            BuildConfigurationValue::get_bin_directory,
        )),
        Arc::new(ConfigSymlink::new(
            "testlogs",
            BuildConfigurationValue::get_test_logs_directory,
        )),
        Arc::new(GenfilesSymlink(ConfigSymlink::new(
            "genfiles",
            BuildConfigurationValue::get_genfiles_directory,
        ))),
        // output directory (bazel-out)
        Arc::new(OutputSymlink),
        // execroot
        Arc::new(ExecRootSymlink),
    ]
});

/// The `genfiles` `ConfigSymlink`, which `--incompatible_skip_genfiles_symlink` disables.
// Java: com/google/devtools/build/lib/buildtool/OutputDirectoryLinksUtils.java:315 OutputDirectoryLinksUtils.STANDARD_LINK_DEFINITIONS
struct GenfilesSymlink(ConfigSymlink);

impl SymlinkDefinition for GenfilesSymlink {
    fn get_link_name(&self, symlink_prefix: &str, workspace_base_name: &str) -> String {
        self.0.get_link_name(symlink_prefix, workspace_base_name)
    }

    // Java: com/google/devtools/build/lib/buildtool/OutputDirectoryLinksUtils.java:317 OutputDirectoryLinksUtils.STANDARD_LINK_DEFINITIONS.getLinkPaths
    fn get_link_paths(
        &self,
        build_request_options: &BuildRequestOptions,
        target_configs: &[Arc<BuildConfigurationValue>],
        repository_name: &RepositoryName,
        output_path: &Path,
        exec_root: &Path,
    ) -> IndexSet<Path> {
        if build_request_options.incompatible_skip_genfiles_symlink {
            return IndexSet::new();
        }
        self.0.get_link_paths(
            build_request_options,
            target_configs,
            repository_name,
            output_path,
            exec_root,
        )
    }
}

/// Output directory (bazel-out).
// Java: com/google/devtools/build/lib/buildtool/OutputDirectoryLinksUtils.java:331 OutputDirectoryLinksUtils.STANDARD_LINK_DEFINITIONS
struct OutputSymlink;

impl SymlinkDefinition for OutputSymlink {
    // Java: com/google/devtools/build/lib/buildtool/OutputDirectoryLinksUtils.java:333 OutputDirectoryLinksUtils.STANDARD_LINK_DEFINITIONS.getLinkName
    fn get_link_name(&self, symlink_prefix: &str, _workspace_base_name: &str) -> String {
        format!("{}out", symlink_prefix)
    }

    // Java: com/google/devtools/build/lib/buildtool/OutputDirectoryLinksUtils.java:338 OutputDirectoryLinksUtils.STANDARD_LINK_DEFINITIONS.getLinkPaths
    fn get_link_paths(
        &self,
        _build_request_options: &BuildRequestOptions,
        _target_configs: &[Arc<BuildConfigurationValue>],
        _repository_name: &RepositoryName,
        output_path: &Path,
        _exec_root: &Path,
    ) -> IndexSet<Path> {
        IndexSet::from([output_path.clone()])
    }
}

/// Execroot (bazel-<workspace base name>).
// Java: com/google/devtools/build/lib/buildtool/OutputDirectoryLinksUtils.java:348 OutputDirectoryLinksUtils.STANDARD_LINK_DEFINITIONS
struct ExecRootSymlink;

impl SymlinkDefinition for ExecRootSymlink {
    // Java: com/google/devtools/build/lib/buildtool/OutputDirectoryLinksUtils.java:350 OutputDirectoryLinksUtils.STANDARD_LINK_DEFINITIONS.getLinkName
    fn get_link_name(&self, symlink_prefix: &str, workspace_base_name: &str) -> String {
        format!("{}{}", symlink_prefix, workspace_base_name)
    }

    // Java: com/google/devtools/build/lib/buildtool/OutputDirectoryLinksUtils.java:355 OutputDirectoryLinksUtils.STANDARD_LINK_DEFINITIONS.getLinkPaths
    fn get_link_paths(
        &self,
        _build_request_options: &BuildRequestOptions,
        _target_configs: &[Arc<BuildConfigurationValue>],
        _repository_name: &RepositoryName,
        _output_path: &Path,
        exec_root: &Path,
    ) -> IndexSet<Path> {
        IndexSet::from([exec_root.clone()])
    }
}

/// Describes the outcome of symlink creation.
// Java: com/google/devtools/build/lib/buildtool/OutputDirectoryLinksUtils.java:369 OutputDirectoryLinksUtils.SymlinkCreationResult
#[derive(Clone, Debug, Default)]
pub struct SymlinkCreationResult {
    convenience_symlink_protos: Vec<ConvenienceSymlink>,
    created_symlinks: IndexMap<PathFragment, PathFragment>,
}

impl SymlinkCreationResult {
    /// `EMPTY_SYMLINK_CREATION_RESULT`.
    // Java: com/google/devtools/build/lib/buildtool/OutputDirectoryLinksUtils.java:365 OutputDirectoryLinksUtils.EMPTY_SYMLINK_CREATION_RESULT
    pub fn empty() -> SymlinkCreationResult {
        SymlinkCreationResult::default()
    }

    /// Returns descriptions of what symlinks were created and destroyed.
    // Java: com/google/devtools/build/lib/buildtool/OutputDirectoryLinksUtils.java:381 SymlinkCreationResult.getConvenienceSymlinkProtos
    pub fn get_convenience_symlink_protos(&self) -> &[ConvenienceSymlink] {
        &self.convenience_symlink_protos
    }

    /// Returns symlink name -> target mappings of symlinks that were actually created (or in the
    /// case of `LOG_ONLY`, would have been created).
    // Java: com/google/devtools/build/lib/buildtool/OutputDirectoryLinksUtils.java:389 SymlinkCreationResult.getCreatedSymlinks
    pub fn get_created_symlinks(&self) -> &IndexMap<PathFragment, PathFragment> {
        &self.created_symlinks
    }
}

/// Creates or updates an existing symbolic link from `link` to `target`. Missing ancestor
/// directories of `link` will also be created.
///
/// This operation is not atomic.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:329 FileSystemUtils.ensureSymbolicLink
fn ensure_symbolic_link(link: &Path, target: &PathFragment) -> std::io::Result<()> {
    // TODO(bazel-team): (2009) consider adding the logic for recovering from the case when
    // we have already created a parent directory symlink earlier.
    let mut parent_known_to_exist = false;
    // This will throw if the path already exists and is not a symbolic link.
    match link.read_symbolic_link() {
        Ok(existing) => {
            if &existing == target {
                // Nothing to do.
                return Ok(());
            }
            // The symlink exists, but points elsewhere.
            link.delete()?;
            parent_known_to_exist = true;
        }
        // Path does not exist; fall through.
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
        Err(e) => return Err(e),
    }
    if !parent_known_to_exist && let Some(parent) = link.get_parent_directory() {
        parent.create_directory_and_parents()?;
    }
    link.create_symbolic_link_to_fragment(target)
}

/// Attempts to remove a relative chain of directories under a given base. Returns true if the
/// removal was successful, and returns false if the removal fails because a directory was not
/// empty.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:641 FileSystemUtils.removeDirectoryAndParents
fn remove_directory_and_parents(base: &Path, mut to_remove: PathFragment) -> bool {
    if to_remove.is_absolute() {
        return false;
    }
    while !to_remove.is_empty() {
        let p = base.get_relative(&to_remove);
        if p.exists() && p.delete().is_err() {
            return false;
        }
        to_remove = match to_remove.get_parent_directory() {
            Some(parent) => parent,
            None => break,
        };
    }
    true
}

/// `IOException.getMessage()`: Blaze's file system errors already carry Java's message; plain OS
/// errors lose Rust's " (os error N)" suffix.
pub(crate) fn java_io_message(e: &std::io::Error) -> String {
    let text = e.to_string();
    match text.rfind(" (os error ") {
        Some(i) if e.raw_os_error().is_some() => text[..i].to_string(),
        _ => text,
    }
}
