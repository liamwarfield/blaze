//! Minimal port of `lib/util/FileType.java` and `lib/util/FileTypeSet.java`, as attribute
//! definitions need them.
// PORT: move to blaze_util (lib/util) when a crate there needs it.

/// A file type, matched by file name suffix (Java `FileType`, the `FileType.of(...)` kind).
// Java: com/google/devtools/build/lib/util/FileType.java:30 FileType
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct FileType {
    extensions: Vec<String>,
}

impl FileType {
    // Java: com/google/devtools/build/lib/util/FileType.java:43 FileType.of
    // Java: com/google/devtools/build/lib/util/FileType.java:47 FileType.of
    // Java: com/google/devtools/build/lib/util/FileType.java:51 FileType.of
    // Java: com/google/devtools/build/lib/util/FileType.java:58 FileType.SingletonFileType.<init>
    // Java: com/google/devtools/build/lib/util/FileType.java:76 FileType.ListFileType.<init>
    pub fn of(extensions: &[&str]) -> FileType {
        FileType {
            extensions: extensions.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    // Java: com/google/devtools/build/lib/util/FileType.java:148 FileType.getExtensions
    // Java: com/google/devtools/build/lib/util/FileType.java:67 FileType.SingletonFileType.getExtensions
    // Java: com/google/devtools/build/lib/util/FileType.java:85 FileType.ListFileType.getExtensions
    pub fn get_extensions(&self) -> &[String] {
        &self.extensions
    }

    /// `hasAnyExtension(path, extensions)`: `FileType.hasExtension` on Linux is `endsWith`
    /// (the case-insensitive branch is Windows only).
    // Java: com/google/devtools/build/lib/util/FileType.java:108 FileType.apply
    // Java: com/google/devtools/build/lib/util/FileType.java:62 FileType.SingletonFileType.apply
    // Java: com/google/devtools/build/lib/util/FileType.java:80 FileType.ListFileType.apply
    // Java: com/google/devtools/build/lib/util/FileType.java:131 FileType.hasAnyExtension
    // Java: com/google/devtools/build/lib/util/FileType.java:153 FileType.matches
    pub fn apply(&self, path: &str) -> bool {
        self.extensions
            .iter()
            .any(|ext| path.ends_with(ext.as_str()))
    }
}

/// `getExtensions().toString()`: `[.a, .b]`.
// Java: com/google/devtools/build/lib/util/FileType.java:102 FileType.toString
impl std::fmt::Display for FileType {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "[{}]", self.extensions.join(", "))
    }
}

// PORT: `FileType.NO_EXTENSION` and the C++ file types are ported where the C++ rules use them
// (blaze_rules `cpp_file_types`, blaze_actions `solib_symlink_action`).
// PORT: skipped, equality is derived (the extension list, as ListFileType.equals): FileType.hashCode, FileType.equals
// PORT: skipped, no caller at the pin: FileType.contains, FileType.typeMatchingPredicateFor, FileType.typeMatchingPredicateFrom
// PORT: skipped, no caller at the pin: FileType.filter, FileType.except, FileType.filterList

/// A set of file types (Java `FileTypeSet`).
// Java: com/google/devtools/build/lib/util/FileTypeSet.java:29 FileTypeSet
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum FileTypeSet {
    /// Java `FileTypeSet.ANY_FILE`.
    AnyFile,
    /// Java `FileTypeSet.NO_FILE`.
    NoFile,
    /// Java `FileTypeSet.of(...)`.
    Of(Vec<FileType>),
}

impl FileTypeSet {
    // Java: com/google/devtools/build/lib/util/FileTypeSet.java:84 FileTypeSet.of
    // Java: com/google/devtools/build/lib/util/FileTypeSet.java:97 FileTypeSet.of
    pub fn of(file_types: Vec<FileType>) -> FileTypeSet {
        // No file types: `NO_FILE` (whose `toString` is "no files").
        if file_types.is_empty() {
            return FileTypeSet::NoFile;
        }
        // `ImmutableSet.copyOf(fileTypes)`: duplicates collapse, first occurrence first.
        let mut unique: Vec<FileType> = Vec::with_capacity(file_types.len());
        for file_type in file_types {
            if !unique.contains(&file_type) {
                unique.push(file_type);
            }
        }
        FileTypeSet::Of(unique)
    }

    // Java: com/google/devtools/build/lib/util/FileTypeSet.java:130 FileTypeSet.apply
    // Java: com/google/devtools/build/lib/util/FileTypeSet.java:111 FileTypeSet.matches
    pub fn apply(&self, filename: &str) -> bool {
        match self {
            FileTypeSet::AnyFile => true,
            FileTypeSet::NoFile => false,
            FileTypeSet::Of(types) => types.iter().any(|t| t.apply(filename)),
        }
    }
}
