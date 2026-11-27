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
        // BLAKE3 and GITSHA1 (vfs/bazel/BazelHashFunctions). No two names are equal ignoring
        // case, so the map's order does not matter.
        match DigestHashFunction::from_name(input) {
            Some(f) => Ok(Some(Value::new(Foreign(f)))),
            None => Err(OptionsParsingException::new("Not a valid hash function.")),
        }
    }
    fn get_type_description(&self) -> String {
        "hash function".into()
    }
}
