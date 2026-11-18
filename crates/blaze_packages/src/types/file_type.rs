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
    // Java: com/google/devtools/build/lib/util/FileType.java FileType.of
    pub fn of(extensions: &[&str]) -> FileType {
        FileType {
            extensions: extensions.iter().map(|s| (*s).to_owned()).collect(),
        }
    }

    // Java: com/google/devtools/build/lib/util/FileType.java FileType.getExtensions
    pub fn get_extensions(&self) -> &[String] {
        &self.extensions
    }

    // Java: com/google/devtools/build/lib/util/FileType.java FileType.apply
    pub fn apply(&self, path: &str) -> bool {
        self.extensions
            .iter()
            .any(|ext| path.ends_with(ext.as_str()))
    }
}

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
    // Java: com/google/devtools/build/lib/util/FileTypeSet.java FileTypeSet.of
    pub fn of(file_types: Vec<FileType>) -> FileTypeSet {
        FileTypeSet::Of(file_types)
    }

    // Java: com/google/devtools/build/lib/util/FileTypeSet.java FileTypeSet.apply
    pub fn apply(&self, filename: &str) -> bool {
        match self {
            FileTypeSet::AnyFile => true,
            FileTypeSet::NoFile => false,
            FileTypeSet::Of(types) => types.iter().any(|t| t.apply(filename)),
        }
    }
}
