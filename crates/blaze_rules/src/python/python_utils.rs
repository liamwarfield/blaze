//! Port of `rules/python/PythonUtils.java`: the `__init__.py` empty-files supplier that
//! `py_builtins.merge_runfiles_with_generated_inits_empty_files_supplier` installs.

use blaze_actions::analysis::runfiles::{EmptyFilesSupplier, JavaHashMap};
use blaze_util::Fingerprint;
use blaze_vfs::PathFragment;
use indexmap::IndexSet;

// Java: com/google/devtools/build/lib/rules/python/PythonUtils.java:28 PythonUtils.INIT_PY
pub const INIT_PY: &str = "__init__.py";
// Java: com/google/devtools/build/lib/rules/python/PythonUtils.java:29 PythonUtils.INIT_PYC
pub const INIT_PYC: &str = "__init__.pyc";
// Java: com/google/devtools/build/lib/rules/python/PythonUtils.java:30 PythonUtils.PYCACHE
pub const PYCACHE: &str = "__pycache__";

/// `FileType.of(".py", ".so", ".pyc")`.
// Java: com/google/devtools/build/lib/rules/python/PythonUtils.java:32 PythonUtils.REQUIRES_INIT_PY
const REQUIRES_INIT_PY: &[&str] = &[".py", ".so", ".pyc"];

/// `REQUIRES_INIT_PY.matches(source)`: `FileType.matches(HasFileType)` tests the normalized path
/// string against each extension.
// Java: com/google/devtools/build/lib/util/FileType.java:159 FileType.matches
fn requires_init_py(source: &PathFragment) -> bool {
    let path = source.file_path_for_file_type_matcher();
    REQUIRES_INIT_PY.iter().any(|ext| path.ends_with(ext))
}

/// Used to get the set of empty `__init__.py` files to be added to a given set of files to allow
/// the Python runtime to import subdirectories potentially containing Python code to be imported
/// as packages. Ideally this feature goes away with `--incompatible_default_to_explicit_init_py`
/// as the long term default behavior.
// Java: com/google/devtools/build/lib/rules/python/PythonUtils.java:40 PythonUtils.GetInitPyFiles
pub struct PythonUtilsGetInitPyFiles {
    is_package_init: fn(&PathFragment) -> bool,
    /// `UUID` as (most significant, least significant) bits.
    guid: (i64, i64),
}

impl PythonUtilsGetInitPyFiles {
    /// The predicate `is_package_init(source)` should be true when a given source is known to be
    /// a valid `__init__.py` file equivalent, meaning no empty `__init__.py` file need be created.
    /// Useful for custom Python runtimes that may have non-standard Python package import logic.
    ///
    /// `guid` (most and least significant bits) uniquely identifies the particular
    /// `is_package_init` predicate for the purpose of fingerprinting this supplier.
    // Java: com/google/devtools/build/lib/rules/python/PythonUtils.java:52 GetInitPyFiles.<init>
    pub const fn new(
        is_package_init: fn(&PathFragment) -> bool,
        guid: (i64, i64),
    ) -> PythonUtilsGetInitPyFiles {
        PythonUtilsGetInitPyFiles {
            is_package_init,
            guid,
        }
    }

    /// Returns the set of empty `__init__.py(c)` files to be added to a given set of files to
    /// allow the Python runtime to find the `.py` and `.so` files present in the tree.
    ///
    /// PORT: java-hash-order. Java collects the result in a `new HashSet<>()` and returns
    /// `ImmutableSet.copyOf` of it, so the order is `HashSet` iteration order, which reaches
    /// `runfiles.empty_filenames` (and from there the PythonZipper command line). The result is
    /// collected in a [`JavaHashMap`] with `HashSet`'s default table (16 buckets, threshold 12:
    /// `with_expected_size(12)` allocates exactly that).
    // Java: com/google/devtools/build/lib/rules/python/PythonUtils.java:72 GetInitPyFiles.getInitPyFiles
    fn get_init_py_files(&self, manifest_files: &IndexSet<PathFragment>) -> Vec<PathFragment> {
        let mut result: JavaHashMap<()> = JavaHashMap::with_expected_size(12);
        // A set of directories that already have package init files.
        let mut has_package_init_dirs: IndexSet<PathFragment> = IndexSet::new(); // For b/142135992.

        // Find directories containing Python package init files based on a caller supplied test
        // in order to support non-standard Python package init naming schemes.
        // This loop is done prior to the one below as we assume no order in the set and that we
        // may find inits in parent directories listed after subdirectories which the nested loop
        // below would need to know of.
        for source in manifest_files {
            if (self.is_package_init)(source) {
                if let Some(parent) = source.get_parent_directory() {
                    has_package_init_dirs.insert(parent);
                }
            }
        }

        let pycache = PathFragment::create(PYCACHE);
        for source in manifest_files {
            // If we have a python or .so file at this level...
            if requires_init_py(source) {
                // ...then record that we need an __init__.py in this and all parents
                // directories...
                let mut source = source.clone();
                while source.is_multi_segment() {
                    source = source
                        .get_parent_directory()
                        .expect("a multi-segment path has a parent");
                    // ...unless it's a Python .pyc cache or we already have __init__ there.
                    if !source.ends_with(&pycache) && !has_package_init_dirs.contains(&source) {
                        let initpy = source.get_relative_str(INIT_PY);
                        let initpyc = source.get_relative_str(INIT_PYC);

                        if !manifest_files.contains(&initpy) && !manifest_files.contains(&initpyc) {
                            result.put(initpy, ());
                        }
                    }
                }
            }
        }

        result.iter().map(|(k, _)| k.clone()).collect()
    }
}

impl EmptyFilesSupplier for PythonUtilsGetInitPyFiles {
    // Java: com/google/devtools/build/lib/rules/python/PythonUtils.java:58 GetInitPyFiles.getExtraPaths
    fn get_extra_paths(&self, manifest_paths: &IndexSet<PathFragment>) -> Vec<PathFragment> {
        self.get_init_py_files(manifest_paths)
    }

    // Java: com/google/devtools/build/lib/rules/python/PythonUtils.java:63 GetInitPyFiles.fingerprint
    fn fingerprint(&self, fp: &mut Fingerprint) {
        // Java `Fingerprint.addUUID`: least significant bits, then most significant bits.
        fp.add_uuid(self.guid.1, self.guid.0);
    }

    fn get_class_name(&self) -> String {
        "com.google.devtools.build.lib.rules.python.PythonUtils$GetInitPyFiles".to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn never(_: &PathFragment) -> bool {
        false
    }

    fn supplier() -> PythonUtilsGetInitPyFiles {
        PythonUtilsGetInitPyFiles::new(never, (0, 0))
    }

    fn paths(p: &[&str]) -> IndexSet<PathFragment> {
        p.iter().map(|s| PathFragment::create(s)).collect()
    }

    fn sorted(v: Vec<PathFragment>) -> Vec<String> {
        let mut v: Vec<String> = v.iter().map(|p| p.get_path_string().to_owned()).collect();
        v.sort();
        v
    }

    #[test]
    fn adds_init_py_to_every_parent_of_python_files() {
        let extra = supplier().get_extra_paths(&paths(&["a/b/c.py", "a/d.txt"]));
        assert_eq!(sorted(extra), vec!["a/__init__.py", "a/b/__init__.py"]);
    }

    #[test]
    fn skips_existing_inits_and_pycache() {
        let extra = supplier().get_extra_paths(&paths(&[
            "a/__init__.py",
            "a/b/__pycache__/m.pyc",
            "a/c/__init__.pyc",
            "a/c/x.so",
        ]));
        // a/__init__.py and a/c/__init__.pyc exist; a/b/__pycache__ is skipped.
        assert_eq!(sorted(extra), vec!["a/b/__init__.py"]);
    }

    #[test]
    fn top_level_files_need_no_init() {
        assert!(supplier().get_extra_paths(&paths(&["x.py"])).is_empty());
    }
}
