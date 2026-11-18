//! `DigestHashFunction.DigestFunctionConverter` from `com/google/devtools/build/lib/vfs/DigestHashFunction.java`.

use super::prelude::*;

// Java: com/google/devtools/build/lib/vfs/DigestHashFunction.java:131 DigestHashFunction.DigestFunctionConverter
/// Converts a hash function name (case-insensitively) to the registered `DigestHashFunction`.
#[derive(Clone, Copy, Debug, Default)]
pub struct DigestFunctionConverter;

impl Converter for DigestFunctionConverter {
    // Java: com/google/devtools/build/lib/vfs/DigestHashFunction.java:133 DigestHashFunction.DigestFunctionConverter.convert
    fn convert(&self, input: Option<&str>, _: Option<&ConversionContext>) -> ConvResult {
        let input = non_null(input);
        // Java iterates hashFunctionRegistry: SHA-1/SHA1 and SHA-256/SHA256 (DigestHashFunction),
        // BLAKE3 and GITSHA1 (vfs/bazel/BazelHashFunctions).
        for name in ["SHA-256", "SHA256"] {
            if name.eq_ignore_ascii_case(input) {
                return Ok(Some(Value::new(Foreign(DigestHashFunction::Sha256))));
            }
        }
        for name in ["SHA-1", "SHA1", "BLAKE3", "GITSHA1"] {
            if name.eq_ignore_ascii_case(input) {
                unported!(
                    "DigestHashFunction",
                    "{} (blaze_vfs only has SHA-256)",
                    name
                );
            }
        }
        Err(OptionsParsingException::new("Not a valid hash function."))
    }
    fn get_type_description(&self) -> String {
        "hash function".into()
    }
}
