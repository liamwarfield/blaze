//! Port of `runtime/commands/TargetPatternsHelper.java`.

use std::io::Read;
use std::sync::Arc;

use blaze_events::{EventBus, Postable};
use blaze_optdefs::command::build_request_options::BuildRequestOptions;
use blaze_protos::failure_details::{
    FailureDetail, TargetPatterns, failure_detail::Category, target_patterns,
};
use blaze_vfs::{Path, PathFragment};

use crate::command_environment::CommandEnvironment;
use crate::options_result::OptionsParsingResult;

/// An event which is posted when a user-specified input file is read.
// Java: com/google/devtools/build/lib/runtime/events/InputFileEvent.java:19 InputFileEvent
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct InputFileEvent {
    /// `type()`.
    pub type_: String,
    /// `size()`: the size of the file in bytes.
    pub size: i64,
}

impl InputFileEvent {
    // Java: com/google/devtools/build/lib/runtime/events/InputFileEvent.java:20 InputFileEvent.create
    pub fn create(type_: &str, size: i64) -> InputFileEvent {
        InputFileEvent {
            type_: type_.to_owned(),
            size,
        }
    }
}

impl Postable for InputFileEvent {}

/// Thrown when target patterns couldn't be read.
// Java: com/google/devtools/build/lib/runtime/commands/TargetPatternsHelper.java:88 TargetPatternsHelper.TargetPatternsHelperException
#[derive(Clone, Debug)]
pub struct TargetPatternsHelperException {
    message: String,
    detailed_code: target_patterns::Code,
}

impl std::fmt::Display for TargetPatternsHelperException {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for TargetPatternsHelperException {}

impl TargetPatternsHelperException {
    // Java: com/google/devtools/build/lib/runtime/commands/TargetPatternsHelper.java:91 TargetPatternsHelperException.<init>
    fn new(message: String, detailed_code: target_patterns::Code) -> TargetPatternsHelperException {
        TargetPatternsHelperException {
            message,
            detailed_code,
        }
    }

    /// `getMessage()`.
    pub fn get_message(&self) -> &str {
        &self.message
    }

    // Java: com/google/devtools/build/lib/runtime/commands/TargetPatternsHelper.java:96 TargetPatternsHelperException.getFailureDetail
    pub fn get_failure_detail(&self) -> FailureDetail {
        FailureDetail {
            message: self.message.clone(),
            category: Some(Category::TargetPatterns(TargetPatterns {
                code: self.detailed_code as i32,
            })),
        }
    }
}

/// Reads a list of target patterns, either from the command-line residue or by reading newline
/// delimited target patterns from the --target_pattern_file flag. If --target_pattern_file is
/// specified and options contain a residue, or if the file cannot be read, throws
/// [`TargetPatternsHelperException`].
// Java: com/google/devtools/build/lib/runtime/commands/TargetPatternsHelper.java:48 TargetPatternsHelper.readFrom
pub fn read_from(
    env: &CommandEnvironment,
    options: &OptionsParsingResult,
) -> Result<Vec<String>, TargetPatternsHelperException> {
    let event_bus: &dyn EventBus = &**env.get_event_bus();
    read_from_with(env.get_working_directory(), event_bus, options)
}

/// [`read_from`] over what it reads from the `CommandEnvironment`: `env.getWorkingDirectory()`
/// and `env.getEventBus()`.
// Java: com/google/devtools/build/lib/runtime/commands/TargetPatternsHelper.java:48 TargetPatternsHelper.readFrom
pub fn read_from_with(
    working_directory: &Path,
    event_bus: &dyn EventBus,
    options: &OptionsParsingResult,
) -> Result<Vec<String>, TargetPatternsHelperException> {
    let mut targets: Vec<String> = options.get_residue();
    let build_request_options: BuildRequestOptions = options
        .get_options::<BuildRequestOptions>()
        .unwrap_or_else(|| panic!("NullPointerException: BuildRequestOptions"));
    if !targets.is_empty() && !build_request_options.target_pattern_file.is_empty() {
        return Err(TargetPatternsHelperException::new(
            "Command-line target pattern and --target_pattern_file cannot both be specified"
                .to_owned(),
            target_patterns::Code::TargetPatternFileWithCommandLinePattern,
        ));
    } else if !build_request_options.target_pattern_file.is_empty() {
        // Works for absolute or relative file.
        let residue_path = working_directory.get_relative(&PathFragment::create(
            &build_request_options.target_pattern_file,
        ));
        let read = || -> std::io::Result<Vec<u8>> {
            event_bus.post(Arc::new(InputFileEvent::create(
                /* type= */ "target_pattern_file",
                residue_path.get_file_size()?,
            )));
            // FileSystemUtils.readLines(residuePath, ISO_8859_1)
            let mut bytes = Vec::new();
            residue_path.get_input_stream()?.read_to_end(&mut bytes)?;
            Ok(bytes)
        };
        match read() {
            Ok(bytes) => {
                // new String(bytes, ISO_8859_1): the internal string form (Latin-1 characters
                // holding the file's bytes). PORT: StringEncoding identity for UTF-8 content. A
                // Rust String cannot hold other bytes, and replacing them would change the
                // patterns silently.
                let content = match String::from_utf8(bytes) {
                    Ok(content) => content,
                    Err(e) => blaze_util::unported!(
                        "TargetPatternsHelper.readFrom",
                        "--target_pattern_file {} is not valid UTF-8 ({})",
                        residue_path.get_path_string(),
                        e.utf8_error()
                    ),
                };
                targets = read_lines(&content)
                    .into_iter()
                    // TARGET_PATTERN_SPLITTER.splitToList(s).get(0)
                    .map(|s| s.split('#').next().unwrap_or(""))
                    // String::trim: removes the characters up to U+0020 from both ends.
                    .map(|s| s.trim_matches(|c: char| c <= ' '))
                    .filter(|s| !s.is_empty())
                    .map(str::to_owned)
                    .collect();
            }
            Err(e) => {
                return Err(TargetPatternsHelperException::new(
                    format!(
                        "I/O error reading from {}: {}",
                        residue_path.get_path_string(),
                        e
                    ),
                    target_patterns::Code::TargetPatternFileReadFailure,
                ));
            }
        }
    } else {
        // ProjectFileSupport.getTargets(env.getRuntime().getProjectFileProvider(), options):
        // Bazel has no project file provider, so the targets are the residue.
        // Java: com/google/devtools/build/lib/runtime/ProjectFileSupport.java:100 ProjectFileSupport.getTargets
        targets = options.get_residue();
    }
    Ok(targets)
}

/// `FileSystemUtils.readLines` (`CharStreams.readLines`): the lines of `content`, without their
/// terminators (`\n`, `\r` or `\r\n`); a last line needs no terminator.
// Java: com/google/devtools/build/lib/vfs/FileSystemUtils.java:795 FileSystemUtils.readLines
fn read_lines(content: &str) -> Vec<&str> {
    let mut lines = Vec::new();
    let bytes = content.as_bytes();
    let mut start = 0;
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'\n' => {
                lines.push(&content[start..i]);
                i += 1;
                start = i;
            }
            b'\r' => {
                lines.push(&content[start..i]);
                i += 1;
                if i < bytes.len() && bytes[i] == b'\n' {
                    i += 1;
                }
                start = i;
            }
            _ => i += 1,
        }
    }
    if start < bytes.len() {
        lines.push(&content[start..]);
    }
    lines
}

#[cfg(test)]
mod tests {
    // Java: src/test/java/com/google/devtools/build/lib/runtime/commands/TargetPatternsHelperTest.java
    use blaze_options::{OptionsClass, OptionsParser};
    use parking_lot::Mutex;

    use super::*;

    #[derive(Default)]
    struct MockEventBus {
        input_file_events: Mutex<Vec<InputFileEvent>>,
    }

    impl EventBus for MockEventBus {
        fn post(&self, event: Arc<dyn Postable>) {
            let any: &dyn std::any::Any = &*event;
            self.input_file_events.lock().push(
                any.downcast_ref::<InputFileEvent>()
                    .expect("an InputFileEvent")
                    .clone(),
            );
        }
    }

    struct Fixture {
        _tmp: tempfile::TempDir,
        working_directory: Path,
        options: OptionsParser,
        mock_event_bus: MockEventBus,
    }

    fn set_up() -> Fixture {
        let options = OptionsParser::builder()
            .options_classes(&[BuildRequestOptions::class()])
            .build()
            .unwrap_or_else(|e| e.throw());
        let tmp = tempfile::tempdir().unwrap();
        let fs = blaze_vfs::UnixFileSystem::create(blaze_vfs::DigestHashFunction::Sha256);
        std::fs::create_dir(tmp.path().join("wd")).unwrap();
        let working_directory = Path::create_from_str(tmp.path().join("wd").to_str().unwrap(), &fs);
        Fixture {
            _tmp: tmp,
            working_directory,
            options,
            mock_event_bus: MockEventBus::default(),
        }
    }

    /// `options.parse("--target_pattern_file=patterns.txt")`.
    fn parse_target_pattern_file(f: &mut Fixture) {
        f.options
            .parse(&["--target_pattern_file=patterns.txt"])
            .unwrap_or_else(|e| panic!("{}", e.get_message()));
    }

    fn read(f: &Fixture) -> Result<Vec<String>, TargetPatternsHelperException> {
        read_from_with(&f.working_directory, &f.mock_event_bus, &f.options)
    }

    #[test]
    fn test_empty() {
        // tests when no residue and no --target_pattern_file are set
        let f = set_up();
        assert!(read(&f).unwrap().is_empty());
    }

    #[test]
    fn test_target_pattern_file() {
        let mut f = set_up();
        let content = "//some/...\n//patterns";
        std::fs::write(
            f.working_directory
                .get_relative_str("patterns.txt")
                .get_path_string(),
            content,
        )
        .unwrap();
        parse_target_pattern_file(&mut f);

        assert_eq!(
            read(&f).unwrap(),
            vec!["//some/...".to_owned(), "//patterns".to_owned()]
        );
        assert_eq!(
            *f.mock_event_bus.input_file_events.lock(),
            vec![InputFileEvent::create(
                /* type= */ "target_pattern_file",
                content.len() as i64
            )]
        );
    }

    #[test]
    fn test_no_target_pattern_file() {
        let mut f = set_up();
        let patterns = vec!["//some/...".to_owned(), "//patterns".to_owned()];
        f.options.set_residue(patterns.clone(), Vec::new());

        assert_eq!(read(&f).unwrap(), patterns);
    }

    #[test]
    fn test_specify_pattern_and_file_throws() {
        let mut f = set_up();
        parse_target_pattern_file(&mut f);
        f.options
            .set_residue(vec!["//some:pattern".to_owned()], Vec::new());

        let expected = read(&f).unwrap_err();

        let message =
            "Command-line target pattern and --target_pattern_file cannot both be specified";
        assert_eq!(expected.get_message(), message);
        assert_eq!(
            expected.get_failure_detail(),
            FailureDetail {
                message: message.to_owned(),
                category: Some(Category::TargetPatterns(TargetPatterns {
                    code: target_patterns::Code::TargetPatternFileWithCommandLinePattern as i32,
                })),
            }
        );
    }

    #[test]
    fn test_specify_non_existing_file_throws() {
        let mut f = set_up();
        parse_target_pattern_file(&mut f);

        let expected = read(&f).unwrap_err();

        // "I/O error reading from .*patterns.txt.*\\(No such file or directory\\)"
        let message = expected.get_message();
        assert!(message.starts_with("I/O error reading from "), "{message}");
        let rest = &message["I/O error reading from ".len()..];
        let at = rest.find("patterns.txt").expect(message);
        assert!(
            rest[at..].contains("(No such file or directory)"),
            "{message}"
        );
        assert_eq!(expected.get_failure_detail().message, message);
        assert_eq!(
            expected.get_failure_detail().category,
            Some(Category::TargetPatterns(TargetPatterns {
                code: target_patterns::Code::TargetPatternFileReadFailure as i32,
            }))
        );
    }

    // The comment and blank-line handling of readFrom (TARGET_PATTERN_SPLITTER, String::trim,
    // the empty-line filter), which the Java tests do not exercise: values from the Java
    // expression `Splitter.on('#').splitToList(s).get(0).trim()` applied by hand.
    #[test]
    fn target_pattern_file_comments_and_blank_lines() {
        let mut f = set_up();
        std::fs::write(
            f.working_directory
                .get_relative_str("patterns.txt")
                .get_path_string(),
            "# a comment\n  //a:b  # trailing\r\n\n\t//c/...\r//d#x#y\n",
        )
        .unwrap();
        parse_target_pattern_file(&mut f);

        assert_eq!(
            read(&f).unwrap(),
            vec!["//a:b".to_owned(), "//c/...".to_owned(), "//d".to_owned()]
        );
    }
}
