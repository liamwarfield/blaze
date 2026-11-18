//! Port of `lib/analysis/SymlinkEntry.java`.

use std::fmt;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use blaze_vfs::PathFragment;

use crate::actions::artifact::Artifact;

/// An entry in the runfiles map.
///
/// The PathFragment must not be an absolute path nor contain "..". Overlapping runfiles links are
/// also refused. This is the case where you ask to create a link to "foo" and also
/// "foo/bar.txt", i.e. you're asking it to make "foo" both a file (symlink) and a directory.
///
/// Links to directories are heavily discouraged.
///
/// Java deliberately gives this class no `equals()`: the semantics of the symlinks nested set
/// dictate that later entries overwrite earlier ones, while nested sets return duplicate entries
/// only once. With three entries for the same path where the first and the last are equal and
/// the middle one differs, value equality would make the *middle* one take effect. Identity
/// equality prevents this, so [`PartialEq`] and [`Hash`] here compare the shared allocation
/// (Java object identity).
// Java: com/google/devtools/build/lib/analysis/SymlinkEntry.java:51 SymlinkEntry
#[derive(Clone)]
pub struct SymlinkEntry(Arc<SymlinkEntryInner>);

struct SymlinkEntryInner {
    path: PathFragment,
    artifact: Artifact,
}

impl SymlinkEntry {
    // Java: com/google/devtools/build/lib/analysis/SymlinkEntry.java:55 SymlinkEntry.<init>
    pub fn new(path: PathFragment, artifact: Artifact) -> SymlinkEntry {
        SymlinkEntry(Arc::new(SymlinkEntryInner { path, artifact }))
    }

    // Java: com/google/devtools/build/lib/analysis/SymlinkEntry.java:61 SymlinkEntry.getPathString
    pub fn get_path_string(&self) -> &str {
        self.0.path.get_path_string()
    }

    // Java: com/google/devtools/build/lib/analysis/SymlinkEntry.java:65 SymlinkEntry.getPath
    pub fn get_path(&self) -> &PathFragment {
        &self.0.path
    }

    // Java: com/google/devtools/build/lib/analysis/SymlinkEntry.java:70 SymlinkEntry.getArtifact
    pub fn get_artifact(&self) -> &Artifact {
        &self.0.artifact
    }

    // Java: com/google/devtools/build/lib/analysis/SymlinkEntry.java:75 SymlinkEntry.isImmutable
    pub fn is_immutable(&self) -> bool {
        true
    }

    /// Starlark `repr`: `SymlinkEntry(path = "<path>", target_file = <artifact repr>)`.
    // Java: com/google/devtools/build/lib/analysis/SymlinkEntry.java:80 SymlinkEntry.repr
    pub fn repr(&self) -> String {
        let mut printer = String::new();
        printer.push_str("SymlinkEntry(path = ");
        printer.push_str(&starlark::values::string::StarlarkStr::repr(
            self.get_path_string(),
        ));
        printer.push_str(", target_file = ");
        printer.push_str(&self.0.artifact.repr());
        printer.push(')');
        printer
    }

    /// Java `==` on `SymlinkEntry` references.
    pub fn ptr_eq(a: &SymlinkEntry, b: &SymlinkEntry) -> bool {
        Arc::ptr_eq(&a.0, &b.0)
    }
}

// Java: Object.equals (identity); see the class comment.
impl PartialEq for SymlinkEntry {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl Eq for SymlinkEntry {}

// Java: Object.hashCode (identity).
impl Hash for SymlinkEntry {
    fn hash<H: Hasher>(&self, state: &mut H) {
        (Arc::as_ptr(&self.0) as usize).hash(state);
    }
}

impl fmt::Debug for SymlinkEntry {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.repr())
    }
}
