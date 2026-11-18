//! Port of `Label`.
//!
//! Starlark integration (the `@StarlarkBuiltin` value, `Label.relative`, `debugPrint`'s thread
//! lookup) belongs to a later crate that wraps `Label`; this module ports the data type and the
//! pure methods behind it.

use std::cmp::Ordering;
use std::collections::HashMap;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, LazyLock};

use blaze_vfs::PathFragment;

use crate::interner::{WeakInterner, stable_hash};
use crate::label_parser::{LabelParserParts, validate_and_process_target_name};
use crate::label_syntax_exception::{EvalException, LabelSyntaxException};
use crate::package_identifier::PackageIdentifier;
use crate::repository_mapping::RepositoryMapping;
use crate::repository_name::{MAIN, RepositoryName};

/// Package names that aren't made relative to the current repository because they mean special
/// things to Bazel.
// Java: com/google/devtools/build/lib/cmdline/Label.java:94 Label.ABSOLUTE_PACKAGE_NAMES
const ABSOLUTE_PACKAGE_NAMES: [&str; 2] = [
    // Used for select's `//conditions:default` label (not a target)
    "conditions",
    // Used for the public and private visibility labels (not targets)
    "visibility",
];

/// Identifies a BUILD target: a package and a target name, e.g. `//dir1/dir2:target_name`
/// (Java `Label`).
///
/// Interned (weakly, like Java's `LabelInterner`), immutable, cheap to clone, `Send + Sync`.
/// `Display` is Java's `toString()`, the canonical form.
// Java: com/google/devtools/build/lib/cmdline/Label.java:89 Label
#[derive(Clone)]
pub struct Label(Arc<LabelInner>);

struct LabelInner {
    /// The name and repository of the package.
    package_identifier: PackageIdentifier,
    /// The name of the target within the package. Canonical.
    name: Arc<str>,
    hash: u64,
}

// Java: com/google/devtools/build/lib/cmdline/Label.java:657 Label.equals
impl PartialEq for LabelInner {
    fn eq(&self, other: &Self) -> bool {
        // Package identifiers are (weakly) interned so we compare them first.
        self.hash == other.hash
            && self.package_identifier == other.package_identifier
            && self.name == other.name
    }
}

impl Eq for LabelInner {}

// PORT: Java's LabelInterner (pooled interner with Skyframe package pools) is replaced by a weak
// interner; the pool transfer is incremental-state plumbing.
// Java: com/google/devtools/build/lib/cmdline/Label.java:111 Label.interner
static INTERNER: LazyLock<WeakInterner<LabelInner>> = LazyLock::new(WeakInterner::new);

/// The context of a current repo, necessary to parse a repo-relative label ("//foo:bar")
/// (Java `Label.RepoContext`).
// Java: com/google/devtools/build/lib/cmdline/Label.java:118 Label.RepoContext
pub trait LabelRepoContext {
    fn current_repo(&self) -> &RepositoryName;

    fn repo_mapping(&self) -> &RepositoryMapping;

    // Java: com/google/devtools/build/lib/cmdline/Label.java:127 Label.RepoContext.rootPackage
    fn root_package(&self) -> LabelPackageContext {
        LabelPackageContext::of(
            &PackageIdentifier::create_root_package(self.current_repo().clone()),
            self.repo_mapping().clone(),
        )
    }
}

/// The plain implementation of [`LabelRepoContext`] (Java `Label.RepoContextImpl`).
// Java: com/google/devtools/build/lib/cmdline/Label.java:133 Label.RepoContextImpl
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LabelRepoContextImpl {
    current_repo: RepositoryName,
    repo_mapping: RepositoryMapping,
}

impl LabelRepoContextImpl {
    // Java: com/google/devtools/build/lib/cmdline/Label.java:119 Label.RepoContext.of
    pub fn of(current_repo: RepositoryName, repo_mapping: RepositoryMapping) -> Self {
        LabelRepoContextImpl {
            current_repo,
            repo_mapping,
        }
    }
}

impl LabelRepoContext for LabelRepoContextImpl {
    fn current_repo(&self) -> &RepositoryName {
        &self.current_repo
    }

    fn repo_mapping(&self) -> &RepositoryMapping {
        &self.repo_mapping
    }
}

/// The context of a current package, necessary to parse a package-relative label (":foo")
/// (Java `Label.PackageContext` / `Label.PackageContextImpl`).
// Java: com/google/devtools/build/lib/cmdline/Label.java:136 Label.PackageContext
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct LabelPackageContext {
    current_repo: RepositoryName,
    repo_mapping: RepositoryMapping,
    package_fragment: PathFragment,
}

impl LabelPackageContext {
    // Java: com/google/devtools/build/lib/cmdline/Label.java:137 Label.PackageContext.of
    pub fn of(current_package: &PackageIdentifier, repo_mapping: RepositoryMapping) -> Self {
        LabelPackageContext {
            current_repo: current_package.get_repository().clone(),
            repo_mapping,
            package_fragment: current_package.get_package_fragment().clone(),
        }
    }

    // Java: com/google/devtools/build/lib/cmdline/Label.java:142 Label.PackageContext.packageFragment
    pub fn package_fragment(&self) -> &PathFragment {
        &self.package_fragment
    }

    // Java: com/google/devtools/build/lib/cmdline/Label.java:144 Label.PackageContext.packageIdentifier
    pub fn package_identifier(&self) -> PackageIdentifier {
        PackageIdentifier::create(self.current_repo.clone(), self.package_fragment.clone())
    }
}

impl LabelRepoContext for LabelPackageContext {
    fn current_repo(&self) -> &RepositoryName {
        &self.current_repo
    }

    fn repo_mapping(&self) -> &RepositoryMapping {
        &self.repo_mapping
    }
}

/// Records repo mapping entries used by [`Label::parse_with_package_context_recording`] (Java
/// `Label.RepoMappingRecorder`).
// Java: com/google/devtools/build/lib/cmdline/Label.java:248 Label.RepoMappingRecorder
pub trait LabelRepoMappingRecorder {
    fn record(
        &mut self,
        from_repo: &RepositoryName,
        apparent_repo_name: &str,
        canonical_repo_name: &RepositoryName,
    );

    // Java: com/google/devtools/build/lib/cmdline/Label.java:251 Label.RepoMappingRecorder.record
    fn record_all(&mut self, entries: &[(RepositoryName, String, RepositoryName)]) {
        for (row, column, value) in entries {
            self.record(row, column, value);
        }
    }
}

/// A [`LabelRepoMappingRecorder`] backed by a table (Java `Label.SimpleRepoMappingRecorder`).
// Java: com/google/devtools/build/lib/cmdline/Label.java:266 Label.SimpleRepoMappingRecorder
#[derive(Debug, Clone, Default)]
pub struct LabelSimpleRepoMappingRecorder {
    entries: HashMap<(RepositoryName, String), RepositoryName>,
}

impl LabelSimpleRepoMappingRecorder {
    pub fn new() -> Self {
        Self::default()
    }

    /// The recorded `(fromRepo, apparentName, canonicalName)` cells, rows ordered by repository
    /// name and columns in natural order (Java `ImmutableTable.cellSet()` order).
    // Java: com/google/devtools/build/lib/cmdline/Label.java:275 Label.SimpleRepoMappingRecorder.recordedEntries
    pub fn recorded_entries(&self) -> Vec<(RepositoryName, String, RepositoryName)> {
        let mut cells: Vec<(RepositoryName, String, RepositoryName)> = self
            .entries
            .iter()
            .map(|((row, column), value)| (row.clone(), column.clone(), value.clone()))
            .collect();
        cells.sort_by(|a, b| {
            a.0.get_name()
                .as_bytes()
                .cmp(b.0.get_name().as_bytes())
                .then_with(|| a.1.as_bytes().cmp(b.1.as_bytes()))
        });
        cells
    }
}

impl LabelRepoMappingRecorder for LabelSimpleRepoMappingRecorder {
    // Java: com/google/devtools/build/lib/cmdline/Label.java:270 Label.SimpleRepoMappingRecorder.record
    fn record(
        &mut self,
        from_repo: &RepositoryName,
        apparent_repo_name: &str,
        canonical_repo_name: &RepositoryName,
    ) {
        self.entries.insert(
            (from_repo.clone(), apparent_repo_name.to_owned()),
            canonical_repo_name.clone(),
        );
    }
}

/// Computes the repo name for the label, within the context of a current repo.
// Java: com/google/devtools/build/lib/cmdline/Label.java:177 Label.computeRepoNameWithRepoContext
fn compute_repo_name_with_repo_context(
    parts: &LabelParserParts,
    repo_context: &dyn LabelRepoContext,
) -> RepositoryName {
    let Some(repo) = parts.repo() else {
        // Certain package names when used without a "@" part are always absolutely in the main
        // repo, disregarding the current repo and repo mappings.
        if ABSOLUTE_PACKAGE_NAMES.contains(&parts.pkg()) {
            return MAIN.clone();
        }
        return repo_context.current_repo().clone();
    };
    if parts.repo_is_canonical() {
        // This label uses the canonical label literal syntax starting with two @'s ("@@foo//bar").
        return RepositoryName::create_unvalidated(repo);
    }
    repo_context.repo_mapping().get(repo)
}

impl Label {
    /// Parses a canonical label string `[@repo]//foo/bar[:quux]`. A `@repo` part must be a
    /// canonical repo name; without it the label is in the main repo.
    // Java: com/google/devtools/build/lib/cmdline/Label.java:157 Label.parseCanonical
    pub fn parse_canonical(raw: &str) -> Result<Label, LabelSyntaxException> {
        let parts = LabelParserParts::parse(raw)?;
        parts.check_pkg_does_not_end_with_triple_dots()?;
        parts.check_pkg_is_absolute()?;
        let repo_name = match parts.repo() {
            None => MAIN.clone(),
            Some(repo) => RepositoryName::create_unvalidated(repo),
        };
        Ok(Self::create_unvalidated(
            PackageIdentifier::create(repo_name, PathFragment::create(parts.pkg())),
            parts.target(),
        ))
    }

    /// Like [`Label::parse_canonical`], but panics (Java `IllegalArgumentException`) on error.
    // Java: com/google/devtools/build/lib/cmdline/Label.java:168 Label.parseCanonicalUnchecked
    pub fn parse_canonical_unchecked(raw: &str) -> Label {
        match Self::parse_canonical(raw) {
            Ok(label) => label,
            Err(e) => panic!("java.lang.IllegalArgumentException: {}", e),
        }
    }

    /// Parses `[@repo]//foo/bar[:quux]` within a repo context: `@repo` undergoes the repo
    /// mapping; without it the label is in the current repo.
    // Java: com/google/devtools/build/lib/cmdline/Label.java:200 Label.parseWithRepoContext
    pub fn parse_with_repo_context(
        raw: &str,
        repo_context: &dyn LabelRepoContext,
    ) -> Result<Label, LabelSyntaxException> {
        let parts = LabelParserParts::parse(raw)?;
        parts.check_pkg_does_not_end_with_triple_dots()?;
        parts.check_pkg_is_absolute()?;
        let repo_name = compute_repo_name_with_repo_context(&parts, repo_context);
        Ok(Self::create_unvalidated(
            PackageIdentifier::create(repo_name, PathFragment::create(parts.pkg())),
            parts.target(),
        ))
    }

    /// Parses a label within a package context; package-relative forms (`:quux`) are allowed.
    // Java: com/google/devtools/build/lib/cmdline/Label.java:217 Label.parseWithPackageContext
    pub fn parse_with_package_context(
        raw: &str,
        package_context: &LabelPackageContext,
    ) -> Result<Label, LabelSyntaxException> {
        Self::parse_with_package_context_internal(&LabelParserParts::parse(raw)?, package_context)
    }

    /// [`Label::parse_with_package_context`], recording any apparent-repo mapping used.
    // PORT: Java overload parseWithPackageContext(String, PackageContext, RepoMappingRecorder).
    // Java: com/google/devtools/build/lib/cmdline/Label.java:222 Label.parseWithPackageContext
    pub fn parse_with_package_context_recording(
        raw: &str,
        package_context: &LabelPackageContext,
        repo_mapping_recorder: Option<&mut dyn LabelRepoMappingRecorder>,
    ) -> Result<Label, LabelSyntaxException> {
        let parts = LabelParserParts::parse(raw)?;
        let parsed = Self::parse_with_package_context_internal(&parts, package_context)?;
        if let Some(recorder) = repo_mapping_recorder
            && let Some(repo) = parts.repo()
            && !parts.repo_is_canonical()
        {
            recorder.record(
                package_context.current_repo(),
                repo,
                parsed.get_repository(),
            );
        }
        Ok(parsed)
    }

    // Java: com/google/devtools/build/lib/cmdline/Label.java:234 Label.parseWithPackageContextInternal
    fn parse_with_package_context_internal(
        parts: &LabelParserParts,
        package_context: &LabelPackageContext,
    ) -> Result<Label, LabelSyntaxException> {
        parts.check_pkg_does_not_end_with_triple_dots()?;
        // pkg is either absolute or empty
        if !parts.pkg().is_empty() {
            parts.check_pkg_is_absolute()?;
        }
        let repo_name = compute_repo_name_with_repo_context(parts, package_context);
        let pkg_fragment = if parts.pkg_is_absolute() {
            PathFragment::create(parts.pkg())
        } else {
            package_context.package_fragment().clone()
        };
        Ok(Self::create_unvalidated(
            PackageIdentifier::create(repo_name, pkg_fragment),
            parts.target(),
        ))
    }

    /// Creates a label from a package name (without `//`) and a target name, validating both.
    // Java: com/google/devtools/build/lib/cmdline/Label.java:293 Label.create
    pub fn create(package_name: &str, target_name: &str) -> Result<Label, LabelSyntaxException> {
        Ok(Self::create_unvalidated(
            PackageIdentifier::parse(package_name)?,
            &validate_and_process_target_name(
                package_name,
                target_name,
                /* pkg_ends_with_triple_dots= */ false,
            )?,
        ))
    }

    /// Creates a label in the given package, validating the target name.
    // PORT: Java overload create(PackageIdentifier, String).
    // Java: com/google/devtools/build/lib/cmdline/Label.java:303 Label.create
    pub fn create_in_package(
        package_id: &PackageIdentifier,
        target_name: &str,
    ) -> Result<Label, LabelSyntaxException> {
        Ok(Self::create_unvalidated(
            package_id.clone(),
            &validate_and_process_target_name(
                package_id.get_package_fragment().get_path_string(),
                target_name,
                /* pkg_ends_with_triple_dots= */ false,
            )?,
        ))
    }

    /// Creates a label without validating the target name. Only for known-valid names.
    // Java: com/google/devtools/build/lib/cmdline/Label.java:319 Label.createUnvalidated
    pub fn create_unvalidated(package_identifier: PackageIdentifier, name: &str) -> Label {
        // Java: com/google/devtools/build/lib/cmdline/Label.java:339 Label.<init>
        // Java: com/google/devtools/build/lib/cmdline/Label.java:642 Label.hashCode
        let hash = stable_hash(&(name, package_identifier.cached_hash()));
        let inner = LabelInner {
            package_identifier,
            name: Arc::from(name),
            hash,
        };
        Label(INTERNER.intern(hash, inner))
    }

    // Java: com/google/devtools/build/lib/cmdline/Label.java:347 Label.getPackageIdentifier
    pub fn get_package_identifier(&self) -> &PackageIdentifier {
        &self.0.package_identifier
    }

    // Java: com/google/devtools/build/lib/cmdline/Label.java:351 Label.getRepository
    pub fn get_repository(&self) -> &RepositoryName {
        self.0.package_identifier.get_repository()
    }

    /// The package name without repository (`//file/base:fileutils_test` gives `file/base`).
    /// Starlark: `Label.package`.
    // Java: com/google/devtools/build/lib/cmdline/Label.java:367 Label.getPackageName
    pub fn get_package_name(&self) -> &str {
        self.0
            .package_identifier
            .get_package_fragment()
            .get_path_string()
    }

    /// Starlark `Label.workspace_root` (deprecated). The `StarlarkSemantics` argument is reduced
    /// to the `--experimental_sibling_repository_layout` bit it reads.
    // Java: com/google/devtools/build/lib/cmdline/Label.java:392 Label.getWorkspaceRootForStarlarkOnly
    pub fn get_workspace_root_for_starlark_only(
        &self,
        experimental_sibling_repository_layout: bool,
    ) -> Result<String, EvalException> {
        self.check_repo_visibility_for_starlark("workspace_root")?;
        Ok(self
            .get_repository()
            .get_exec_path(experimental_sibling_repository_layout)
            .to_string())
    }

    /// Starlark `Label.repo_root`. See [`Label::get_workspace_root_for_starlark_only`].
    // Java: com/google/devtools/build/lib/cmdline/Label.java:420 Label.getRepoRootForStarlarkOnly
    pub fn get_repo_root_for_starlark_only(
        &self,
        experimental_sibling_repository_layout: bool,
    ) -> Result<String, EvalException> {
        self.check_repo_visibility_for_starlark("repo_root")?;
        Ok(self
            .get_repository()
            .get_exec_path(experimental_sibling_repository_layout)
            .to_string())
    }

    // Java: com/google/devtools/build/lib/cmdline/Label.java:436 Label.getPackageFragment
    pub fn get_package_fragment(&self) -> &PathFragment {
        self.0.package_identifier.get_package_fragment()
    }

    /// The label as a path fragment (package + name), without the repository. Panics if the name
    /// is ".".
    // Java: com/google/devtools/build/lib/cmdline/Label.java:449 Label.toPathFragment
    pub fn to_path_fragment(&self) -> PathFragment {
        // PathFragments are normalized, so if we do this on a non-file target named '.'
        // then the package would be returned. Detect this and throw.
        // A target named '.' can never refer to a file.
        assert!(&*self.0.name != ".", "IllegalArgumentException");
        self.get_package_fragment().get_relative_str(&self.0.name)
    }

    /// The target name (`//foo/bar:baz` gives `baz`). Starlark: `Label.name`.
    // Java: com/google/devtools/build/lib/cmdline/Label.java:467 Label.getName
    pub fn get_name(&self) -> &str {
        &self.0.name
    }

    /// The canonical form; `parse_canonical(x.get_canonical_form()) == x`.
    // Java: com/google/devtools/build/lib/cmdline/Label.java:490 Label.getCanonicalForm
    pub fn get_canonical_form(&self) -> String {
        format!(
            "{}:{}",
            self.0.package_identifier.get_canonical_form(),
            self.0.name
        )
    }

    /// An absolutely unambiguous canonical form (`@@repo//pkg:name`, `@@//pkg:name`).
    // Java: com/google/devtools/build/lib/cmdline/Label.java:499 Label.getUnambiguousCanonicalForm
    pub fn get_unambiguous_canonical_form(&self) -> String {
        format!(
            "{}:{}",
            self.0.package_identifier.get_unambiguous_canonical_form(),
            self.0.name
        )
    }

    /// A full label string for display in the context of the main repository mapping.
    // Java: com/google/devtools/build/lib/cmdline/Label.java:511 Label.getDisplayForm
    pub fn get_display_form(&self, main_repository_mapping: Option<&RepositoryMapping>) -> String {
        format!(
            "{}:{}",
            self.0
                .package_identifier
                .get_display_form(main_repository_mapping),
            self.0.name
        )
    }

    /// Like [`Label::get_display_form`] but elides the name where possible (`//foo/bar` for
    /// `//foo/bar:bar`, `@repo` for `@repo//:repo`).
    // Java: com/google/devtools/build/lib/cmdline/Label.java:527 Label.getShorthandDisplayForm
    pub fn get_shorthand_display_form(
        &self,
        main_repository_mapping: Option<&RepositoryMapping>,
    ) -> String {
        let base_name = self.get_package_fragment().get_base_name();
        if base_name == &*self.0.name {
            return self
                .0
                .package_identifier
                .get_display_form(main_repository_mapping);
        } else if base_name.is_empty() {
            let repository_display_form = self
                .get_package_identifier()
                .get_repository()
                .get_display_form(main_repository_mapping);
            // Simplify @foo//:foo or @@foo//:foo to @foo; note that `name` cannot start with '@'
            if repository_display_form == format!("@{}", self.0.name)
                || repository_display_form == format!("@@{}", self.0.name)
            {
                return repository_display_form;
            }
        }
        self.get_display_form(main_repository_mapping)
    }

    /// Starlark `Label.workspace_name` (deprecated): the canonical repo name without "@".
    // Java: com/google/devtools/build/lib/cmdline/Label.java:553 Label.getWorkspaceName
    pub fn get_workspace_name(&self) -> Result<String, EvalException> {
        self.check_repo_visibility_for_starlark("workspace_name")?;
        Ok(self.get_repository().get_name().to_owned())
    }

    /// Starlark `Label.repo_name`: the canonical repo name without "@".
    // Java: com/google/devtools/build/lib/cmdline/Label.java:566 Label.getRepoName
    pub fn get_repo_name(&self) -> Result<String, EvalException> {
        self.check_repo_visibility_for_starlark("repo_name")?;
        Ok(self.get_repository().get_name().to_owned())
    }

    /// A label in the same package with the given (validated) target name. Starlark:
    /// `Label.same_package_label`.
    // Java: com/google/devtools/build/lib/cmdline/Label.java:580 Label.getSamePackageLabel
    pub fn get_same_package_label(&self, target_name: &str) -> Result<Label, LabelSyntaxException> {
        Self::create_in_package(&self.0.package_identifier, target_name)
    }

    /// A user-friendly representation that works for `None` ("(unknown)").
    // Java: com/google/devtools/build/lib/cmdline/Label.java:691 Label.print
    pub fn print(label: Option<&Label>) -> String {
        match label {
            None => "(unknown)".to_owned(),
            Some(label) => label.to_string(),
        }
    }

    /// The directory in which `label` would reside if it were interpreted as a path.
    // Java: com/google/devtools/build/lib/cmdline/Label.java:699 Label.getContainingDirectory
    pub fn get_containing_directory(label: &Label) -> PathFragment {
        let pkg = label.get_package_fragment();
        let name = label.get_name();
        if name == "." {
            return pkg.clone();
        }
        if PathFragment::is_normalized_relative_path(name)
            && !PathFragment::contains_separator(name)
        {
            // Optimize for the common case of a label like '//pkg:target'.
            return pkg.clone();
        }
        pkg.get_relative_str(name)
            .get_parent_directory()
            .expect("non-empty relative path has a parent")
    }

    /// Starlark `str(label)`. The `StarlarkSemantics` argument is reduced to the
    /// `--incompatible_unambiguous_label_stringification` bit it reads (default true).
    // Java: com/google/devtools/build/lib/cmdline/Label.java:740 Label.str
    pub fn str(&self, incompatible_unambiguous_label_stringification: bool) -> String {
        if self.get_repository().is_main() && !incompatible_unambiguous_label_stringification {
            // If this label is in the main repo and we're not using unambiguous label
            // stringification, the result should always be "//foo:bar".
            return self.get_canonical_form();
        }

        // Otherwise, we use canonical label literal syntax here and prepend an extra '@'.
        // So the result looks like "@@//foo:bar" for the main repo and "@@foo+//bar:quux" for
        // other repos.
        self.get_unambiguous_canonical_form()
    }

    /// Starlark `print()` form: the shorthand display form under the main repo mapping (Java
    /// reads the mapping from the thread's `StarlarkThreadContext`; here it is passed in).
    // Java: com/google/devtools/build/lib/cmdline/Label.java:726 Label.debugPrint
    pub fn debug_print(&self, main_repo_mapping: Option<&RepositoryMapping>) -> String {
        self.get_shorthand_display_form(main_repo_mapping)
    }

    // Java: com/google/devtools/build/lib/cmdline/Label.java:757 Label.expandToCommandLine
    pub fn expand_to_command_line(&self) -> String {
        // TODO(wyv): Consider using StarlarkSemantics here too for optional unambiguity.
        self.get_canonical_form()
    }

    // Java: com/google/devtools/build/lib/cmdline/Label.java:762 Label.checkRepoVisibilityForStarlark
    fn check_repo_visibility_for_starlark(&self, method: &str) -> Result<(), EvalException> {
        if !self.get_repository().is_visible() {
            return Err(EvalException::new(format!(
                "'{}' is not allowed on invalid Label {}",
                method, self
            )));
        }
        Ok(())
    }

    /// Java reference equality.
    pub fn ptr_eq(a: &Label, b: &Label) -> bool {
        Arc::ptr_eq(&a.0, &b.0)
    }
}

/// The canonical form (Java `toString()`).
// Java: com/google/devtools/build/lib/cmdline/Label.java:479 Label.toString
impl fmt::Display for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.get_canonical_form())
    }
}

impl fmt::Debug for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "Label({})", self.get_unambiguous_canonical_form())
    }
}

/// Two labels are equal iff both their name and their package name are equal.
// Java: com/google/devtools/build/lib/cmdline/Label.java:657 Label.equals
impl PartialEq for Label {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || *self.0 == *other.0
    }
}

impl Eq for Label {}

// Java: com/google/devtools/build/lib/cmdline/Label.java:642 Label.hashCode
impl Hash for Label {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.0.hash);
    }
}

/// Ordered by package, then target name; `//a:b/c` comes before `//a/b:a`.
// Java: com/google/devtools/build/lib/cmdline/Label.java:677 Label.compareTo
impl Ord for Label {
    fn cmp(&self, other: &Self) -> Ordering {
        if Arc::ptr_eq(&self.0, &other.0) {
            return Ordering::Equal;
        }
        self.0
            .package_identifier
            .cmp(&other.0.package_identifier)
            .then_with(|| self.0.name.as_bytes().cmp(other.0.name.as_bytes()))
    }
}

impl PartialOrd for Label {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
