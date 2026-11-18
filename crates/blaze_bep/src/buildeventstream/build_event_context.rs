//! Port of `BuildEventContext`, `PathConverter` and `ArtifactGroupNamer`.

use blaze_actions::Artifact;
use blaze_collect::nestedset::NestedSetNode;
use blaze_optdefs::misc::build_event_context::OutputGroupFileMode;
use blaze_optdefs::misc::build_event_protocol_options::BuildEventProtocolOptions;
use blaze_protos::build_event_stream::build_event_id::NamedSetOfFilesId;
use blaze_util::check_state;
use blaze_vfs::Path;

/// Interface for conversion of paths to URIs.
// Java: com/google/devtools/build/lib/buildeventstream/PathConverter.java:26 PathConverter
pub trait PathConverter: Send + Sync {
    /// Return the URI corresponding to the given path, if the path can be converted to a URI by
    /// this path converter; return `None` otherwise.
    // Java: com/google/devtools/build/lib/buildeventstream/PathConverter.java:41 PathConverter.apply
    fn apply(&self, path: &Path) -> Option<String>;
}

/// `PathConverter.NO_CONVERSION`: throws for every path.
// Java: com/google/devtools/build/lib/buildeventstream/PathConverter.java:28 PathConverter.NO_CONVERSION
pub struct NoConversion;

impl PathConverter for NoConversion {
    fn apply(&self, path: &Path) -> Option<String> {
        check_state!(
            false,
            "Can't convert '{}', as it has not been declared as a referenced artifact of a build event",
            path.get_path_string()
        );
        unreachable!()
    }
}

/// A path converter that converts a path to a file URI.
// Java: com/google/devtools/build/lib/buildeventstream/PathConverter.java:39 PathConverter.FileUriPathConverter
pub struct FileUriPathConverter;

impl PathConverter for FileUriPathConverter {
    // Java: com/google/devtools/build/lib/buildeventstream/PathConverter.java:41 FileUriPathConverter.apply
    fn apply(&self, path: &Path) -> Option<String> {
        Some(FileUriPathConverter::path_to_uri_string(
            path.get_path_string(),
        ))
    }
}

impl FileUriPathConverter {
    /// Returns the path encoded as an `URI`.
    ///
    /// This concrete implementation returns URIs with "file" as the scheme. For Example:
    /// - On Unix the path "/tmp/foo bar.txt" will be encoded as "file:///tmp/foo%20bar.txt".
    /// - On Windows the path "C:\Temp\Foo Bar.txt" will be encoded as
    ///   "file:///C:/Temp/Foo%20Bar.txt"
    ///
    /// Implementors extending this class for special filesystems will likely need to override
    /// this method.
    // Java: com/google/devtools/build/lib/buildeventstream/PathConverter.java:57 FileUriPathConverter.pathToUriString
    pub fn path_to_uri_string(path: &str) -> String {
        let mut path = path.to_string();
        if !path.starts_with('/') {
            path = format!("/{path}");
        }
        // new URI("file", /* authority= */ "", path, null, null).toString()
        format!("file://{}", java_uri_quote_path(&path))
    }
}

/// `java.net.URI.quote(path, L_PATH, H_PATH)` as used by the multi-argument URI constructors:
/// every character that is not legal in a path component (and is ASCII) is percent-encoded as
/// its UTF-8 bytes; non-ASCII characters that are not control or space characters ("other") are
/// kept.
// PORT: java.net.URI.quote / appendEscape / appendEncoded (JDK 21).
fn java_uri_quote_path(s: &str) -> String {
    fn legal_ascii(c: char) -> bool {
        // L_PATH = L_PCHAR | ";" | "/"; L_PCHAR = L_UNRESERVED | L_ESCAPED | ":@&=+$,"
        // L_UNRESERVED = alphanum | "-_.!~*'()"; "%" is never legal here (escapes are quoted
        // by the multi-argument constructors).
        c.is_ascii_alphanumeric() || "-_.!~*'();/:@&=+$,".contains(c)
    }
    let mut out = String::with_capacity(s.len());
    for c in s.chars() {
        if c.is_ascii() {
            if legal_ascii(c) {
                out.push(c);
            } else {
                out.push_str(&format!("%{:02X}", c as u32));
            }
        } else if c.is_whitespace() || c.is_control() {
            let mut buf = [0u8; 4];
            for b in c.encode_utf8(&mut buf).bytes() {
                out.push_str(&format!("%{b:02X}"));
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// The identity of a node of an artifact set, as `NestedSet.Node` is in Java: a branch node is
/// equal only to itself, a leaf node to any leaf of an equal element.
///
/// PORT: Java's streamer manipulates `NestedSet<?>` holding both `Artifact`s and expanded
/// artifacts. The Rust `NestedSet` is typed, so sets the streamer builds after expansion are
/// [`crate::runtime::named_artifact_group::ExpandedSet`]s, identified by their own address.
#[derive(Clone, PartialEq, Eq, Hash, Debug)]
pub enum ArtifactGroupNode {
    /// `NestedSet<Artifact>.toNode()`.
    Artifacts(NestedSetNode<Artifact>),
    /// A set built by the streamer (`NamedArtifactGroup.expandSet` or a split).
    Expanded(usize),
}

/// Interface for conversion of paths to URIs.
// Java: com/google/devtools/build/lib/buildeventstream/ArtifactGroupNamer.java:21 ArtifactGroupNamer
pub trait ArtifactGroupNamer: Send + Sync {
    /// Return the name of a NestedSet of artifacts, identified by its Node. A build event should
    /// only assume that this function is defined if the corresponding `NestedSet<Artifact>` is
    /// declared via the `EventReportingArtifacts` interface. On undefined positions, `None` is
    /// returned.
    // Java: com/google/devtools/build/lib/buildeventstream/ArtifactGroupNamer.java:28 ArtifactGroupNamer.apply
    fn apply(&self, node: &ArtifactGroupNode) -> Option<NamedSetOfFilesId>;
}

/// Interface for providing build events with the converters needed for computing the protobuf
/// representation.
// Java: com/google/devtools/build/lib/buildeventstream/BuildEventContext.java:21 BuildEventContext
pub trait BuildEventContext {
    /// Return the [`PathConverter`] to be used in order to obtain URIs for the file paths to be
    /// reported in the event.
    // Java: com/google/devtools/build/lib/buildeventstream/BuildEventContext.java:26 BuildEventContext.pathConverter
    fn path_converter(&self) -> &dyn PathConverter;

    /// Return the [`ArtifactGroupNamer`] that can be used to refer to a `NestedSet<Artifact>`
    /// declared via the `EventReportingArtifacts` interface.
    // Java: com/google/devtools/build/lib/buildeventstream/BuildEventContext.java:33 BuildEventContext.artifactGroupNamer
    fn artifact_group_namer(&self) -> &dyn ArtifactGroupNamer;

    /// Returns the options for the build event stream.
    // Java: com/google/devtools/build/lib/buildeventstream/BuildEventContext.java:38 BuildEventContext.getOptions
    fn get_options(&self) -> &BuildEventProtocolOptions;

    /// Return the `OutputGroupFileMode` to use for a given output group.
    // Java: com/google/devtools/build/lib/buildeventstream/BuildEventContext.java:48 BuildEventContext.getFileModeForOutputGroup
    fn get_file_mode_for_output_group(&self, _output_group: &str) -> OutputGroupFileMode {
        OutputGroupFileMode::NAMED_SET_OF_FILES_ONLY
    }
}
