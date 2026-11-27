//! Port of `RepositoryName`.

use std::cmp::Ordering;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::{Arc, LazyLock};

use blaze_vfs::PathFragment;

use crate::interner::{WeakInterner, stable_hash};
use crate::label_constants::{EXPERIMENTAL_EXTERNAL_PATH_PREFIX, EXTERNAL_PATH_PREFIX};
use crate::label_parser::syntax_error;
use crate::label_syntax_exception::{EvalException, LabelSyntaxException};
use crate::repository_mapping::RepositoryMapping;
use crate::string_utilities::sanitize_control_chars;

/// The canonical name of an external repository (Java `RepositoryName`).
///
/// Immutable, cheap to clone (one `Arc`), `Send + Sync`. Visible names created by
/// [`RepositoryName::create`] are interned. A name may also be "not visible" (see
/// [`RepositoryName::to_non_visible`]): it then remembers the context repo it was requested from
/// and renders as `@@[unknown repo 'x' requested from @@ctx]`.
// PORT: skipped, serialization for remote analysis caching (not ported, ruling 2026-11-19 (1)): RepositoryName.repositoryNameCodec, RepositoryName.Codec.getEncodedClass, RepositoryName.Codec.serialize, RepositoryName.Codec.deserialize
// Java: com/google/devtools/build/lib/cmdline/RepositoryName.java:45 RepositoryName
#[derive(Clone)]
pub struct RepositoryName(Arc<RepositoryNameInner>);

struct RepositoryNameInner {
    name: Arc<str>,
    /// The context repository where this name was requested if it is not visible from there.
    context_repo_if_not_visible: Option<RepositoryName>,
    /// If `context_repo_if_not_visible` is set, the suffix appended to the error.
    did_you_mean_suffix: Option<String>,
    hash: u64,
}

impl PartialEq for RepositoryNameInner {
    fn eq(&self, other: &Self) -> bool {
        self.hash == other.hash
            && self.name == other.name
            && self.context_repo_if_not_visible == other.context_repo_if_not_visible
            && self.did_you_mean_suffix == other.did_you_mean_suffix
    }
}

impl Eq for RepositoryNameInner {}

// PORT: Java uses a Caffeine cache with weak values; this is a weak-value interner.
// Java: com/google/devtools/build/lib/cmdline/RepositoryName.java:66 RepositoryName.repositoryNameCache
static INTERNER: LazyLock<WeakInterner<RepositoryNameInner>> = LazyLock::new(WeakInterner::new);

/// `@bazel_tools`.
// Java: com/google/devtools/build/lib/cmdline/RepositoryName.java:48 RepositoryName.BAZEL_TOOLS
pub static BAZEL_TOOLS: LazyLock<RepositoryName> =
    LazyLock::new(|| RepositoryName::new_interned("bazel_tools"));

/// The main repository (empty name).
// Java: com/google/devtools/build/lib/cmdline/RepositoryName.java:50 RepositoryName.MAIN
pub static MAIN: LazyLock<RepositoryName> = LazyLock::new(|| RepositoryName::new_interned(""));

/// `@_builtins`.
// Java: com/google/devtools/build/lib/cmdline/RepositoryName.java:53 RepositoryName.BUILTINS
pub static BUILTINS: LazyLock<RepositoryName> =
    LazyLock::new(|| RepositoryName::new_interned("_builtins"));

// Java: com/google/devtools/build/lib/cmdline/RepositoryName.java:55 RepositoryName.VALID_REPO_NAME
fn matches_valid_repo_name(name: &str) -> bool {
    // [\w\-.+]*  (Java \w is [a-zA-Z_0-9])
    name.bytes()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, b'_' | b'-' | b'.' | b'+'))
}

// Java: com/google/devtools/build/lib/cmdline/RepositoryName.java:58 RepositoryName.VALID_USER_PROVIDED_NAME
fn matches_valid_user_provided_name(name: &str) -> bool {
    // [a-zA-Z0-9][-.\w]*$ with Matcher.matches()
    let b = name.as_bytes();
    !b.is_empty()
        && b[0].is_ascii_alphanumeric()
        && b[1..]
            .iter()
            .all(|&c| c.is_ascii_alphanumeric() || matches!(c, b'-' | b'.' | b'_'))
}

/// Whether `name` matches `VALID_MODULE_NAME`: begins with a lowercase letter, ends with a
/// lowercase letter or a digit, and contains only lowercase letters, digits, or one of '._-'.
// Java: com/google/devtools/build/lib/cmdline/RepositoryName.java:64 RepositoryName.VALID_MODULE_NAME
pub fn is_valid_module_name(name: &str) -> bool {
    // [a-z]([a-z0-9._-]*[a-z0-9])?
    let b = name.as_bytes();
    if b.is_empty() || !b[0].is_ascii_lowercase() {
        return false;
    }
    if b.len() == 1 {
        return true;
    }
    let last = b[b.len() - 1];
    (last.is_ascii_lowercase() || last.is_ascii_digit())
        && b[1..b.len() - 1].iter().all(|&c| {
            c.is_ascii_lowercase() || c.is_ascii_digit() || matches!(c, b'.' | b'_' | b'-')
        })
}

impl RepositoryName {
    // Java: com/google/devtools/build/lib/cmdline/RepositoryName.java:160 RepositoryName.<init>
    fn new_inner(
        name: Arc<str>,
        context_repo_if_not_visible: Option<RepositoryName>,
        did_you_mean_suffix: Option<String>,
    ) -> RepositoryNameInner {
        let hash = stable_hash(&(
            &*name,
            context_repo_if_not_visible.as_ref().map(|r| r.0.hash),
            did_you_mean_suffix.as_deref(),
        ));
        RepositoryNameInner {
            name,
            context_repo_if_not_visible,
            did_you_mean_suffix,
            hash,
        }
    }

    fn new_interned(name: &str) -> RepositoryName {
        let inner = Self::new_inner(Arc::from(name), None, None);
        RepositoryName(INTERNER.intern(inner.hash, inner))
    }

    /// Makes sure that name is a valid repository name and creates a new `RepositoryName`.
    // Java: com/google/devtools/build/lib/cmdline/RepositoryName.java:80 RepositoryName.create
    pub fn create(name: &str) -> Result<RepositoryName, LabelSyntaxException> {
        if name.is_empty() {
            return Ok(MAIN.clone());
        }
        if name == BUILTINS.get_name() {
            return Ok(BUILTINS.clone());
        }
        Self::validate(name)?;
        Ok(Self::new_interned(name))
    }

    /// Creates a `RepositoryName` from a known-valid string.
    ///
    /// Panics if the name is in fact invalid (Java: the cache loader's `LabelSyntaxException`
    /// escapes as an unchecked `CompletionException`).
    // Java: com/google/devtools/build/lib/cmdline/RepositoryName.java:97 RepositoryName.createUnvalidated
    pub fn create_unvalidated(name: &str) -> RepositoryName {
        if name.is_empty() {
            return MAIN.clone();
        }
        if name == BUILTINS.get_name() {
            return BUILTINS.clone();
        }
        if let Err(e) = Self::validate(name) {
            panic!("java.util.concurrent.CompletionException: {}", e);
        }
        Self::new_interned(name)
    }

    /// Extracts the repository name from a path fragment created with
    /// `PackageIdentifier.getSourceRoot`: returns the repository name and the path with the
    /// "external/"-prefix and repository name stripped, or `None`.
    // Java: com/google/devtools/build/lib/cmdline/RepositoryName.java:119 RepositoryName.fromPathFragment
    pub fn from_path_fragment(
        path: &PathFragment,
        sibling_repository_layout: bool,
    ) -> Option<(RepositoryName, PathFragment)> {
        if !path.is_multi_segment() {
            return None;
        }

        let prefix: &PathFragment = if sibling_repository_layout {
            &EXPERIMENTAL_EXTERNAL_PATH_PREFIX
        } else {
            &EXTERNAL_PATH_PREFIX
        };
        if !path.starts_with(prefix) {
            return None;
        }

        match Self::create(path.get_segment(1)) {
            Ok(repo_name) => {
                let sub_path = path.sub_fragment_from(2);
                Some((repo_name, sub_path))
            }
            Err(_) => None,
        }
    }

    /// Performs validity checking; the error message is sanitized.
    // Java: com/google/devtools/build/lib/cmdline/RepositoryName.java:178 RepositoryName.validate
    pub(crate) fn validate(name: &str) -> Result<(), LabelSyntaxException> {
        if name.is_empty() || name == "_builtins" {
            return Ok(());
        }

        // Some special cases for more user-friendly error messages.
        if name == "." || name == ".." {
            return Err(syntax_error(format!(
                "invalid repository name '{}': repo names are not allowed to be '{}'",
                name, name
            )));
        }

        if !matches_valid_repo_name(name) {
            return Err(syntax_error(format!(
                "invalid repository name '{}': repo names may contain only A-Z, a-z, 0-9, '-', \
                 '_', '.' and '+'",
                sanitize_control_chars(name)
            )));
        }
        Ok(())
    }

    /// Validates a repo name provided by the user. Such names have tighter restrictions: they
    /// must start with a letter or digit and cannot contain a plus (+).
    // Java: com/google/devtools/build/lib/cmdline/RepositoryName.java:201 RepositoryName.validateUserProvidedRepoName
    pub fn validate_user_provided_repo_name(name: &str) -> Result<(), EvalException> {
        if !matches_valid_user_provided_name(name) {
            return Err(EvalException::new(format!(
                "invalid user-provided repo name '{}': valid names may contain only A-Z, a-z, \
                 0-9, '-', '_', '.', and must start with a letter or a number",
                sanitize_control_chars(name)
            )));
        }
        Ok(())
    }

    /// Returns true if the given name cannot possibly be a canonical repository name.
    // Java: com/google/devtools/build/lib/cmdline/RepositoryName.java:211 RepositoryName.isApparent
    pub fn is_apparent(name: &str) -> bool {
        !name.is_empty() && !name.contains('+')
    }

    /// Returns the bare repository name without the leading "@".
    // Java: com/google/devtools/build/lib/cmdline/RepositoryName.java:216 RepositoryName.getName
    pub fn get_name(&self) -> &str {
        &self.0.name
    }

    /// Returns the marker file name for this repository.
    // Java: com/google/devtools/build/lib/cmdline/RepositoryName.java:221 RepositoryName.getMarkerFileName
    pub fn get_marker_file_name(&self) -> String {
        format!("@{}.marker", self.0.name)
    }

    /// Returns an instance indicating the requested repository name is not visible from
    /// `context_repo`. Panics if `context_repo` is itself not visible.
    // Java: com/google/devtools/build/lib/cmdline/RepositoryName.java:230 RepositoryName.toNonVisible
    pub fn to_non_visible_with_suffix(
        &self,
        context_repo: &RepositoryName,
        did_you_mean_suffix: &str,
    ) -> RepositoryName {
        assert!(context_repo.is_visible(), "IllegalArgumentException");
        RepositoryName(Arc::new(Self::new_inner(
            self.0.name.clone(),
            Some(context_repo.clone()),
            Some(did_you_mean_suffix.to_owned()),
        )))
    }

    /// [`RepositoryName::to_non_visible_with_suffix`] with an empty suffix.
    // Java: com/google/devtools/build/lib/cmdline/RepositoryName.java:238 RepositoryName.toNonVisible
    pub fn to_non_visible(&self, context_repo: &RepositoryName) -> RepositoryName {
        self.to_non_visible_with_suffix(context_repo, "")
    }

    // Java: com/google/devtools/build/lib/cmdline/RepositoryName.java:242 RepositoryName.isVisible
    pub fn is_visible(&self) -> bool {
        self.0.context_repo_if_not_visible.is_none()
    }

    // Java: com/google/devtools/build/lib/cmdline/RepositoryName.java:246 RepositoryName.isContextRepoMainRepo
    pub fn is_context_repo_main_repo(&self) -> bool {
        !self.is_visible()
            && self
                .0
                .context_repo_if_not_visible
                .as_ref()
                .is_some_and(|c| c.is_main())
    }

    /// Must only be called if `is_visible()` returns false.
    // Java: com/google/devtools/build/lib/cmdline/RepositoryName.java:251 RepositoryName.getContextRepoDisplayString
    pub fn get_context_repo_display_string(&self) -> String {
        let context = self
            .0
            .context_repo_if_not_visible
            .as_ref()
            .expect("NullPointerException");
        if context.is_main() {
            "main repository".to_owned()
        } else {
            format!("repository '{}'", context)
        }
    }

    /// Returns if this is the main repository.
    // Java: com/google/devtools/build/lib/cmdline/RepositoryName.java:261 RepositoryName.isMain
    pub fn is_main(&self) -> bool {
        *self == *MAIN
    }

    /// Returns the repository name with two leading "@"s (the canonical repo name form).
    // Java: com/google/devtools/build/lib/cmdline/RepositoryName.java:270 RepositoryName.getNameWithAt
    pub fn get_name_with_at(&self) -> String {
        if let Some(context) = &self.0.context_repo_if_not_visible {
            return format!(
                "@@[unknown repo '{}' requested from {}{}]",
                self.0.name,
                context,
                self.0.did_you_mean_suffix.as_deref().unwrap_or("null")
            );
        }
        format!("@@{}", self.0.name)
    }

    /// Returns the name with leading "@"s, except for the main repo, which is "".
    // Java: com/google/devtools/build/lib/cmdline/RepositoryName.java:284 RepositoryName.getCanonicalForm
    pub fn get_canonical_form(&self) -> String {
        if self.is_main() {
            String::new()
        } else {
            self.get_name_with_at()
        }
    }

    /// Returns the repository part of a label's display form: "" for the main repo, `@apparent`
    /// if the main repo mapping has an apparent name for it, `@@canonical` otherwise.
    ///
    /// Panics if `main_repository_mapping` is given and its context repo is not the main repo.
    // Java: com/google/devtools/build/lib/cmdline/RepositoryName.java:305 RepositoryName.getDisplayForm
    pub fn get_display_form(&self, main_repository_mapping: Option<&RepositoryMapping>) -> String {
        assert!(
            main_repository_mapping.is_none_or(|m| m.context_repo().is_main()),
            "IllegalArgumentException"
        );
        if !self.is_visible() {
            return self.get_name_with_at();
        }
        if self.is_main() {
            // Packages in the main repository can always use repo-relative form.
            return String::new();
        }
        let Some(main_repository_mapping) = main_repository_mapping else {
            return self.get_name_with_at();
        };
        // If possible, represent the repository with a non-canonical label using the apparent name
        // the main repository has for it, otherwise fall back to a canonical label.
        match main_repository_mapping.get_inverse(self) {
            Some(apparent_name) => format!("@{}", apparent_name),
            None => self.get_name_with_at(),
        }
    }

    /// Returns the runfiles/execRoot path for this repository: empty for the main repo,
    /// `external/<name>` or (sibling layout) `../<name>` otherwise.
    // Java: com/google/devtools/build/lib/cmdline/RepositoryName.java:333 RepositoryName.getExecPath
    pub fn get_exec_path(&self, sibling_repository_layout: bool) -> PathFragment {
        if self.is_main() {
            return PathFragment::empty_fragment();
        }
        let prefix: &PathFragment = if sibling_repository_layout {
            &EXPERIMENTAL_EXTERNAL_PATH_PREFIX
        } else {
            &EXTERNAL_PATH_PREFIX
        };
        prefix.get_relative_str(self.get_name())
    }

    /// Returns the runfiles path relative to the x.runfiles/main-repo directory.
    // Java: com/google/devtools/build/lib/cmdline/RepositoryName.java:346 RepositoryName.getRunfilesPath
    pub fn get_runfiles_path(&self) -> PathFragment {
        if self.is_main() {
            PathFragment::empty_fragment()
        } else {
            PathFragment::create("..").get_relative_str(self.get_name())
        }
    }

    /// Java reference equality.
    pub fn ptr_eq(a: &RepositoryName, b: &RepositoryName) -> bool {
        Arc::ptr_eq(&a.0, &b.0)
    }

    pub(crate) fn cached_hash(&self) -> u64 {
        self.0.hash
    }
}

/// Same as [`RepositoryName::get_name_with_at`].
// Java: com/google/devtools/build/lib/cmdline/RepositoryName.java:354 RepositoryName.toString
impl fmt::Display for RepositoryName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.get_name_with_at())
    }
}

impl fmt::Debug for RepositoryName {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "RepositoryName({})", self.get_name_with_at())
    }
}

// Java: com/google/devtools/build/lib/cmdline/RepositoryName.java:359 RepositoryName.equals
impl PartialEq for RepositoryName {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0) || *self.0 == *other.0
    }
}

impl Eq for RepositoryName {}

// Java: com/google/devtools/build/lib/cmdline/RepositoryName.java:372 RepositoryName.hashCode
impl Hash for RepositoryName {
    fn hash<H: Hasher>(&self, state: &mut H) {
        state.write_u64(self.0.hash);
    }
}

// PORT: Java RepositoryName is not Comparable. Callers that order repositories (e.g.
// PackageIdentifier.compareTo, Label.SimpleRepoMappingRecorder) order by getName(); this Ord does
// the same and then breaks ties on the not-visible fields so that it stays consistent with Eq.
impl Ord for RepositoryName {
    fn cmp(&self, other: &Self) -> Ordering {
        if Arc::ptr_eq(&self.0, &other.0) {
            return Ordering::Equal;
        }
        self.0
            .name
            .as_bytes()
            .cmp(other.0.name.as_bytes())
            .then_with(|| {
                self.0
                    .context_repo_if_not_visible
                    .cmp(&other.0.context_repo_if_not_visible)
            })
            .then_with(|| self.0.did_you_mean_suffix.cmp(&other.0.did_you_mean_suffix))
    }
}

impl PartialOrd for RepositoryName {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
