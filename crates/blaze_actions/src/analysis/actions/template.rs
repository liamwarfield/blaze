//! Port of `lib/analysis/actions/Template.java`.

use std::fmt;
use std::io::{self, Read};

use crate::actions::artifact::Artifact;

/// A template that contains text content, or alternatively throws an `IOException`.
// Java: com/google/devtools/build/lib/analysis/actions/Template.java:31 Template
#[derive(Clone, Debug)]
pub enum Template {
    /// A resource that failed to load.
    // Java: com/google/devtools/build/lib/analysis/actions/Template.java:55 Template.ErrorTemplate
    Error {
        /// The `IOException`'s message.
        message: String,
        template_name: String,
    },
    // Java: com/google/devtools/build/lib/analysis/actions/Template.java:77 Template.StringTemplate
    String(String),
    // Java: com/google/devtools/build/lib/analysis/actions/Template.java:95 Template.ArtifactTemplate
    Artifact(Artifact),
}

impl Template {
    /// Returns the text content of the template.
    ///
    /// PORT: Java takes an `ArtifactPathResolver`; every analysis-time caller passes
    /// `ArtifactPathResolver.IDENTITY` (the artifact's own path), which is what this does. The
    /// execution-time resolver belongs to wave W6.
    // Java: com/google/devtools/build/lib/analysis/actions/Template.java:37 Template.getContent
    pub fn get_content(&self) -> io::Result<String> {
        match self {
            // Java: com/google/devtools/build/lib/analysis/actions/Template.java:65 ErrorTemplate.getContent
            Template::Error {
                message,
                template_name,
            } => Err(io::Error::other(format!(
                "failed to load resource file '{}' due to I/O error: {}",
                template_name, message
            ))),
            // Java: com/google/devtools/build/lib/analysis/actions/Template.java:85 StringTemplate.getContent
            Template::String(template_text) => Ok(template_text.clone()),
            // Java: com/google/devtools/build/lib/analysis/actions/Template.java:103 ArtifactTemplate.getContent
            Template::Artifact(template_artifact) => {
                let template_path = template_artifact.get_path();
                // Bazel's internal encoding for strings is raw bytes as Latin-1
                let read = || -> io::Result<Vec<u8>> {
                    let mut bytes = Vec::new();
                    template_path.get_input_stream()?.read_to_end(&mut bytes)?;
                    Ok(bytes)
                };
                match read() {
                    // Java: `FileSystemUtils.readContent(templatePath, ISO_8859_1)`. Until the
                    // decoding switch (`string_encoding::STRINGS_ARE_INTERNAL`) a template that
                    // is not valid UTF-8 has no string.
                    Ok(bytes) => {
                        match blaze_util::string_encoding::internal_string_from_bytes(&bytes) {
                            Ok(s) => Ok(s.into_owned()),
                            Err(_) => blaze_util::unported!(
                                "ArtifactTemplate.getContent",
                                "template '{}' is not valid UTF-8",
                                template_path.get_path_string()
                            ),
                        }
                    }
                    Err(e) => Err(io::Error::new(
                        e.kind(),
                        format!(
                            "failed to load template file '{}' due to I/O error: {}",
                            template_path.get_path_string(),
                            e
                        ),
                    )),
                }
            }
        }
    }

    // Java: com/google/devtools/build/lib/analysis/actions/Template.java:40 Template.getTemplateArtifact
    pub fn get_template_artifact(&self) -> Option<&Artifact> {
        match self {
            // Java: com/google/devtools/build/lib/analysis/actions/Template.java:125 ArtifactTemplate.getTemplateArtifact
            Template::Artifact(a) => Some(a),
            _ => None,
        }
    }

    /// Returns a string that is used for the action key. This must change if the getContent
    /// method returns something different, but is not allowed to throw an exception.
    // Java: com/google/devtools/build/lib/analysis/actions/Template.java:48 Template.getKey
    pub fn get_key(&self) -> String {
        match self {
            // Java: com/google/devtools/build/lib/analysis/actions/Template.java:72 ErrorTemplate.getKey
            Template::Error { message, .. } => format!("ERROR: {}", message),
            // Java: com/google/devtools/build/lib/analysis/actions/Template.java:90 StringTemplate.getKey
            Template::String(template_text) => template_text.clone(),
            // Java: com/google/devtools/build/lib/analysis/actions/Template.java:119 ArtifactTemplate.getKey
            // This isn't strictly necessary, because the action inputs are automatically
            // considered.
            Template::Artifact(a) => format!("ARTIFACT: {}", a.get_exec_path_string()),
        }
    }

    /// Loads a template from the given resource. If the resource cannot be loaded, the returned
    /// template throws an `IOException` when `get_content` is called. This makes it safe to use
    /// this method in a constant initializer.
    ///
    /// PORT: Java loads the resource relative to a class (`ResourceFileLoader.loadResource`). Rust
    /// has no class resources: the caller loads the resource (usually `include_str!`) and passes
    /// the result; `Err` is the `IOException` message.
    // Java: com/google/devtools/build/lib/analysis/actions/Template.java:135 Template.forResource
    pub fn for_resource(content: Result<String, String>, template_name: &str) -> Template {
        match content {
            Ok(content) => Template::for_string(content),
            Err(message) => Template::Error {
                message,
                template_name: template_name.to_string(),
            },
        }
    }

    /// Returns a template for the given text string.
    // Java: com/google/devtools/build/lib/analysis/actions/Template.java:145 Template.forString
    pub fn for_string(template_text: impl Into<String>) -> Template {
        Template::String(template_text.into())
    }

    /// Returns a template that loads the given artifact. It is important that the artifact is also
    /// an input for the action, or this won't work. Therefore you should use the corresponding
    /// `TemplateExpansionAction` constructor.
    // Java: com/google/devtools/build/lib/analysis/actions/Template.java:155 Template.forArtifact
    pub fn for_artifact(template_artifact: Artifact) -> Template {
        Template::Artifact(template_artifact)
    }
}

// Java: com/google/devtools/build/lib/analysis/actions/Template.java:51 Template.toString
impl fmt::Display for Template {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.get_key())
    }
}
