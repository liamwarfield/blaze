//! Port of `lib/actions/ArtifactRoot.java`.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use blaze_cmdline::label_constants::EXTERNAL_REPOSITORY_LOCATION;
use blaze_util::check_argument;
use blaze_vfs::{Path, PathFragment, Root};

/// The type of an artifact root.
// Java: com/google/devtools/build/lib/actions/ArtifactRoot.java:181 ArtifactRoot.RootType
#[allow(non_camel_case_types, clippy::upper_case_acronyms)]
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum RootType {
    MAIN_SOURCE,
    EXTERNAL_SOURCE,
    OUTPUT,
    SIBLING_MAIN_OUTPUT,
    SIBLING_EXTERNAL_OUTPUT,
}

/// A root for an artifact. The roots are the directories containing artifacts, and they are
/// mapped together into a single directory tree to form the execution environment. There are two
/// kinds of roots, source roots and derived roots. Source roots correspond to entries of the
/// package path, and they can be anywhere on disk. Derived roots correspond to output directories;
/// there are generally different output directories for different configurations, and different
/// types of output (bin, genfiles, includes, etc.).
///
/// When mapping the roots into a single directory tree, the source roots are merged, such that
/// each package is accessed in its entirety from a single source root. The package cache is
/// responsible for determining that mapping. The derived roots, on the other hand, have to be
/// distinct. (It is currently allowed to have a derived root that is the prefix of another one.)
///
/// The derived roots must have paths that point inside the exec root, i.e. below the directory
/// that is the root of the merged directory tree.
// Java: com/google/devtools/build/lib/actions/ArtifactRoot.java:93 ArtifactRoot
// PORT: Java interns roots (weak interner); `Arc` sharing plus structural equality gives the same
// observable behaviour.
#[derive(Clone)]
pub struct ArtifactRoot(Arc<ArtifactRootInner>);

struct ArtifactRootInner {
    root: Root,
    exec_path: PathFragment,
    root_type: RootType,
}

impl ArtifactRoot {
    /// Do not use except in tests and in `SkyframeExecutor`.
    ///
    /// Returns the given path as a source root. The path may not be `null`.
    // Java: com/google/devtools/build/lib/actions/ArtifactRoot.java:102 ArtifactRoot.asSourceRoot
    pub fn as_source_root(root: Root) -> ArtifactRoot {
        ArtifactRoot::new(root, PathFragment::empty_fragment(), RootType::MAIN_SOURCE)
    }

    /// Returns the given path as a source root for an external repository: the root is
    /// `<output_base>/external/<canonical repo name>`.
    // Java: com/google/devtools/build/lib/actions/ArtifactRoot.java:115 ArtifactRoot.asExternalSourceRoot
    pub fn as_external_source_root(root: Root) -> ArtifactRoot {
        let parent_ends_with_external = root
            .as_path()
            .and_then(|p| p.as_fragment().get_parent_directory())
            .is_some_and(|parent| parent.ends_with(&EXTERNAL_REPOSITORY_LOCATION));
        check_argument!(parent_ends_with_external);
        ArtifactRoot::new(
            root,
            PathFragment::empty_fragment(),
            RootType::EXTERNAL_SOURCE,
        )
    }

    /// Constructs an ArtifactRoot given the output prefixes. (eg, "bin"), and (eg, "testlogs")
    /// relative to the execRoot.
    ///
    /// Be careful with this method - all derived roots must be within the derived artifacts tree,
    /// defined in ArtifactFactory (see `ArtifactFactory#isDerivedArtifact`).
    // Java: com/google/devtools/build/lib/actions/ArtifactRoot.java:135 ArtifactRoot.asDerivedRoot
    // Java: com/google/devtools/build/lib/actions/ArtifactRoot.java:152 ArtifactRoot.asDerivedRoot
    pub fn as_derived_root(
        exec_root: &Path,
        root_type: RootType,
        prefixes: &[&str],
    ) -> ArtifactRoot {
        let mut exec_path = PathFragment::empty_fragment();
        for prefix in prefixes {
            // Tests can have empty segments here, be gentle to them.
            if !prefix.is_empty() {
                exec_path = exec_path.get_child(prefix);
            }
        }
        ArtifactRoot::as_derived_root_fragment(exec_root, root_type, exec_path)
    }

    /// Returns an ArtifactRoot for the given exec path, relative to the exec root.
    // Java: com/google/devtools/build/lib/actions/ArtifactRoot.java:135 ArtifactRoot.asDerivedRoot
    pub fn as_derived_root_fragment(
        exec_root: &Path,
        root_type: RootType,
        exec_path: PathFragment,
    ) -> ArtifactRoot {
        // Make sure that we are not creating a derived artifact under the execRoot.
        check_argument!(!exec_path.is_empty(), "empty execPath");
        check_argument!(
            !exec_path.is_absolute(),
            "execPath must be relative: {}",
            exec_path
        );
        check_argument!(
            !exec_path.contains_uplevel_references(),
            "execPath: {} contains parent directory reference (..)",
            exec_path
        );
        check_argument!(
            is_output_root_type(root_type),
            "{:?} is not a derived root type",
            root_type
        );
        let root = exec_root.get_relative(&exec_path);
        ArtifactRoot::new(Root::from_path(root), exec_path, root_type)
    }

    fn new(root: Root, exec_path: PathFragment, root_type: RootType) -> ArtifactRoot {
        ArtifactRoot(Arc::new(ArtifactRootInner {
            root,
            exec_path,
            root_type,
        }))
    }

    // Java: com/google/devtools/build/lib/actions/ArtifactRoot.java:207 ArtifactRoot.getRoot
    pub fn get_root(&self) -> &Root {
        &self.0.root
    }

    /// Returns the path fragment from the exec root to the actual root. For source roots, this
    /// returns the empty fragment.
    // Java: com/google/devtools/build/lib/actions/ArtifactRoot.java:215 ArtifactRoot.getExecPath
    pub fn get_exec_path(&self) -> &PathFragment {
        &self.0.exec_path
    }

    // Java: com/google/devtools/build/lib/actions/ArtifactRoot.java:220 ArtifactRoot.getExecPathString
    pub fn get_exec_path_string(&self) -> &str {
        self.0.exec_path.get_path_string()
    }

    // Java: com/google/devtools/build/lib/actions/ArtifactRoot.java:224 ArtifactRoot.getRootType
    pub fn get_root_type(&self) -> RootType {
        self.0.root_type
    }

    // Java: com/google/devtools/build/lib/actions/ArtifactRoot.java:228 ArtifactRoot.isSourceRoot
    pub fn is_source_root(&self) -> bool {
        self.0.root_type == RootType::MAIN_SOURCE || self.0.root_type == RootType::EXTERNAL_SOURCE
    }

    // Java: com/google/devtools/build/lib/actions/ArtifactRoot.java:238 ArtifactRoot.isExternal
    pub fn is_external(&self) -> bool {
        self.0.root_type == RootType::EXTERNAL_SOURCE
            || self.0.root_type == RootType::SIBLING_EXTERNAL_OUTPUT
    }

    /// Returns true if the ArtifactRoot is a legacy derived root (`RootType::OUTPUT`).
    // Java: com/google/devtools/build/lib/actions/ArtifactRoot.java:246 ArtifactRoot.isLegacy
    pub fn is_legacy(&self) -> bool {
        self.0.root_type == RootType::OUTPUT
    }

    /// Java `compareTo`: compares the roots only.
    // Java: com/google/devtools/build/lib/actions/ArtifactRoot.java:251 ArtifactRoot.compareTo
    pub fn compare_to(&self, o: &ArtifactRoot) -> std::cmp::Ordering {
        self.0.root.cmp(&o.0.root)
    }

    /// Java `repr`.
    // Java: com/google/devtools/build/lib/actions/ArtifactRoot.java:302 ArtifactRoot.repr
    pub fn repr(&self) -> &'static str {
        if self.is_source_root() {
            "<source root>"
        } else {
            "<derived root>"
        }
    }

    /// `CommandLineItem.expandToCommandLine`.
    // Java: com/google/devtools/build/lib/actions/ArtifactRoot.java:307 ArtifactRoot.expandToCommandLine
    pub fn expand_to_command_line(&self) -> String {
        self.get_exec_path_string().to_string()
    }
}

// Java: com/google/devtools/build/lib/actions/ArtifactRoot.java:162 ArtifactRoot.isOutputRootType
fn is_output_root_type(root_type: RootType) -> bool {
    root_type == RootType::SIBLING_MAIN_OUTPUT
        || root_type == RootType::SIBLING_EXTERNAL_OUTPUT
        || root_type == RootType::OUTPUT
}

// Java: com/google/devtools/build/lib/actions/ArtifactRoot.java:286 ArtifactRoot.equals
impl PartialEq for ArtifactRoot {
    fn eq(&self, r: &Self) -> bool {
        Arc::ptr_eq(&self.0, &r.0)
            || (self.0.root == r.0.root
                && self.0.exec_path == r.0.exec_path
                && self.0.root_type == r.0.root_type)
    }
}

impl Eq for ArtifactRoot {}

// Java: com/google/devtools/build/lib/actions/ArtifactRoot.java:256 ArtifactRoot.hashCode
impl Hash for ArtifactRoot {
    fn hash<H: Hasher>(&self, state: &mut H) {
        self.0.root.hash(state);
        self.0.exec_path.hash(state);
        self.0.root_type.hash(state);
    }
}

// Java: com/google/devtools/build/lib/actions/ArtifactRoot.java:297 ArtifactRoot.toString
impl fmt::Display for ArtifactRoot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}{}",
            self.0.root,
            if self.is_source_root() {
                "[source]"
            } else {
                "[derived]"
            }
        )
    }
}

impl fmt::Debug for ArtifactRoot {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

impl blaze_collect::command_line_item::CommandLineItem for ArtifactRoot {
    fn expand_to_command_line(&self) -> String {
        ArtifactRoot::expand_to_command_line(self)
    }
}
