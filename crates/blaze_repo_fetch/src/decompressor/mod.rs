//! Port of `com.google.devtools.build.lib.bazel.repository.decompressor`: archive extraction
//! (`DecompressorValue` and its decompressors) and `PatchUtil`.

pub mod ar;
pub mod compressed;
pub mod compressed_tar;
pub mod patch_util;
pub mod sevenz;
pub mod strip_prefixed_path;
mod tar_checksum;
pub mod zip;

pub use strip_prefixed_path::StripPrefixedPath;

use indexmap::IndexMap;

use blaze_vfs::{Path, PathFragment};

use crate::errors::{
    DecompressorError, IoException, IoExceptionKind, RepositoryFunctionException, Transience,
};

/// Description of an archive to be decompressed.
// Java: com/google/devtools/build/lib/bazel/repository/decompressor/DecompressorDescriptor.java:31 DecompressorDescriptor
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct DecompressorDescriptor {
    /// The context in which this decompression is happening. Should only be used for reporting.
    pub context: String,
    pub archive_path: Path,
    pub destination_path: Path,
    pub prefix: Option<String>,
    pub strip_components: i32,
    pub rename_files: IndexMap<String, String>,
}

/// Builder for [`DecompressorDescriptor`].
// Java: com/google/devtools/build/lib/bazel/repository/decompressor/DecompressorDescriptor.java:54 DecompressorDescriptor.Builder
#[derive(Clone, Debug, Default)]
pub struct DecompressorDescriptorBuilder {
    context: String,
    archive_path: Option<Path>,
    destination_path: Option<Path>,
    prefix: Option<String>,
    strip_components: i32,
    rename_files: IndexMap<String, String>,
}

impl DecompressorDescriptor {
    // Java: com/google/devtools/build/lib/bazel/repository/decompressor/DecompressorDescriptor.java:46 DecompressorDescriptor.builder
    pub fn builder() -> DecompressorDescriptorBuilder {
        DecompressorDescriptorBuilder::default()
    }
}

impl DecompressorDescriptorBuilder {
    pub fn set_context(mut self, context: &str) -> Self {
        self.context = context.to_string();
        self
    }

    pub fn set_archive_path(mut self, p: Path) -> Self {
        self.archive_path = Some(p);
        self
    }

    pub fn set_destination_path(mut self, p: Path) -> Self {
        self.destination_path = Some(p);
        self
    }

    pub fn set_prefix(mut self, prefix: &str) -> Self {
        self.prefix = Some(prefix.to_string());
        self
    }

    pub fn set_strip_components(mut self, n: i32) -> Self {
        self.strip_components = n;
        self
    }

    pub fn set_rename_files(mut self, m: IndexMap<String, String>) -> Self {
        self.rename_files = m;
        self
    }

    /// Panics like Java's `IllegalArgumentException` / `NullPointerException` on bad input.
    // Java: com/google/devtools/build/lib/bazel/repository/decompressor/DecompressorDescriptor.java:71 Builder.build
    pub fn build(self) -> DecompressorDescriptor {
        let d = DecompressorDescriptor {
            context: self.context,
            archive_path: self.archive_path.expect("archivePath"),
            destination_path: self.destination_path.expect("destinationPath"),
            prefix: self.prefix,
            strip_components: self.strip_components,
            rename_files: self.rename_files,
        };
        if d.strip_components < 0 {
            panic!("IllegalArgumentException: 'strip_components' must be non-negative");
        }
        if d.strip_components != 0 && d.prefix.as_ref().is_some_and(|p| !p.is_empty()) {
            panic!(
                "IllegalArgumentException: Only one of 'strip_prefix' or 'strip_components' can be set"
            );
        }
        d
    }
}

/// The decompressors (Java `DecompressorValue.Decompressor` implementations).
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Decompressor {
    ZipDecompressor,
    TarFunction,
    TarGzFunction,
    GzFunction,
    TarXzFunction,
    XzFunction,
    TarZstFunction,
    ZstFunction,
    TarBz2Function,
    Bz2Function,
    ArFunction,
    SevenZDecompressor,
}

/// List of supported compressor format file extensions with their corresponding Decompressor
/// instance. The order here is intentional and is the order in which a decompressor is searched
/// for.
// Java: com/google/devtools/build/lib/bazel/repository/decompressor/DecompressorValue.java:84 DecompressorValue.supportedFormats
const SUPPORTED_FORMATS: &[(&[&str], Decompressor)] = &[
    (
        &["zip", "jar", "war", "aar", "nupkg", "whl"],
        Decompressor::ZipDecompressor,
    ),
    (&["tar"], Decompressor::TarFunction),
    (&["tar.gz", "tgz"], Decompressor::TarGzFunction),
    (&["gz"], Decompressor::GzFunction),
    (&["tar.xz", "txz"], Decompressor::TarXzFunction),
    (&["xz"], Decompressor::XzFunction),
    (&["tar.zst", "tzst"], Decompressor::TarZstFunction),
    (&["zst"], Decompressor::ZstFunction),
    (&["tar.bz2", "tbz"], Decompressor::TarBz2Function),
    (&["bz2"], Decompressor::Bz2Function),
    (&["ar", "deb"], Decompressor::ArFunction),
    (&["7z"], Decompressor::SevenZDecompressor),
];

/// Human-readable list of supported extensions, e.g. `.zip, .jar, ... or .7z`.
// Java: com/google/devtools/build/lib/bazel/repository/decompressor/DecompressorValue.java:119 DecompressorValue.readableSupportedFormats
pub fn readable_supported_formats(prefix: &str, suffix: &str, conjunction: &str) -> String {
    let all = all_supported_extensions(prefix, suffix);
    let comma_separated = all[..all.len() - 1].join(", ");
    format!("{} {} {}", comma_separated, conjunction, all[all.len() - 1])
}

// Java: com/google/devtools/build/lib/bazel/repository/decompressor/DecompressorValue.java:128 DecompressorValue.allSupportedExtensions
pub fn all_supported_extensions(prefix: &str, suffix: &str) -> Vec<String> {
    SUPPORTED_FORMATS
        .iter()
        .flat_map(|(exts, _)| exts.iter())
        .map(|t| format!("{}{}{}", prefix, t, suffix))
        .collect()
}

/// `CouldNotFindPrefixException`'s message.
// Java: com/google/devtools/build/lib/bazel/repository/decompressor/DecompressorValue.java:147 CouldNotFindPrefixException.prepareErrorMessage
pub fn could_not_find_prefix_exception(prefix: &str, available_prefixes: &[String]) -> IoException {
    let error = format!(
        "Prefix \"{}\" was given, but not found in the archive. ",
        prefix
    );
    let mut suggestion = "Here are possible prefixes for this archive: ".to_string();
    let mut suggestion_body = String::new();
    if available_prefixes.is_empty() {
        suggestion = "We could not find any directory in this archive (maybe there is no need for `strip_prefix`?)".to_string();
    } else {
        // Add a list of possible suggestion wrapped with `"` and separated by `, `.
        suggestion_body = format!("\"{}\".", available_prefixes.join("\", \""));
    }
    IoException::of(
        IoExceptionKind::CouldNotFindPrefixException,
        format!("{}{}{}", error, suggestion, suggestion_body),
    )
}

// Java: com/google/devtools/build/lib/bazel/repository/decompressor/DecompressorValue.java:164 CouldNotFindPrefixException.maybeMakePrefixSuggestion
pub fn maybe_make_prefix_suggestion(path_fragment: &PathFragment) -> Option<String> {
    if !path_fragment.is_multi_segment() {
        return None;
    }
    Some(path_fragment.get_segment(0).to_string())
}

/// Collects prefix suggestions in a Java `HashSet<String>` and returns them in its order.
pub(crate) fn hash_set_prefixes(prefixes: Vec<String>) -> Vec<String> {
    crate::java_hash::java_hash_set_order(prefixes)
}

impl Decompressor {
    // Java: com/google/devtools/build/lib/bazel/repository/decompressor/DecompressorValue.java:171 Decompressor.decompress
    pub fn decompress(
        self,
        descriptor: &DecompressorDescriptor,
    ) -> Result<Path, DecompressorError> {
        match self {
            Decompressor::ZipDecompressor => zip::decompress(descriptor),
            Decompressor::TarFunction
            | Decompressor::TarGzFunction
            | Decompressor::TarXzFunction
            | Decompressor::TarZstFunction
            | Decompressor::TarBz2Function => compressed_tar::decompress(self, descriptor),
            Decompressor::GzFunction
            | Decompressor::XzFunction
            | Decompressor::ZstFunction
            | Decompressor::Bz2Function => compressed::decompress(self, descriptor),
            Decompressor::ArFunction => ar::decompress(descriptor),
            Decompressor::SevenZDecompressor => sevenz::decompress(descriptor),
        }
    }
}

/// Returns the decompressor for the archive's file name extension.
// Java: com/google/devtools/build/lib/bazel/repository/decompressor/DecompressorValue.java:193 DecompressorValue.getDecompressor
pub fn get_decompressor_for_path(
    archive_path: &Path,
) -> Result<Decompressor, RepositoryFunctionException> {
    let base_name = archive_path.get_base_name();
    // Return the corresponding decompressor if the archive's basename ends with a matching
    // extension. Eg. If the file ends in .tar.gz or .tgz, use the TarGzFunction decompressor.
    for (exts, decompressor) in SUPPORTED_FORMATS {
        if exts.iter().any(|t| base_name.ends_with(&format!(".{}", t))) {
            return Ok(*decompressor);
        }
    }
    Err(RepositoryFunctionException::eval(
        format!(
            "Expected a file with a {} suffix (got {})",
            readable_supported_formats(".", "", "or"),
            archive_path
        ),
        Transience::PERSISTENT,
    ))
}

/// Returns a decompressor based on the file type extension (without a leading dot).
// Java: com/google/devtools/build/lib/bazel/repository/decompressor/DecompressorValue.java:223 DecompressorValue.getDecompressor
pub fn get_decompressor_for_type(type_: &str) -> Result<Decompressor, RepositoryFunctionException> {
    for (exts, decompressor) in SUPPORTED_FORMATS {
        if exts.contains(&type_) {
            return Ok(*decompressor);
        }
    }
    Err(RepositoryFunctionException::eval(
        format!(
            "No decompressor found for type {}. Available types are: {}",
            type_,
            readable_supported_formats("", "", "or")
        ),
        Transience::PERSISTENT,
    ))
}

/// Decompresses an archive according to `descriptor`, using `force_decompressor_type` (e.g.
/// "zip") if given, else the archive's file extension.
// Java: com/google/devtools/build/lib/bazel/repository/decompressor/DecompressorValue.java:252 DecompressorValue.decompress
pub fn decompress(
    descriptor: &DecompressorDescriptor,
    force_decompressor_type: Option<&str>,
) -> Result<Path, RepositoryFunctionException> {
    let decompressor = match force_decompressor_type {
        Some(t) => get_decompressor_for_type(t)?,
        None => get_decompressor_for_path(&descriptor.archive_path)?,
    };
    match decompressor.decompress(descriptor) {
        Ok(p) => Ok(p),
        Err(DecompressorError::Repository(e)) => Err(e),
        Err(DecompressorError::Io(e)) => {
            let destination_directory = descriptor.archive_path.get_parent_directory();
            Err(RepositoryFunctionException::io(
                IoException::new(format!(
                    "Error extracting {} to {}: {}",
                    descriptor.archive_path,
                    destination_directory
                        .map(|p| p.to_string())
                        .unwrap_or_else(|| "null".to_string()),
                    e.get_message()
                )),
                Transience::TRANSIENT,
            ))
        }
    }
}

/// Shared helper: `entry.getMode() | 0400` chmod and mtime setting for an extracted file.
pub(crate) fn finish_file(
    path: &Path,
    mode: u32,
    mtime_millis: Option<i64>,
) -> Result<(), IoException> {
    // Ensure that all files are at least user-readable. Some archives contain files that
    // are not, but many other tools are working around this and thus mask these issues.
    path.chmod(mode | 0o400)?;
    if let Some(t) = mtime_millis {
        path.set_last_modified_time(t)?;
    }
    Ok(())
}
