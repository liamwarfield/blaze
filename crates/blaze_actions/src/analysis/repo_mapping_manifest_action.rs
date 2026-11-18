//! Port of `lib/analysis/RepoMappingManifestAction.java`.

use std::any::Any;
use std::collections::BTreeMap;
use std::hash::{Hash, Hasher};
use std::io::{self, Write};

use blaze_cmdline::repository_name::MAIN;
use blaze_cmdline::{PackageIdentifier, RepositoryMapping, RepositoryName};
use blaze_collect::command_line_item::{ExceptionlessMapFn, MapFn, MapFnError};
use blaze_collect::{NestedSet, NestedSetBuilder, Order};
use blaze_util::Fingerprint;
use indexmap::{IndexMap, IndexSet};

use crate::actions::action::{
    AbstractAction, Action, ComputeKeyError, FileContentsProvider, InputMetadataProvider,
};
use crate::actions::action_key_context::ActionKeyContext;
use crate::actions::action_owner::ActionOwner;
use crate::actions::artifact::Artifact;
use crate::analysis::actions::abstract_file_write_action::{
    AbstractFileWriteAction, DeterministicWriter, FnDeterministicWriter,
    NewDeterministicWriterError, exec_properties, new_abstract_file_write_action,
};
use crate::analysis::symlink_entry::SymlinkEntry;

// Java: com/google/devtools/build/lib/analysis/RepoMappingManifestAction.java:61 RepoMappingManifestAction.MY_UUID
// UUID.fromString("458e351c-4d30-433d-b927-da6cddd4737f")
const MY_UUID_STRING: &str = "458e351c-4d30-433d-b927-da6cddd4737f";
const MY_UUID_MOST_SIGNIFICANT: i64 = 0x458e351c4d30433d_u64 as i64;
const MY_UUID_LEAST_SIGNIFICANT: i64 = 0xb927da6cddd4737f_u64 as i64;

/// The parts of a `Package.Metadata` that `RepoMappingManifestAction` reads: the package
/// identifier (whose repository is the source repo of the mapping) and the package's
/// repository mapping.
///
/// PORT: Java's `transitivePackages` is a `NestedSet<Package.Metadata>`. `Package.Metadata`
/// lives in blaze_packages, which blaze_actions cannot depend on; the analysis lead converts
/// each `blaze_packages::PackageMetadata` of
/// `RuleContext.getTransitivePackagesForRunfileRepoMappingManifest()` into this struct
/// (`package_identifier`, `repository_mapping`). Java's record `equals` compares all fields and
/// `hashCode` uses only the package identifier ("within a single build a package is uniquely
/// identified by its PackageIdentifier"); equality here compares the two fields kept.
// Java: com/google/devtools/build/lib/packages/Package.java:1389 Package.Metadata
#[derive(Clone, Debug)]
pub struct RepoMappingPackageMetadata {
    package_identifier: PackageIdentifier,
    repository_mapping: RepositoryMapping,
}

impl RepoMappingPackageMetadata {
    pub fn new(
        package_identifier: PackageIdentifier,
        repository_mapping: RepositoryMapping,
    ) -> RepoMappingPackageMetadata {
        RepoMappingPackageMetadata {
            package_identifier,
            repository_mapping,
        }
    }

    /// `Package.Metadata.packageIdentifier()`.
    pub fn package_identifier(&self) -> &PackageIdentifier {
        &self.package_identifier
    }

    /// `Package.Metadata.repositoryMapping()`.
    pub fn repository_mapping(&self) -> &RepositoryMapping {
        &self.repository_mapping
    }
}

impl PartialEq for RepoMappingPackageMetadata {
    fn eq(&self, other: &Self) -> bool {
        self.package_identifier == other.package_identifier
            && self.repository_mapping == other.repository_mapping
    }
}

impl Eq for RepoMappingPackageMetadata {}

// Java: com/google/devtools/build/lib/packages/Package.java:1406 Package.Metadata.hashCode
impl Hash for RepoMappingPackageMetadata {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.package_identifier.hash(state);
    }
}

/// The fingerprint of a repository mapping's entries.
// Java: com/google/devtools/build/lib/analysis/RepoMappingManifestAction.java:64 RepoMappingManifestAction.repoMappingFingerprintCache
// PORT: the Caffeine weak-key cache is a memory/CPU optimization; the digest is recomputed.
fn repo_mapping_fingerprint(repo_mapping: &IndexMap<String, RepositoryName>) -> String {
    let mut fp = Fingerprint::new();
    fp.add_int(repo_mapping.len() as i32);
    for (apparent_name, canonical_name) in repo_mapping {
        fp.add_string(apparent_name);
        fp.add_string(canonical_name.get_name());
    }
    fp.hex_digest_and_reset()
}

/// Uses MapFn's args parameter just like Fingerprint#addString to compute a cacheable
/// fingerprint of just the repo name and mapping of a given Package.
// Java: com/google/devtools/build/lib/analysis/RepoMappingManifestAction.java:82 RepoMappingManifestAction.REPO_AND_MAPPING_DIGEST_FN
pub struct RepoAndMappingDigestFn;
pub static REPO_AND_MAPPING_DIGEST_FN: RepoAndMappingDigestFn = RepoAndMappingDigestFn;

impl MapFn<RepoMappingPackageMetadata> for RepoAndMappingDigestFn {
    fn expand_to_command_line(
        &self,
        pkg_metadata: &RepoMappingPackageMetadata,
        args: &mut dyn FnMut(&str),
    ) -> Result<(), MapFnError> {
        args(
            pkg_metadata
                .package_identifier()
                .get_repository()
                .get_name(),
        );
        args(&repo_mapping_fingerprint(
            pkg_metadata.repository_mapping().entries(),
        ));
        Ok(())
    }
}

impl ExceptionlessMapFn<RepoMappingPackageMetadata> for RepoAndMappingDigestFn {}

// Java: com/google/devtools/build/lib/analysis/RepoMappingManifestAction.java:88 RepoMappingManifestAction.OWNER_REPO_FN
pub struct OwnerRepoFn;
pub static OWNER_REPO_FN: OwnerRepoFn = OwnerRepoFn;

impl MapFn<Artifact> for OwnerRepoFn {
    fn expand_to_command_line(
        &self,
        artifact: &Artifact,
        args: &mut dyn FnMut(&str),
    ) -> Result<(), MapFnError> {
        match artifact.get_owner() {
            Some(owner) => args(owner.get_repository().get_name()),
            None => args(""),
        }
        Ok(())
    }
}

impl ExceptionlessMapFn<Artifact> for OwnerRepoFn {}

// Java: com/google/devtools/build/lib/analysis/RepoMappingManifestAction.java:94 RepoMappingManifestAction.FIRST_SEGMENT_FN
pub struct FirstSegmentFn;
pub static FIRST_SEGMENT_FN: FirstSegmentFn = FirstSegmentFn;

impl MapFn<SymlinkEntry> for FirstSegmentFn {
    fn expand_to_command_line(
        &self,
        symlink: &SymlinkEntry,
        args: &mut dyn FnMut(&str),
    ) -> Result<(), MapFnError> {
        args(symlink.get_path().get_segment(0));
        Ok(())
    }
}

impl ExceptionlessMapFn<SymlinkEntry> for FirstSegmentFn {}

/// The separator character used to combine the segments of a canonical repository name.
// Java: com/google/devtools/build/lib/analysis/RepoMappingManifestAction.java:182 RepoMappingManifestAction.REPO_NAME_SEPARATOR
const REPO_NAME_SEPARATOR: char = '+';

/// Creates a manifest file describing the repos and mappings relevant for a runfile tree.
// Java: com/google/devtools/build/lib/analysis/RepoMappingManifestAction.java:58 RepoMappingManifestAction
#[derive(Debug)]
pub struct RepoMappingManifestAction {
    base: AbstractAction,
    transitive_packages: NestedSet<RepoMappingPackageMetadata>,
    runfiles_artifacts: NestedSet<Artifact>,
    has_runfiles_symlinks: bool,
    runfiles_root_symlinks: NestedSet<SymlinkEntry>,
    workspace_name: String,
    emit_compact_repo_mapping: bool,
}

impl RepoMappingManifestAction {
    // Java: com/google/devtools/build/lib/analysis/RepoMappingManifestAction.java:104 RepoMappingManifestAction.<init>
    pub fn new(
        owner: ActionOwner,
        output: Artifact,
        transitive_packages: NestedSet<RepoMappingPackageMetadata>,
        runfiles_artifacts: NestedSet<Artifact>,
        runfiles_symlinks: &NestedSet<SymlinkEntry>,
        runfiles_root_symlinks: NestedSet<SymlinkEntry>,
        workspace_name: String,
        emit_compact_repo_mapping: bool,
    ) -> RepoMappingManifestAction {
        let base = new_abstract_file_write_action(
            owner,
            NestedSetBuilder::empty_set(Order::STABLE_ORDER),
            output,
        );
        RepoMappingManifestAction {
            base,
            transitive_packages,
            runfiles_artifacts,
            has_runfiles_symlinks: !runfiles_symlinks.is_empty(),
            runfiles_root_symlinks,
            workspace_name,
            emit_compact_repo_mapping,
        }
    }

    /// `newDeterministicWriter()`: writes the manifest.
    // Java: com/google/devtools/build/lib/analysis/RepoMappingManifestAction.java:176 RepoMappingManifestAction.newDeterministicWriter
    // PORT: the writer body is this method; Java's PrintWriter over ISO-8859-1 writes the
    // internal strings' bytes (StringEncoding identity).
    pub fn write_contents(&self, out: &mut dyn Write) -> io::Result<()> {
        let mut writer = io::BufWriter::new(out);

        let mut repos_in_runfiles_paths: IndexSet<String> = IndexSet::new();
        // The runfiles paths of symlinks are always prefixed with the main workspace name, *not*
        // the name of the repository adding the symlink.
        if self.has_runfiles_symlinks {
            repos_in_runfiles_paths.insert(MAIN.get_name().to_string());
        }

        // Since root symlinks are the only way to stage a runfile at a specific path under the
        // current repository's runfiles directory, recognize canonical repository names that
        // appear as the first segment of their runfiles paths.
        for symlink in self.runfiles_root_symlinks.to_list().iter() {
            repos_in_runfiles_paths.insert(symlink.get_path().get_segment(0).to_string());
        }

        for artifact in self.runfiles_artifacts.to_list().iter() {
            if let Some(owner) = artifact.get_owner() {
                repos_in_runfiles_paths.insert(owner.get_repository().get_name().to_string());
            }
        }

        // ImmutableSortedMap by RepositoryName::getName; all packages in a given repository have
        // the same repository mapping, so the particular way of resolving duplicates does not
        // matter (Java keeps the first).
        let mut sorted_repo_mappings: BTreeMap<String, (RepositoryName, RepositoryMapping)> =
            BTreeMap::new();
        for pkg_metadata in self.transitive_packages.to_list().iter() {
            let repository = pkg_metadata.package_identifier().get_repository();
            sorted_repo_mappings
                .entry(repository.get_name().to_string())
                .or_insert_with(|| {
                    (
                        repository.clone(),
                        pkg_metadata.repository_mapping().clone(),
                    )
                });
        }
        let sorted: Vec<&(RepositoryName, RepositoryMapping)> =
            sorted_repo_mappings.values().collect();

        if self.emit_compact_repo_mapping {
            let mut i = 0;
            while i < sorted.len() {
                // If multiple (consecutive in sort order) repositories have identical repo
                // mappings, we merge them into a single entry in the manifest, with the source
                // repo name being the common prefix of the individual names, followed by a
                // wildcard '*'. This is meant to reduce the size of the manifest entries for
                // module extension repos from quadratic to linear in the number of repos, so we
                // limit ourselves to those repositories.
                let first_repo_and_mapping = sorted[i];
                i += 1;
                let mut group_size = 1;
                while i < sorted.len() && should_merge(first_repo_and_mapping, sorted[i]) {
                    group_size += 1;
                    i += 1;
                }
                let first_repo_name = first_repo_and_mapping.0.get_name();
                let source = if group_size == 1 {
                    first_repo_name.to_string()
                } else {
                    replace_last_segment_with_asterisk(first_repo_name)
                };
                for (apparent_name, canonical_name) in compute_relevant_entries(
                    &repos_in_runfiles_paths,
                    first_repo_and_mapping.1.entries(),
                ) {
                    self.write_entry(&mut writer, &source, &apparent_name, &canonical_name)?;
                }
            }
        } else {
            // All repositories generated by a module extension have the same Map instance as
            // the entries of their RepositoryMapping, with every repo appearing as an entry.
            // PORT: Java caches the relevant entries per entries-map instance
            // (IdentityHashMap) to avoid quadratic time; the output is the same without it.
            for repo_and_mapping in &sorted {
                for (apparent_name, canonical_name) in
                    compute_relevant_entries(&repos_in_runfiles_paths, repo_and_mapping.1.entries())
                {
                    self.write_entry(
                        &mut writer,
                        repo_and_mapping.0.get_name(),
                        &apparent_name,
                        &canonical_name,
                    )?;
                }
            }
        }
        writer.flush()?;
        Ok(())
    }

    // Java: com/google/devtools/build/lib/analysis/RepoMappingManifestAction.java:316 RepoMappingManifestAction.writeEntry
    fn write_entry(
        &self,
        writer: &mut dyn Write,
        source: &str,
        target_apparent_name: &str,
        target_canonical_name: &RepositoryName,
    ) -> io::Result<()> {
        // The canonical name of the main repo is the empty string, which is not a valid name
        // for a directory, so the "workspace name" is used the name of the directory under the
        // runfiles tree for it.
        let target_repo_directory_name = if target_canonical_name.is_main() {
            self.workspace_name.as_str()
        } else {
            target_canonical_name.get_name()
        };
        writer.write_all(
            format!(
                "{},{},{}\n",
                source, target_apparent_name, target_repo_directory_name
            )
            .as_bytes(),
        )
    }
}

/// Mappings of repos generated by the same extension are reference equal in Java due to
/// `ModuleExtensionRepoMappingEntriesFunction`, so Java compares the entries maps with `==`.
// Java: com/google/devtools/build/lib/analysis/RepoMappingManifestAction.java:276 RepoMappingManifestAction.shouldMerge
// PORT: Blaze's RepositoryMapping keeps its entries inline (no shared entries map across repos
// with different context repos), so identity cannot be observed; this compares the entries by
// value. Repos with the same name prefix up to the last '+' are repos of one extension, whose
// Java entries maps are the same instance, so the result is the same.
fn should_merge(
    first: &(RepositoryName, RepositoryMapping),
    second: &(RepositoryName, RepositoryMapping),
) -> bool {
    first.1.entries() == second.1.entries()
        && have_same_prefix(first.0.get_name(), second.0.get_name())
}

/// Returns whether the two repository names agree up to the last segment of their names.
// Java: com/google/devtools/build/lib/analysis/RepoMappingManifestAction.java:287 RepoMappingManifestAction.haveSamePrefix
fn have_same_prefix(first: &str, second: &str) -> bool {
    let first_separator_pos = first.rfind(REPO_NAME_SEPARATOR);
    let second_separator_pos = second.rfind(REPO_NAME_SEPARATOR);
    let Some(first_separator_pos) = first_separator_pos else {
        return false;
    };
    if Some(first_separator_pos) != second_separator_pos {
        return false;
    }
    first.as_bytes()[..first_separator_pos] == second.as_bytes()[..first_separator_pos]
}

// Java: com/google/devtools/build/lib/analysis/RepoMappingManifestAction.java:296 RepoMappingManifestAction.replaceLastSegmentWithAsterisk
fn replace_last_segment_with_asterisk(repo_name: &str) -> String {
    // lastIndexOf returns -1 when absent, so substring(0, 0).
    let end = repo_name
        .rfind(REPO_NAME_SEPARATOR)
        .map(|p| p + 1)
        .unwrap_or(0);
    format!("{}*", &repo_name[..end])
}

// Java: com/google/devtools/build/lib/analysis/RepoMappingManifestAction.java:300 RepoMappingManifestAction.computeRelevantEntries
fn compute_relevant_entries(
    repos_in_runfiles_paths: &IndexSet<String>,
    mapping_entries: &IndexMap<String, RepositoryName>,
) -> Vec<(String, RepositoryName)> {
    let mut entries: Vec<(String, RepositoryName)> = mapping_entries
        .iter()
        // The apparent repo name can only be empty for the main repo. We skip this line as
        // Rlocation paths can't reference an empty apparent name anyway.
        .filter(|(apparent, _)| !apparent.is_empty())
        // We only write entries for repos whose canonical names appear in runfiles paths.
        .filter(|(_, canonical)| repos_in_runfiles_paths.contains(canonical.get_name()))
        .map(|(apparent, canonical)| (apparent.clone(), canonical.clone()))
        .collect();
    // Entry.comparingByKey (stable sort).
    entries.sort_by(|a, b| a.0.cmp(&b.0));
    entries
}

impl Action for RepoMappingManifestAction {
    fn abstract_action(&self) -> &AbstractAction {
        &self.base
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    // Java: com/google/devtools/build/lib/analysis/RepoMappingManifestAction.java:123 RepoMappingManifestAction.getMnemonic
    fn get_mnemonic(&self) -> String {
        "RepoMappingManifest".to_string()
    }

    // Java: com/google/devtools/build/lib/analysis/RepoMappingManifestAction.java:128 RepoMappingManifestAction.getRawProgressMessage
    fn get_raw_progress_message(&self) -> Option<String> {
        Some(format!(
            "Writing repo mapping manifest for {}",
            match self.get_owner().get_label() {
                Some(label) => label.to_string(),
                None => "null".to_string(),
            }
        ))
    }

    // Java: com/google/devtools/build/lib/analysis/RepoMappingManifestAction.java:133 RepoMappingManifestAction.computeKey
    fn compute_key(
        &self,
        action_key_context: &ActionKeyContext,
        _input_metadata_provider: Option<&dyn InputMetadataProvider>,
        fp: &mut Fingerprint,
    ) -> Result<(), ComputeKeyError> {
        fp.add_uuid(MY_UUID_LEAST_SIGNIFICANT, MY_UUID_MOST_SIGNIFICANT);
        action_key_context.add_nested_set_to_fingerprint_exceptionless(
            &REPO_AND_MAPPING_DIGEST_FN
                as &'static dyn ExceptionlessMapFn<RepoMappingPackageMetadata>,
            fp,
            &self.transitive_packages,
        );
        action_key_context.add_nested_set_to_fingerprint_exceptionless(
            &OWNER_REPO_FN as &'static dyn ExceptionlessMapFn<Artifact>,
            fp,
            &self.runfiles_artifacts,
        );
        fp.add_boolean(self.has_runfiles_symlinks);
        action_key_context.add_nested_set_to_fingerprint_exceptionless(
            &FIRST_SEGMENT_FN as &'static dyn ExceptionlessMapFn<SymlinkEntry>,
            fp,
            &self.runfiles_root_symlinks,
        );
        fp.add_string(&self.workspace_name);
        fp.add_boolean(self.emit_compact_repo_mapping);
        Ok(())
    }

    // Java: com/google/devtools/build/lib/analysis/RepoMappingManifestAction.java:148 RepoMappingManifestAction.describeKey
    fn describe_key(&self) -> Option<String> {
        Some(format!(
            "GUID: {}\ntransitivePackages: {}\nrunfilesArtifacts: {}\nhasRunfilesSymlinks: {}\n\
             runfilesRootSymlinks: {}\nworkspaceName: {}\nemitCompactRepoMapping: {}",
            MY_UUID_STRING,
            ActionKeyContext::describe_nested_set_fingerprint(
                &REPO_AND_MAPPING_DIGEST_FN as &dyn ExceptionlessMapFn<RepoMappingPackageMetadata>,
                &self.transitive_packages
            ),
            ActionKeyContext::describe_nested_set_fingerprint(
                &OWNER_REPO_FN as &dyn ExceptionlessMapFn<Artifact>,
                &self.runfiles_artifacts
            ),
            self.has_runfiles_symlinks,
            ActionKeyContext::describe_nested_set_fingerprint(
                &FIRST_SEGMENT_FN as &dyn ExceptionlessMapFn<SymlinkEntry>,
                &self.runfiles_root_symlinks
            ),
            self.workspace_name,
            self.emit_compact_repo_mapping
        ))
    }

    // Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:125 AbstractFileWriteAction.getExecProperties
    fn get_exec_properties(&self) -> IndexMap<String, String> {
        exec_properties()
    }

    fn as_file_contents_provider(&self) -> Option<&dyn FileContentsProvider> {
        Some(self)
    }
}

impl AbstractFileWriteAction for RepoMappingManifestAction {
    // Java: com/google/devtools/build/lib/analysis/RepoMappingManifestAction.java:176 RepoMappingManifestAction.newDeterministicWriter
    fn new_deterministic_writer(
        &self,
    ) -> Result<Box<dyn DeterministicWriter + '_>, NewDeterministicWriterError> {
        Ok(Box::new(FnDeterministicWriter(
            move |out: &mut dyn Write| self.write_contents(out),
        )))
    }
}

impl FileContentsProvider for RepoMappingManifestAction {
    /// Get the contents of a file internally using an in memory output stream.
    // Java: com/google/devtools/build/lib/analysis/RepoMappingManifestAction.java:174 RepoMappingManifestAction.getFileContents
    fn get_file_contents(&self) -> Result<String, io::Error> {
        let mut stream: Vec<u8> = Vec::new();
        self.write_contents(&mut stream)?;
        // PORT: StringEncoding identity; ISO-8859-1 decoding of the internal bytes.
        String::from_utf8(stream).map_err(io::Error::other)
    }

    // Java: com/google/devtools/build/lib/analysis/actions/AbstractFileWriteAction.java:55 AbstractFileWriteAction.makeExecutable
    fn make_executable(&self) -> bool {
        AbstractFileWriteAction::make_executable(self)
    }
}
