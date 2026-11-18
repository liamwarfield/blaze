//! Port of `packages/AttributeValueSource.java`.

use blaze_cmdline::EvalException;

// Java: com/google/devtools/build/lib/packages/AttributeValueSource.java:30 AttributeValueSource.STARLARK_PREFIX
const STARLARK_PREFIX: &str = "_";

/// How an attribute's value is determined; affects its native name prefix (Java
/// `AttributeValueSource`).
// Java: com/google/devtools/build/lib/packages/AttributeValueSource.java:23 AttributeValueSource
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AttributeValueSource {
    NativeComputedDefault,
    ComputedDefault,
    LateBound,
    Materializer,
    Direct,
}

impl AttributeValueSource {
    // Java: com/google/devtools/build/lib/packages/AttributeValueSource.java:24 AttributeValueSource.nativePrefix
    fn native_prefix(self) -> &'static str {
        match self {
            AttributeValueSource::NativeComputedDefault => "$",
            AttributeValueSource::ComputedDefault => "$",
            AttributeValueSource::LateBound => ":",
            AttributeValueSource::Materializer => ":",
            AttributeValueSource::Direct => "$",
        }
    }

    // Java: com/google/devtools/build/lib/packages/AttributeValueSource.java:24 AttributeValueSource.mustHaveStarlarkPrefix
    fn must_have_starlark_prefix(self) -> bool {
        match self {
            AttributeValueSource::NativeComputedDefault => false,
            AttributeValueSource::ComputedDefault => true,
            AttributeValueSource::LateBound => true,
            AttributeValueSource::Materializer => true,
            AttributeValueSource::Direct => false,
        }
    }

    /// Throws if the given Starlark name is not valid for this type.
    // Java: com/google/devtools/build/lib/packages/AttributeValueSource.java:48 AttributeValueSource.validateStarlarkName
    pub fn validate_starlark_name(self, attr_starlark_name: &str) -> Result<(), EvalException> {
        if attr_starlark_name.is_empty() {
            return Err(EvalException::new("Attribute name must not be empty."));
        }
        if self.must_have_starlark_prefix() && !attr_starlark_name.starts_with(STARLARK_PREFIX) {
            return Err(EvalException::new(format!(
                "When an attribute value is a function, the attribute must be private (i.e. start \
                 with '{STARLARK_PREFIX}'). Found '{attr_starlark_name}'"
            )));
        }
        Ok(())
    }

    /// Converts a Starlark attribute name to the native name (`_foo` to `$foo` / `:foo`).
    // Java: com/google/devtools/build/lib/packages/AttributeValueSource.java:65 AttributeValueSource.convertToNativeName
    pub fn convert_to_native_name(self, attr_starlark_name: &str) -> Result<String, EvalException> {
        self.validate_starlark_name(attr_starlark_name)?;
        // No need to check for mustHaveStarlarkPrefix since this was already done in
        // validateStarlarkName().
        Ok(match attr_starlark_name.strip_prefix(STARLARK_PREFIX) {
            Some(rest) => format!("{}{}", self.native_prefix(), rest),
            None => attr_starlark_name.to_owned(),
        })
    }
}
