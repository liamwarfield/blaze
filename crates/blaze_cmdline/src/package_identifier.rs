//! Port of `PackageIdentifier`.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, LazyLock};

use blaze_vfs::PathFragment;

use crate::interner::{WeakInterner, stable_hash};
use crate::label_constants::{EXPERIMENTAL_EXTERNAL_PATH_PREFIX, EXTERNAL_PATH_PREFIX};
use crate::label_parser::{LabelParserParts, syntax_error};
use crate::label_syntax_exception::LabelSyntaxException;
use crate::repository_mapping::RepositoryMapping;
use crate::repository_name::{MAIN, RepositoryName};

/// Uniquely identifies a package: the (canonical) name of its repository and its path fragment
/// (Java `PackageIdentifier`).
///
/// Interned (weakly, like Java's `SkyKeyInterner`), immutable, cheap to clone, `Send + Sync`.
// Java: com/google/devtools/build/lib/cmdline/PackageIdentifier.java:49 PackageIdentifier
#[derive(Clone)]
pub struct PackageIdentifier(Arc<PackageIdentifierInner>);

struct PackageIdentifierInner {
    repository: RepositoryName,
    pkg_name: PathFragment,
    /// Precomputed hash code over repository and pkgName.
    hash: u64,
}

// Java: com/google/devtools/build/lib/cmdline/PackageIdentifier.java:275 PackageIdentifier.equals
impl PartialEq for PackageIdentifierInner {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash
            && self.pkg_name == other.pkg_name
            && self.repository == other.repository
    }
}

impl Eq for PackageIdentifierInner {}

// Java: com/google/devtools/build/lib/cmdline/PackageIdentifier.java:50 PackageIdentifier.interner
static INTERNER: LazyLock<WeakInterner<PackageIdentifierInner>> = LazyLock::new(WeakInterner::new);

/// The root package of the main repository.
// Java: com/google/devtools/build/lib/cmdline/PackageIdentifier.java:66 PackageIdentifier.EMPTY_PACKAGE_ID
pub static EMPTY_PACKAGE_ID: LazyLock<PackageIdentifier> =
    LazyLock::new(|| PackageIdentifier::create_in_main_repo(PathFragment::empty_fragment()));

impl PackageIdentifier {
    /// Creates a package identifier from a repository name string (validated) and a package path.
    // PORT: Java overload create(String, PathFragment).
    // Java: com/google/devtools/build/lib/cmdline/PackageIdentifier.java:52 PackageIdentifier.create
    pub fn create_from_repo_string(
        repository: &str,
        pkg_name: PathFragment,
    ) -> Result<PackageIdentifier, LabelSyntaxException> {
        Ok(Self::create(RepositoryName::create(repository)?, pkg_name))
    }

    /// Creates (interns) a package identifier. The package name is not validated.
    // Java: com/google/devtools/build/lib/cmdline/PackageIdentifier.java:57 PackageIdentifier.create
    pub fn create(repository: RepositoryName, pkg_name: PathFragment) -> PackageIdentifier {
        // Java: com/google/devtools/build/lib/cmdline/PackageIdentifier.java:139 PackageIdentifier.<init>
        let hash = stable_hash(&(repository.cached_hash(), pkg_name.get_path_string()));
        let inner = PackageIdentifierInner {
            repository,
            pkg_name,
            hash,
        };
        PackageIdentifier(INTERNER.intern(hash, inner))
    }

    /// Creates a package identifier from known-valid strings.
    // Java: com/google/devtools/build/lib/cmdline/PackageIdentifier.java:62 PackageIdentifier.createUnchecked
    pub fn create_unchecked(repository: &str, pkg_name: &str) -> PackageIdentifier {
        Self::create(
            RepositoryName::create_unvalidated(repository),
            PathFragment::create(pkg_name),
        )
    }

    // PORT: Java overload createInMainRepo(String).
    // Java: com/google/devtools/build/lib/cmdline/PackageIdentifier.java:69 PackageIdentifier.createInMainRepo
    pub fn create_in_main_repo_str(name: &str) -> PackageIdentifier {
        Self::create_in_main_repo(PathFragment::create(name))
    }

    // Java: com/google/devtools/build/lib/cmdline/PackageIdentifier.java:73 PackageIdentifier.createInMainRepo
    pub fn create_in_main_repo(name: PathFragment) -> PackageIdentifier {
        Self::create(MAIN.clone(), name)
    }

    // Java: com/google/devtools/build/lib/cmdline/PackageIdentifier.java:77 PackageIdentifier.createRootPackage
    pub fn create_root_package(repository: RepositoryName) -> PackageIdentifier {
        Self::create(repository, PathFragment::empty_fragment())
    }

    /// Infers the package identifier from an exec path by structure alone (no I/O). The result may
    /// be a subdirectory of a package. Returns `None` for invalid labels (e.g. bad repo names).
    ///
    /// Panics if `exec_path` is absolute, or if `for_files` and it has no parent.
    // Java: com/google/devtools/build/lib/cmdline/PackageIdentifier.java:98 PackageIdentifier.discoverFromExecPath
    pub fn discover_from_exec_path(
        exec_path: &PathFragment,
        for_files: bool,
        sibling_repository_layout: bool,
    ) -> Option<PackageIdentifier> {
        assert!(!exec_path.is_absolute(), "{}", exec_path);
        let tofind = if for_files {
            exec_path
                .get_parent_directory()
                .expect("Must pass in files, not root directory")
        } else {
            exec_path.clone()
        };
        let prefix: &PathFragment = if sibling_repository_layout {
            &EXPERIMENTAL_EXTERNAL_PATH_PREFIX
        } else {
            &EXTERNAL_PATH_PREFIX
        };
        if tofind.starts_with(prefix) {
            // Using the path prefix can be either "external" or "..", depending on whether the
            // sibling repository layout is used.
            match RepositoryName::create(tofind.get_segment(1)) {
                Ok(repository) => Some(PackageIdentifier::create(
                    repository,
                    tofind.sub_fragment_from(2),
                )),
                // The path corresponds to an invalid label.
                Err(_) => None,
            }
        } else {
            Some(PackageIdentifier::create_in_main_repo(tofind))
        }
    }

    /// Parses `[@repo]//pkg`, `@@repo//pkg` or `pkg`.
    // Java: com/google/devtools/build/lib/cmdline/PackageIdentifier.java:145 PackageIdentifier.parse
    pub fn parse(input: &str) -> Result<PackageIdentifier, LabelSyntaxException> {
        if input.contains(':') {
            return Err(syntax_error(format!(
                "invalid package identifier '{}': contains ':'",
                input
            )));
        }
        let parts = LabelParserParts::parse(&format!("{}:dummy_target", input))?;
        let repo_name = match parts.repo() {
            None => MAIN.clone(),
            Some(repo) => RepositoryName::create_unvalidated(repo),
        };
        Ok(Self::create(repo_name, PathFragment::create(parts.pkg())))
    }

    // Java: com/google/devtools/build/lib/cmdline/PackageIdentifier.java:155 PackageIdentifier.getRepository
    pub fn get_repository(&self) -> &RepositoryName {
        &self.0.repository
    }

    // Java: com/google/devtools/build/lib/cmdline/PackageIdentifier.java:159 PackageIdentifier.getPackageFragment
    pub fn get_package_fragment(&self) -> &PathFragment {
        &self.0.pkg_name
    }

    /// The package path relative to its source root (the package name, for all repositories).
    // Java: com/google/devtools/build/lib/cmdline/PackageIdentifier.java:169 PackageIdentifier.getSourceRoot
    pub fn get_source_root(&self) -> &PathFragment {
        &self.0.pkg_name
    }

    /// The top level dir after the root.
    // Java: com/google/devtools/build/lib/cmdline/PackageIdentifier.java:178 PackageIdentifier.getTopLevelDir
    pub fn get_top_level_dir(&self) -> &str {
        if self.get_source_root().is_empty() {
            ""
        } else {
            self.get_source_root().get_segment(0)
        }
    }

    /// The package path fragment for derived artifacts: the package name in the main repo or with
    /// the sibling layout, otherwise `external/<repo>/<pkg>`.
    // Java: com/google/devtools/build/lib/cmdline/PackageIdentifier.java:188 PackageIdentifier.getPackagePath
    pub fn get_package_path(&self, sibling_repository_layout: bool) -> PathFragment {
        if self.0.repository.is_main() || sibling_repository_layout {
            self.0.pkg_name.clone()
        } else {
            EXTERNAL_PATH_PREFIX
                .get_relative_str(self.0.repository.get_name())
                .get_relative(&self.0.pkg_name)
        }
    }

    // Java: com/google/devtools/build/lib/cmdline/PackageIdentifier.java:196 PackageIdentifier.getExecPath
    pub fn get_exec_path(&self, sibling_repository_layout: bool) -> PathFragment {
        self.0
            .repository
            .get_exec_path(sibling_repository_layout)
            .get_relative(&self.0.pkg_name)
    }

    /// The runfiles path relative to the x.runfiles/main-repo/ directory.
    // Java: com/google/devtools/build/lib/cmdline/PackageIdentifier.java:204 PackageIdentifier.getRunfilesPath
    pub fn get_runfiles_path(&self) -> PathFragment {
        self.0
            .repository
            .get_runfiles_path()
            .get_relative(&self.0.pkg_name)
    }

    /// The package in label syntax; main-repo packages have no repo qualifier.
    // Java: com/google/devtools/build/lib/cmdline/PackageIdentifier.java:214 PackageIdentifier.getCanonicalForm
    pub fn get_canonical_form(&self) -> String {
        format!(
            "{}//{}",
            self.0.repository.get_canonical_form(),
            self.0.pkg_name
        )
    }

    /// An unambiguous canonical form (`@@repo//pkg`, `@@//pkg` for the main repo).
    // Java: com/google/devtools/build/lib/cmdline/PackageIdentifier.java:223 PackageIdentifier.getUnambiguousCanonicalForm
    pub fn get_unambiguous_canonical_form(&self) -> String {
        format!(
            "{}//{}",
            self.0.repository.get_name_with_at(),
            self.0.pkg_name
        )
    }

    /// A display form as simple as possible in the context of the main repository mapping.
    // Java: com/google/devtools/build/lib/cmdline/PackageIdentifier.java:244 PackageIdentifier.getDisplayForm
    pub fn get_display_form(&self, main_repository_mapping: Option<&RepositoryMapping>) -> String {
        format!(
            "{}//{}",
            self.0.repository.get_display_form(main_repository_mapping),
            self.0.pkg_name
        )
    }

    /// Java reference equality.
    pub fn ptr_eq(a: &PackageIdentifier, b: &PackageIdentifier) -> bool {
        Arc::ptr_eq(&a.0, &b.0)
    }

    pub(crate) fn cached_hash(&self) -> u64 {
        self.0.hash
    }
}

/// Main-repo packages render as the bare path ("foo/bar"); others as "@@repo//foo/bar".
// Java: com/google/devtools/build/lib/cmdline/PackageIdentifier.java:267 PackageIdentifier.toString
impl fmt::Display for PackageIdentifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.repository.is_main() {
            return f.write_str(self.0.pkg_name.get_path_string());
        }
        f.write_str(&self.get_canonical_form())
    }
}

impl fmt::Debug for PackageIdentifier {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "PackageIdentifier({})",
            self.get_unambiguous_canonical_form()
        )
    }
}

// Java: com/google/devtools/build/lib/cmdline/PackageIdentifier.java:275 PackageIdentifier.equals
impl PartialEq for PackageIdentifier {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || *self.0 == *other.0
    }
}

impl Eq for PackageIdentifier {}

// Java: com/google/devtools/build/lib/cmdline/PackageIdentifier.java:289 PackageIdentifier.hashCode
impl Hash for PackageIdentifier {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.0.hash);
    }
}

// Java: com/google/devtools/build/lib/cmdline/PackageIdentifier.java:295 PackageIdentifier.compareTo
impl Ord for PackageIdentifier {
    fn cmp(&self, that: &Self) -> Ordering {
        // Fast-paths for the common case of the same package or a package in the same repository.
        if Arc::ptr_eq(&self.0, &that.0) {
            return Ordering::Equal;
        }
        if RepositoryName::ptr_eq(&self.0.repository, &that.0.repository) {
            return self.0.pkg_name.cmp(&that.0.pkg_name);
        }
        self.0
            .repository
            .get_name()
            .as_bytes()
            .cmp(that.0.repository.get_name().as_bytes())
            .then_with(|| self.0.pkg_name.cmp(&that.0.pkg_name))
            // PORT: Java returns 0 here for equal names that differ only in repo visibility;
            // the tie-break keeps Ord consistent with Eq.
            .then_with(|| self.0.repository.cmp(&that.0.repository))
    }
}

impl PartialOrd for PackageIdentifier {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
