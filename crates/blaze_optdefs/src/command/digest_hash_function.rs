//! `DigestHashFunction.DigestFunctionConverter` from `com/google/devtools/build/lib/vfs/DigestHashFunction.java`.

use super::prelude::*;

// Java: com/google/devtools/build/lib/vfs/DigestHashFunction.java:131 DigestHashFunction.DigestFunctionConverter
/// Converts a hash function name (case-insensitively) to the registered `DigestHashFunction`.
#[derive(Clone, Copy, Debug, Default)]
pub struct DigestFunctionConverter;

impl DigestFunctionConverter {
    /// The typed conversion (`new DigestFunctionConverter().convert(value)`), shared with
    /// `BazelFileSystemModule.getFileSystem`.
    // Java: com/google/devtools/build/lib/vfs/DigestHashFunction.java:133 DigestHashFunction.DigestFunctionConverter.convert
    pub fn convert_name(input: &str) -> Result<DigestHashFunction, OptionsParsingException> {
        // Java iterates hashFunctionRegistry: SHA-1/SHA1 and SHA-256/SHA256 (DigestHashFunction),
        // BLAKE3 and GITSHA1 (vfs/bazel/BazelHashFunctions). The names are distinct ignoring
        // case, so the HashMap's iteration order does not matter.
        if let Some(f) = DigestHashFunction::for_name(input) {
            return Ok(f);
        }
        for name in ["SHA-1", "SHA1", "GITSHA1"] {
            if name.eq_ignore_ascii_case(input) {
                unported!(
                    "DigestHashFunction",
                    "{} (blaze_vfs has SHA-256 and BLAKE3 only)",
                    name
                );
            }
        }
        Err(OptionsParsingException::new("Not a valid hash function."))
    }
}

impl Converter for DigestFunctionConverter {
    // Java: com/google/devtools/build/lib/vfs/DigestHashFunction.java:133 DigestHashFunction.DigestFunctionConverter.convert
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        let input = non_null(input);
        let f = Self::convert_name(input)?;
        Ok(Some(Value::new(Foreign(f))))
    }
    fn get_type_description(&self) -> String {
        "hash function".into()
    }
}
