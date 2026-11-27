//! Port of `starlarkdocextract/ExtractionException.java`.

/// An exception indicating that the module's API documentation could not be extracted.
// Java: com/google/devtools/build/lib/starlarkdocextract/ExtractionException.java:25 ExtractionException
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{message}")]
pub struct ExtractionException {
    pub message: String,
}

impl ExtractionException {
    // Java: com/google/devtools/build/lib/starlarkdocextract/ExtractionException.java:26 ExtractionException.<init>
    pub fn new(message: impl Into<String>) -> Self {
        ExtractionException {
            message: message.into(),
        }
    }

    /// `new ExtractionException(module, message)`: the message prefixed with the module's file
    /// name (`BazelModuleContext.filename()`), when the module has a `BazelModuleContext`.
    // Java: com/google/devtools/build/lib/starlarkdocextract/ExtractionException.java:30 ExtractionException.<init>
    // Java: com/google/devtools/build/lib/starlarkdocextract/ExtractionException.java:42 ExtractionException.prefixWithModuleFilename
    pub fn in_module(module_filename: Option<&str>, message: impl Into<String>) -> Self {
        let message = message.into();
        match module_filename {
            None => ExtractionException { message },
            Some(filename) => ExtractionException {
                message: format!("in {filename}: {message}"),
            },
        }
    }
}
