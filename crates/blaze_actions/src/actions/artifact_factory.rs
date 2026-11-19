//! Port of `lib/actions/ArtifactFactory.java`.

use std::collections::BTreeMap;

use blaze_cmdline::label_constants::EXTERNAL_REPOSITORY_LOCATION;
use blaze_cmdline::{PackageIdentifier, RepositoryName};
use blaze_util::{check_argument, check_state};
use blaze_vfs::{Path, PathFragment, Root};
use parking_lot::Mutex;

use super::action_lookup::ArtifactOwner;
use super::artifact::{Artifact, SpecialArtifactType};
use super::artifact_root::ArtifactRoot;

/// `PackageRoots.PackageRootLookup`: the source root of a package, if known.
// Java: com/google/devtools/build/lib/actions/PackageRoots.java PackageRoots.PackageRootLookup
// PORT: implemented by the execution-phase package roots (blaze_sky_top/blaze_buildtool).
pub trait PackageRootLookup: Send + Sync {
    fn get_root_for_package(&self, package_identifier: &PackageIdentifier) -> Option<Root>;
}

/// `PackageRootResolver.findPackageRootsForFiles`.
// Java: com/google/devtools/build/lib/actions/PackageRootResolver.java PackageRootResolver
pub trait PackageRootResolver {
    /// Returns the source roots of the given exec paths, or `None` if Skyframe dependencies are
    /// missing (Java: null).
    fn find_package_roots_for_files(
        &self,
        exec_paths: &[PathFragment],
    ) -> Result<Option<BTreeMap<PathFragment, Option<Root>>>, String>;
}

/// A source artifact cache entry with the build it was validated in.
// Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:62 SourceArtifactCache.Entry
#[derive(Clone)]
struct Entry {
    artifact: Artifact,
    build_id: i32,
}

impl Entry {
    fn is_invalid(&self, current_build_id: i32) -> bool {
        self.build_id != current_build_id
    }
}

/// The source artifact cache, keyed case-insensitively on the exec path.
// Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:60 ArtifactFactory.SourceArtifactCache
// PORT: Java's ConcurrentSkipListMap with its case-insensitive comparator holds an Entry or a list
// of Entries per equivalence class of the comparator; a map from the class's key
// ([`case_insensitive_key`]) to a Vec of entries has the same lookups.
struct SourceArtifactCache {
    path_to_source_artifact: BTreeMap<String, Vec<Entry>>,
    build_id: i32,
}

/// The key of `exec_path`'s equivalence class under the map's comparator,
/// `comparing(p -> StringEncoding.internalToUnicode(p.getPathString()),
/// String.CASE_INSENSITIVE_ORDER)`: two paths compare equal exactly when their Unicode forms have
/// the same length in UTF-16 units and, unit by unit,
/// `Character.toLowerCase(Character.toUpperCase(c))` agrees. So `É` and `é` are one class, and so
/// are two different byte sequences that are not UTF-8 (both become U+FFFD).
// Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:81 SourceArtifactCache.pathToSourceArtifact
fn case_insensitive_key(exec_path: &PathFragment) -> String {
    blaze_util::string_encoding::internal_to_unicode(exec_path.get_path_string())
        .chars()
        .map(java_case_insensitive_fold)
        .collect()
}

/// `Character.toLowerCase(Character.toUpperCase(c))` for one UTF-16 unit of
/// `String.CASE_INSENSITIVE_ORDER` (JDK `String.CaseInsensitiveComparator`).
///
/// PORT: Java uses the simple case mappings (one char to one char); Rust's `char` methods give
/// the full ones. They agree where the full mapping is one char. Where it is several, the simple
/// upper-case mapping is the char itself for every char whose lower-case mapping then decides
/// (ß, ŉ, the Greek ypogegrammeni letters fold to the same char either way), and the one
/// multi-char lower-case mapping, U+0130, is written out. A supplementary char is two surrogates
/// for Java, which no case mapping changes.
fn java_case_insensitive_fold(c: char) -> char {
    fn single(mut it: impl Iterator<Item = char>) -> Option<char> {
        let first = it.next()?;
        it.next().is_none().then_some(first)
    }
    if c.is_ascii() {
        return c.to_ascii_lowercase();
    }
    if c as u32 > 0xFFFF {
        return c;
    }
    let upper = single(c.to_uppercase()).unwrap_or(c);
    if upper == '\u{130}' {
        return 'i';
    }
    single(upper.to_lowercase()).unwrap_or(upper)
}

impl SourceArtifactCache {
    fn new() -> Self {
        SourceArtifactCache {
            path_to_source_artifact: BTreeMap::new(),
            build_id: -1,
        }
    }

    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:111 SourceArtifactCache.getEntry
    fn get_entry(&self, exec_path: &PathFragment) -> Option<&Entry> {
        self.path_to_source_artifact
            .get(&case_insensitive_key(exec_path))?
            .iter()
            .find(|e| e.artifact.get_exec_path() == exec_path)
    }

    /// `computeEntry`: replaces or adds the entry for `exec_path` with `f(existing)`.
    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:115 SourceArtifactCache.computeEntry
    fn compute_entry(
        &mut self,
        exec_path: &PathFragment,
        f: impl FnOnce(Option<&Entry>, i32) -> Entry,
    ) -> Entry {
        let build_id = self.build_id;
        let entries = self
            .path_to_source_artifact
            .entry(case_insensitive_key(exec_path))
            .or_default();
        for e in entries.iter_mut() {
            if e.artifact.get_exec_path() == exec_path {
                let new_entry = f(Some(e), build_id);
                *e = new_entry.clone();
                return new_entry;
            }
        }
        let new_entry = f(None, build_id);
        entries.push(new_entry.clone());
        new_entry
    }

    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:162 SourceArtifactCache.getArtifact
    fn get_artifact(&self, exec_path: &PathFragment) -> Option<Artifact> {
        self.get_entry(exec_path).map(|e| e.artifact.clone())
    }

    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:176 SourceArtifactCache.getArtifactIfValid
    fn get_artifact_if_valid(&self, exec_path: &PathFragment) -> Option<Artifact> {
        match self.get_entry(exec_path) {
            Some(e) if !e.is_invalid(self.build_id) => Some(e.artifact.clone()),
            _ => None,
        }
    }

    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:187 SourceArtifactCache.getEntriesWithAsciiCaseInsensitivePath
    fn get_entries_with_ascii_case_insensitive_path(&self, exec_path: &PathFragment) -> Vec<Entry> {
        self.path_to_source_artifact
            .get(&case_insensitive_key(exec_path))
            .cloned()
            .unwrap_or_default()
    }

    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:210 SourceArtifactCache.getValidArtifactsWithAsciiCaseInsensitivePath
    fn get_valid_artifacts_with_ascii_case_insensitive_path(
        &self,
        exec_path: &PathFragment,
    ) -> Vec<Artifact> {
        self.get_entries_with_ascii_case_insensitive_path(exec_path)
            .into_iter()
            .filter(|e| !e.is_invalid(self.build_id))
            .map(|e| e.artifact)
            .collect()
    }

    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:225 SourceArtifactCache.getAllArtifactsWithAsciiCaseInsensitivePath
    fn get_all_artifacts_with_ascii_case_insensitive_path(
        &self,
        exec_path: &PathFragment,
    ) -> Vec<Artifact> {
        self.get_entries_with_ascii_case_insensitive_path(exec_path)
            .into_iter()
            .map(|e| e.artifact)
            .collect()
    }

    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:232 SourceArtifactCache.newBuild
    fn new_build(&mut self) {
        self.build_id += 1;
    }

    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:236 SourceArtifactCache.clear
    fn clear(&mut self) {
        self.path_to_source_artifact.clear();
        self.build_id = -1;
    }
}

/// A cache of Artifacts, keyed by Path.
// Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:44 ArtifactFactory
pub struct ArtifactFactory {
    exec_root_parent: Path,
    external_source_base: Path,
    derived_path_prefix: PathFragment,
    sibling_repository_layout: Mutex<bool>,
    source_artifact_cache: Mutex<SourceArtifactCache>,
    package_roots: Mutex<Option<std::sync::Arc<dyn PackageRootLookup>>>,
}

impl ArtifactFactory {
    /// Constructs a new artifact factory that will use a given execution root when creating
    /// artifacts.
    ///
    /// `exec_root_parent`: the execution root's parent path. This will be
    /// `[output_base]/execroot`. `derived_path_prefix`: e.g. `bazel-out`.
    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:247 ArtifactFactory.<init>
    pub fn new(exec_root_parent: Path, derived_path_prefix: &str) -> ArtifactFactory {
        let external_source_base = exec_root_parent
            .get_parent_directory()
            .expect("execroot has a parent")
            .get_relative(&EXTERNAL_REPOSITORY_LOCATION);
        ArtifactFactory {
            exec_root_parent,
            external_source_base,
            derived_path_prefix: PathFragment::create(derived_path_prefix),
            sibling_repository_layout: Mutex::new(false),
            source_artifact_cache: Mutex::new(SourceArtifactCache::new()),
            package_roots: Mutex::new(None),
        }
    }

    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:236 ArtifactFactory.clear
    pub fn clear(&self) {
        *self.package_roots.lock() = None;
        self.source_artifact_cache.lock().clear();
    }

    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:262 ArtifactFactory.setSiblingRepositoryLayout
    pub fn set_sibling_repository_layout(&self, sibling_repository_layout: bool) {
        *self.sibling_repository_layout.lock() = sibling_repository_layout;
    }

    /// Set the set of known packages and their corresponding source artifact roots.
    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:272 ArtifactFactory.setPackageRoots
    pub fn set_package_roots(&self, package_roots: std::sync::Arc<dyn PackageRootLookup>) {
        *self.package_roots.lock() = Some(package_roots);
    }

    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:276 ArtifactFactory.noteAnalysisStarting
    pub fn note_analysis_starting(&self) {
        self.source_artifact_cache.lock().new_build();
    }

    /// `getSourceArtifact(execPath, root)` with `ArtifactOwner.NULL_OWNER`.
    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:281 ArtifactFactory.getSourceArtifact
    pub fn get_source_artifact(&self, exec_path: &PathFragment, root: &Root) -> Artifact {
        self.get_source_artifact_with_owner(exec_path, root, ArtifactOwner::NullOwner)
    }

    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:281 ArtifactFactory.getSourceArtifact
    pub fn get_source_artifact_with_owner(
        &self,
        exec_path: &PathFragment,
        root: &Root,
        owner: ArtifactOwner,
    ) -> Artifact {
        let artifact_root = if root
            .as_path()
            .is_some_and(|p| p.starts_with(&self.external_source_base))
        {
            ArtifactRoot::as_external_source_root(root.clone())
        } else {
            ArtifactRoot::as_source_root(root.clone())
        };
        self.get_source_artifact_in_root(exec_path, artifact_root, owner)
    }

    /// Returns an artifact for a tool at the given root-relative path under the given root,
    /// creating it if not found. This method only works for normalized, relative paths.
    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:281 ArtifactFactory.getSourceArtifact
    pub fn get_source_artifact_in_root(
        &self,
        exec_path: &PathFragment,
        root: ArtifactRoot,
        owner: ArtifactOwner,
    ) -> Artifact {
        check_argument!(
            exec_path.is_absolute() == root.get_root().is_absolute(),
            "{} {} {}",
            exec_path,
            root,
            owner
        );
        self.get_artifact(root, exec_path.clone(), owner, None)
    }

    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:303 ArtifactFactory.validatePath
    fn validate_path(&self, root_relative_path: &PathFragment, root: &ArtifactRoot) {
        check_argument!(!root.is_source_root());
        check_argument!(
            root_relative_path.is_absolute() == root.get_root().is_absolute(),
            "{}",
            root_relative_path
        );
        check_argument!(
            !root_relative_path.contains_uplevel_references(),
            "{}",
            root_relative_path
        );
        let root_path = root.get_root().as_path().expect("derived root has a path");
        check_argument!(
            root_path.starts_with(&self.exec_root_parent),
            "{} must start with {}, root = {}, root fs = {:p}, execRootParent fs = {:p}",
            root.get_root(),
            self.exec_root_parent,
            root,
            std::sync::Arc::as_ptr(root_path.get_file_system()),
            std::sync::Arc::as_ptr(self.exec_root_parent.get_file_system())
        );
        check_argument!(
            *root_path != self.exec_root_parent,
            "{} {} {}",
            root.get_root(),
            self.exec_root_parent,
            root
        );
    }

    /// Returns an artifact for a tool at the given root-relative path under the given root,
    /// creating it if not found. This method only works for normalized, relative paths.
    ///
    /// The root must be below the execRootParent, and the execPath of the resulting Artifact is
    /// computed as `root.getRelative(rootRelativePath).relativeTo(root.execRoot)`.
    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:334 ArtifactFactory.getDerivedArtifact
    pub fn get_derived_artifact(
        &self,
        root_relative_path: &PathFragment,
        root: &ArtifactRoot,
        owner: ArtifactOwner,
    ) -> Artifact {
        self.validate_path(root_relative_path, root);
        self.get_artifact(
            root.clone(),
            root.get_exec_path().get_relative(root_relative_path),
            owner,
            None,
        )
    }

    /// Returns an artifact that represents the output directory of a Fileset at the given
    /// root-relative path under the given root, creating it if not found.
    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:349 ArtifactFactory.getFilesetArtifact
    pub fn get_fileset_artifact(
        &self,
        root_relative_path: &PathFragment,
        root: &ArtifactRoot,
        owner: ArtifactOwner,
    ) -> Artifact {
        self.get_special(
            root_relative_path,
            root,
            owner,
            SpecialArtifactType::FILESET,
        )
    }

    /// Returns an artifact that represents a runfiles tree at the given root-relative path.
    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:360 ArtifactFactory.getRunfilesArtifact
    pub fn get_runfiles_artifact(
        &self,
        root_relative_path: &PathFragment,
        root: &ArtifactRoot,
        owner: ArtifactOwner,
    ) -> Artifact {
        self.get_special(
            root_relative_path,
            root,
            owner,
            SpecialArtifactType::RUNFILES,
        )
    }

    /// Returns an artifact that represents a TreeArtifact; that is, a directory containing some
    /// tree of ArtifactFiles unknown at analysis time.
    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:378 ArtifactFactory.getTreeArtifact
    pub fn get_tree_artifact(
        &self,
        root_relative_path: &PathFragment,
        root: &ArtifactRoot,
        owner: ArtifactOwner,
    ) -> Artifact {
        self.get_special(root_relative_path, root, owner, SpecialArtifactType::TREE)
    }

    /// Returns an artifact that represents an unresolved symlink; that is, an artifact whose
    /// value is a symlink and is never dereferenced.
    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:396 ArtifactFactory.getSymlinkArtifact
    pub fn get_symlink_artifact(
        &self,
        root_relative_path: &PathFragment,
        root: &ArtifactRoot,
        owner: ArtifactOwner,
    ) -> Artifact {
        self.get_special(
            root_relative_path,
            root,
            owner,
            SpecialArtifactType::UNRESOLVED_SYMLINK,
        )
    }

    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:407 ArtifactFactory.getConstantMetadataArtifact
    pub fn get_constant_metadata_artifact(
        &self,
        root_relative_path: &PathFragment,
        root: &ArtifactRoot,
        owner: ArtifactOwner,
    ) -> Artifact {
        self.get_special(
            root_relative_path,
            root,
            owner,
            SpecialArtifactType::CONSTANT_METADATA,
        )
    }

    fn get_special(
        &self,
        root_relative_path: &PathFragment,
        root: &ArtifactRoot,
        owner: ArtifactOwner,
        type_: SpecialArtifactType,
    ) -> Artifact {
        self.validate_path(root_relative_path, root);
        self.get_artifact(
            root.clone(),
            root.get_exec_path().get_relative(root_relative_path),
            owner,
            Some(type_),
        )
    }

    /// Returns the Artifact for the specified path, creating one if not found and setting the
    /// `root` and `owner` attributes.
    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:162 ArtifactFactory.getArtifact
    fn get_artifact(
        &self,
        root: ArtifactRoot,
        exec_path: PathFragment,
        owner: ArtifactOwner,
        type_: Option<SpecialArtifactType>,
    ) -> Artifact {
        if !root.is_source_root() {
            return create_artifact(root, exec_path, owner, type_);
        }

        let mut cache = self.source_artifact_cache.lock();
        if let Some(first_artifact) = cache.get_artifact(&exec_path) {
            if !first_artifact.different_owner_or_root(&owner, &root) {
                return first_artifact;
            }
        }

        // Double-checked locking to avoid locking cost when possible.
        let new_entry = cache.compute_entry(&exec_path, |entry, build_id| match entry {
            Some(e) if !e.artifact.different_owner_or_root(&owner, &root) => e.clone(),
            _ => Entry {
                artifact: create_artifact(root.clone(), exec_path.clone(), owner.clone(), type_),
                build_id,
            },
        });
        new_entry.artifact
    }

    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:473 ArtifactFactory.isDefinitelyNotSourceExecPath
    fn is_definitely_not_source_exec_path(&self, exec_path: &PathFragment) -> bool {
        // Source exec paths cannot escape the source root.
        if *self.sibling_repository_layout.lock() {
            // In the sibling repository layout, the exec path is relative to the execroot, which
            // is one level below the sources of external repositories.
            if exec_path.sub_fragment_from(1).contains_uplevel_references() {
                return true;
            }
        } else if exec_path.contains_uplevel_references() {
            return true;
        }
        false
    }

    /// Returns an `Artifact` with exec path formed by composing `base_exec_path` and
    /// `relative_path` (via `base_exec_path.get_relative(relative_path)` if `base_exec_path` is
    /// not `None`).
    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:501 ArtifactFactory.resolveSourceArtifactWithAncestor
    pub fn resolve_source_artifact_with_ancestor(
        &self,
        relative_path: &PathFragment,
        base_exec_path: Option<&PathFragment>,
        base_root: Option<&ArtifactRoot>,
        repository_name: &RepositoryName,
    ) -> Option<Artifact> {
        check_state!(
            base_exec_path.is_none() == base_root.is_none(),
            "{} {:?} {:?}",
            relative_path,
            base_exec_path,
            base_root
        );
        check_state!(
            !relative_path.is_empty(),
            "{} {:?} {:?}",
            relative_path,
            base_exec_path,
            base_root
        );
        let exec_path = match base_exec_path {
            Some(b) => b.get_relative(relative_path),
            None => relative_path.clone(),
        };

        if self.is_definitely_not_source_exec_path(&exec_path) {
            return None;
        }

        // Don't create an artifact if it's derived.
        if self.is_derived_artifact(&exec_path) {
            return None;
        }
        if let Some(artifact) = self
            .source_artifact_cache
            .lock()
            .get_artifact_if_valid(&exec_path)
        {
            return Some(artifact);
        }
        let source_root = self.find_source_root(
            &exec_path,
            base_exec_path,
            base_root.map(|r| r.get_root()),
            repository_name,
        );
        self.create_artifact_if_not_valid(source_root, &exec_path)
    }

    /// Probe the known packages to find the longest package prefix up until the base, or until
    /// the root directory if our execPath doesn't start with baseExecPath due to uplevel
    /// references.
    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:530 ArtifactFactory.findSourceRoot
    fn find_source_root(
        &self,
        exec_path: &PathFragment,
        base_exec_path: Option<&PathFragment>,
        base_root: Option<&Root>,
        repository_name: &RepositoryName,
    ) -> Option<Root> {
        let mut dir = exec_path.get_parent_directory()?;
        let mut repository_name = repository_name.clone();
        if let Some((repo, rel)) =
            RepositoryName::from_path_fragment(&dir, *self.sibling_repository_layout.lock())
        {
            repository_name = repo;
            dir = rel;
        }

        let package_roots = self
            .package_roots
            .lock()
            .clone()
            .expect("NullPointerException: packageRoots");
        let mut dir = Some(dir);
        while let Some(d) = &dir {
            if Some(d) == base_exec_path {
                break;
            }
            if let Some(source_root) = package_roots.get_root_for_package(
                &PackageIdentifier::create(repository_name.clone(), d.clone()),
            ) {
                return Some(source_root);
            }
            dir = d.get_parent_directory();
        }

        match (&dir, base_exec_path) {
            (Some(d), Some(b)) if d == b => base_root.cloned(),
            _ => None,
        }
    }

    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:570 ArtifactFactory.resolveSourceArtifact
    pub fn resolve_source_artifact(
        &self,
        exec_path: &PathFragment,
        repository_name: &RepositoryName,
    ) -> Option<Artifact> {
        self.resolve_source_artifact_with_ancestor(exec_path, None, None, repository_name)
    }

    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:576 ArtifactFactory.resolveSourceArtifactsAsciiCaseInsensitively
    pub fn resolve_source_artifacts_ascii_case_insensitively(
        &self,
        exec_path: &PathFragment,
        repository_name: &RepositoryName,
    ) -> Vec<Artifact> {
        if self.is_definitely_not_source_exec_path(exec_path) {
            return Vec::new();
        }
        if self.is_derived_artifact(exec_path) {
            return Vec::new();
        }
        let artifacts = self
            .source_artifact_cache
            .lock()
            .get_valid_artifacts_with_ascii_case_insensitive_path(exec_path);
        if !artifacts.is_empty() {
            return artifacts;
        }
        let stale_artifacts = self
            .source_artifact_cache
            .lock()
            .get_all_artifacts_with_ascii_case_insensitive_path(exec_path);
        if !stale_artifacts.is_empty() {
            let mut revalidated = Vec::new();
            for stale in stale_artifacts {
                let source_root =
                    self.find_source_root(stale.get_exec_path(), None, None, repository_name);
                if let Some(valid) =
                    self.create_artifact_if_not_valid(source_root, stale.get_exec_path())
                {
                    revalidated.push(valid);
                }
            }
            if !revalidated.is_empty() {
                return revalidated;
            }
        }
        let source_root = self.find_source_root(exec_path, None, None, repository_name);
        match self.create_artifact_if_not_valid(source_root, exec_path) {
            None => Vec::new(),
            Some(a) => vec![a],
        }
    }

    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:624 ArtifactFactory.resolveSourceArtifacts
    pub fn resolve_source_artifacts(
        &self,
        exec_paths: &[PathFragment],
        resolver: &dyn PackageRootResolver,
    ) -> Result<Option<BTreeMap<PathFragment, Option<Artifact>>>, String> {
        let mut result: BTreeMap<PathFragment, Option<Artifact>> = BTreeMap::new();
        let mut unresolved_paths: Vec<PathFragment> = Vec::new();
        for exec_path in exec_paths {
            if self.is_definitely_not_source_exec_path(exec_path) {
                result.insert(exec_path.clone(), None);
                continue;
            }
            if self.is_derived_artifact(exec_path) {
                result.insert(exec_path.clone(), None);
            } else {
                let a = self
                    .source_artifact_cache
                    .lock()
                    .get_artifact_if_valid(exec_path);
                match a {
                    Some(a) => {
                        result.insert(exec_path.clone(), Some(a));
                    }
                    None => unresolved_paths.push(exec_path.clone()),
                }
            }
        }
        let Some(source_roots) = resolver.find_package_roots_for_files(&unresolved_paths)? else {
            return Ok(None);
        };
        for path in unresolved_paths {
            let root = source_roots.get(&path).cloned().flatten();
            let a = self.create_artifact_if_not_valid(root, &path);
            result.insert(path, a);
        }
        Ok(Some(result))
    }

    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:660 ArtifactFactory.getPathFromSourceExecPath
    pub fn get_path_from_source_exec_path(
        &self,
        exec_root: &Path,
        exec_path: &PathFragment,
    ) -> Path {
        check_state!(
            !exec_path.starts_with(&self.derived_path_prefix),
            "{} is derived: {}",
            exec_path,
            self.derived_path_prefix
        );
        let mut repository_name = blaze_cmdline::repository_name::MAIN.clone();
        let mut repository_relative_path = exec_path.clone();
        if let Some((repo, rel)) =
            RepositoryName::from_path_fragment(exec_path, *self.sibling_repository_layout.lock())
        {
            repository_name = repo;
            repository_relative_path = rel;
        }
        let package_roots = self
            .package_roots
            .lock()
            .clone()
            .expect("NullPointerException: packageRoots");
        let mut source_root = package_roots.get_root_for_package(&PackageIdentifier::create(
            repository_name,
            repository_relative_path.clone(),
        ));
        if source_root.is_none() {
            source_root = self.find_source_root(
                exec_path,
                None,
                None,
                &blaze_cmdline::repository_name::MAIN.clone(),
            );
        }
        match source_root {
            Some(r) => r.get_relative(&repository_relative_path),
            None => exec_root.get_relative(exec_path),
        }
    }

    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:603 ArtifactFactory.createArtifactIfNotValid
    fn create_artifact_if_not_valid(
        &self,
        source_root: Option<Root>,
        exec_path: &PathFragment,
    ) -> Option<Artifact> {
        // not a path that we can find...
        let source_root = source_root?;
        let artifact = self.source_artifact_cache.lock().get_artifact(exec_path);
        match artifact {
            Some(artifact) if source_root == *artifact.get_root().get_root() => {
                // We have to make sure that the cache contains the artifact in case of an
                // incremental build (the entry is stale).
                let mut cache = self.source_artifact_cache.lock();
                cache.compute_entry(exec_path, |cache_entry, build_id| {
                    let cache_entry = cache_entry.expect("entry present").clone();
                    let valid_artifact = cache_entry.artifact.clone();
                    if cache_entry.is_invalid(build_id) {
                        return Entry {
                            artifact: valid_artifact,
                            build_id,
                        };
                    }
                    check_state!(
                        artifact == valid_artifact,
                        "Mismatched artifacts: {} {}",
                        artifact,
                        valid_artifact
                    );
                    cache_entry
                });
                Some(artifact)
            }
            _ => Some(self.get_source_artifact_with_owner(
                exec_path,
                &source_root,
                ArtifactOwner::NullOwner,
            )),
        }
    }

    /// Determines whether the `exec_path` lies within the derived artifacts tree.
    // Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:722 ArtifactFactory.isDerivedArtifact
    pub fn is_derived_artifact(&self, exec_path: &PathFragment) -> bool {
        exec_path.starts_with(&self.derived_path_prefix)
    }
}

// Java: com/google/devtools/build/lib/actions/ArtifactFactory.java:450 ArtifactFactory.createArtifact
fn create_artifact(
    root: ArtifactRoot,
    exec_path: PathFragment,
    owner: ArtifactOwner,
    type_: Option<SpecialArtifactType>,
) -> Artifact {
    let action_lookup_key = |owner: ArtifactOwner| match owner {
        ArtifactOwner::ActionLookupKey(k) => k,
        other => panic!("ClassCastException: {other} is not an ActionLookupKey"),
    };
    match type_ {
        None => {
            if root.is_source_root() {
                Artifact::new_source_artifact(root, exec_path, owner)
            } else {
                Artifact::create_derived_artifact(root, exec_path, action_lookup_key(owner))
            }
        }
        Some(t) => Artifact::create_special_artifact(root, exec_path, action_lookup_key(owner), t),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(s: &str) -> String {
        case_insensitive_key(&PathFragment::create(s))
    }

    #[test]
    fn case_insensitive_key_follows_java_comparator() {
        use blaze_util::string_encoding::unicode_to_internal;
        assert_eq!(key("Foo/BAR.h"), key("foo/bar.h"));
        // `É` (c3 89) and `é` (c3 a9): one class for Java, which compares the Unicode forms.
        assert_eq!(
            key(&unicode_to_internal("dir/\u{c9}.h")),
            key(&unicode_to_internal("dir/\u{e9}.h"))
        );
        assert_ne!(
            key(&unicode_to_internal("dir/\u{e9}.h")),
            key(&unicode_to_internal("dir/e.h"))
        );
        // `🐷` (f0 9f 90 b7): no case.
        assert_eq!(
            key(&unicode_to_internal("\u{1f437}")),
            "\u{1f437}".to_string()
        );
        // Bytes that are not UTF-8 all become U+FFFD.
        assert_eq!(key("a\u{ff}"), key("A\u{fe}"));
        // `ß` has no one-char upper case; `SS` is longer.
        assert_ne!(
            key(&unicode_to_internal("\u{df}")),
            key(&unicode_to_internal("SS"))
        );
        assert_eq!(java_case_insensitive_fold('\u{130}'), 'i');
        assert_eq!(java_case_insensitive_fold('\u{1f88}'), '\u{1f80}');
        assert_eq!(java_case_insensitive_fold('\u{1f80}'), '\u{1f80}');
    }
}
