//! Tests for `RepoRecordedInput`.
//!
//! Java: src/test/java/com/google/devtools/build/lib/rules/repository/RepoRecordedInputTest.java,
//! plus round trips of marker-file lines recorded by the oracle (Bazel 9.3.2).

use std::sync::Arc;

use blaze_cmdline::RepositoryName;
use blaze_repo_rules::repository::repo_recorded_input::RepoCacheFriendlyPath;
use blaze_repo_rules::repository::repo_recorded_input::RepoRecordedInput;
use blaze_repo_rules::repository::repo_recorded_input::RepoRecordedInputWithValue;
use blaze_repo_rules::repository::repo_recorded_input::escape;
use blaze_repo_rules::repository::repo_recorded_input::file_value_to_marker_value;
use blaze_repo_rules::repository::repo_recorded_input::unescape;
use blaze_sky_loading::actions::FileContentsProxy;
use blaze_sky_loading::actions::FileStateValue;
use blaze_sky_loading::actions::FileValue;
use blaze_vfs::DigestHashFunction;
use blaze_vfs::FileSystemRef;
use blaze_vfs::Path;
use blaze_vfs::PathFragment;
use blaze_vfs::Root;
use blaze_vfs::RootedPath;
use blaze_vfs::UnixFileSystem;

fn parse(s: &str) -> RepoRecordedInputWithValue {
    RepoRecordedInputWithValue::parse(s).expect("parse")
}

fn assert_marker_file_escaping(test_case: Option<&str>) {
    let escaped = escape(test_case);
    assert_eq!(unescape(&escaped).as_deref(), test_case);
}

#[test]
fn test_marker_file_escaping() {
    assert_marker_file_escaping(None);
    assert_marker_file_escaping(Some("\\0"));
    assert_marker_file_escaping(Some("a\\0"));
    assert_marker_file_escaping(Some("a b"));
    assert_marker_file_escaping(Some("a b c"));
    assert_marker_file_escaping(Some("a \\b"));
    assert_marker_file_escaping(Some("a \\nb"));
    assert_marker_file_escaping(Some("a \\\\nb"));
    assert_marker_file_escaping(Some("a \\\nb"));
    assert_marker_file_escaping(Some("a \nb"));
}

#[test]
fn test_file_value_to_marker_value() {
    let tmp = tempfile::tempdir().unwrap();
    let fs: FileSystemRef = Arc::new(UnixFileSystem::new(DigestHashFunction::Sha256));
    let root_directory = Path::create_from_str(tmp.path().to_str().unwrap(), &fs);
    // Java: scratch.file("foo", "bar") writes the line "bar".
    std::fs::write(tmp.path().join("foo"), "bar\n").unwrap();
    let path = RootedPath::to_rooted_path(
        Root::from_path(root_directory.clone()),
        &PathFragment::create("foo"),
    );

    // Digest should be returned if the FileValue has it.
    let fv = FileValue::Regular(FileStateValue::RegularFileWithDigest {
        size: 3,
        digest: vec![1, 2, 3, 4],
    });
    assert_eq!(file_value_to_marker_value(&path, &fv).unwrap(), "01020304");

    // Digest should also be returned if the FileStateValue doesn't have it.
    let fv = FileValue::Regular(FileStateValue::RegularFileWithContentsProxy {
        size: 3,
        contents_proxy: FileContentsProxy::new(100, 100, 200),
    });
    let expected_digest = hex::encode(path.as_path().get_digest().unwrap());
    assert_eq!(
        file_value_to_marker_value(&path, &fv).unwrap(),
        expected_digest
    );
}

#[test]
fn test_split_into_batches() {
    assert!(RepoRecordedInputWithValue::split_into_batches(&[]).is_empty());
    assert_eq!(
        RepoRecordedInputWithValue::split_into_batches(&[
            parse("FILE:@@//foo:bar abc"),
            parse("FILE:@@//:baz cba"),
            parse("FILE:@@foo//:baz bac"),
            parse("ENV:KEY value"),
        ]),
        vec![
            vec![parse("FILE:@@//foo:bar abc"), parse("FILE:@@//:baz cba")],
            vec![parse("FILE:@@foo//:baz bac"), parse("ENV:KEY value")],
        ]
    );
}

// The tests below are not in RepoRecordedInputTest.java. Their inputs are lines of marker files
// written by the oracle (Bazel 9.3.2); a line must survive parse + toString unchanged.

const ORACLE_MARKER_LINES: &[&str] = &[
    "DIRENTS:/usr/lib/jvm/java-11-temurin-jdk \
     78ebf7cf46468e35497eea73f5d7194acad6c1791a110dfe220e53030616632a",
    "ENV:RULES_PYTHON_REPO_DEBUG \\0",
    "FILE:@@rules_cc+//cc/private/toolchain/BUILD.tpl \
     014ca82eb6f7ed97b19baa5567e144414c4fd2da086f2c761999243e684cd811",
    "FILE:/usr/bin/chmod a77565b748fd1867a5a5e693b6bcc890025bc55892a714204c660a3a1d9bd460",
    "FILE:/usr/lib/jvm/java-11-temurin-jdk DIR",
    "REPO_MAPPING:apple_support+,build_bazel_apple_support apple_support+",
    "REPO_MAPPING:rules_cc+,rules_cc rules_cc+",
];

#[test]
fn oracle_marker_lines_round_trip() {
    for line in ORACLE_MARKER_LINES {
        let parsed = parse(line);
        assert_eq!(parsed.to_string(), *line);
    }
}

#[test]
fn oracle_marker_lines_parse_to_expected_inputs() {
    assert_eq!(
        parse(ORACLE_MARKER_LINES[0]).input,
        RepoRecordedInput::Dirents(RepoCacheFriendlyPath::create_outside_workspace(
            PathFragment::create("/usr/lib/jvm/java-11-temurin-jdk")
        ))
    );
    let env = parse(ORACLE_MARKER_LINES[1]);
    assert_eq!(
        env.input,
        RepoRecordedInput::EnvVar("RULES_PYTHON_REPO_DEBUG".to_owned())
    );
    assert_eq!(env.value, None);
    assert_eq!(
        parse(ORACLE_MARKER_LINES[2]).input,
        RepoRecordedInput::File(RepoCacheFriendlyPath::create_inside_workspace(
            RepositoryName::create("rules_cc+").unwrap(),
            PathFragment::create("cc/private/toolchain/BUILD.tpl")
        ))
    );
    let mapping = parse(ORACLE_MARKER_LINES[5]);
    assert_eq!(
        mapping.input,
        RepoRecordedInput::RecordedRepoMapping {
            source_repo: RepositoryName::create("apple_support+").unwrap(),
            apparent_name: "build_bazel_apple_support".to_owned(),
        }
    );
    assert_eq!(mapping.value.as_deref(), Some("apple_support+"));
}

#[test]
fn malformed_lines_do_not_parse() {
    // Java: WithValue.parse returns Optional.empty() for a missing space, a leading space, an
    // unknown prefix and inputs whose parser fails.
    assert_eq!(RepoRecordedInputWithValue::parse("FILE:/foo"), None);
    assert_eq!(RepoRecordedInputWithValue::parse(" FILE:/foo x"), None);
    assert_eq!(RepoRecordedInputWithValue::parse("BOGUS:/foo x"), None);
    assert_eq!(RepoRecordedInputWithValue::parse("NOCOLON x"), None);
    assert_eq!(
        RepoRecordedInputWithValue::parse("REPO_MAPPING:foo x"),
        None
    );
    assert_eq!(
        RepoRecordedInputWithValue::parse("FILE:@@not/valid//foo x"),
        None
    );
    assert_eq!(
        RepoRecordedInput::parse("FILE"),
        RepoRecordedInput::parse_failure()
    );
}

#[test]
fn main_repo_path_to_string() {
    let path = RepoCacheFriendlyPath::create_inside_workspace(
        RepositoryName::create("").unwrap(),
        PathFragment::create("foo/bar.bzl"),
    );
    assert_eq!(path.to_string(), "@@//foo/bar.bzl");
    assert!(!path.in_external_repo());
    assert_eq!(
        RepoCacheFriendlyPath::parse("@@//foo/bar.bzl").unwrap(),
        path
    );
    // A single leading `@` is also accepted.
    assert_eq!(
        RepoCacheFriendlyPath::parse("@//foo/bar.bzl").unwrap(),
        path
    );
    assert_eq!(RepoCacheFriendlyPath::parse("//foo/bar.bzl").unwrap(), path);
}

#[test]
fn spaces_and_newlines_are_escaped_in_inputs_and_values() {
    let input = RepoRecordedInput::EnvVar("A B".to_owned());
    assert_eq!(input.to_string(), "ENV:A\\sB");
    let with_value = RepoRecordedInputWithValue::new(input.clone(), Some("x\ny z\\".to_owned()));
    assert_eq!(with_value.to_string(), "ENV:A\\sB x\\ny\\sz\\\\");
    assert_eq!(parse(&with_value.to_string()), with_value);
}

#[test]
fn describe_change_texts() {
    let env = RepoRecordedInput::EnvVar("FOO".to_owned());
    assert_eq!(
        env.describe_change(None, Some("x")),
        "environment variable FOO changed: <unset> -> 'x'"
    );
    assert_eq!(
        env.describe_change(Some("a"), None),
        "environment variable FOO changed: 'a' -> <unset>"
    );
    let mapping = RepoRecordedInput::RecordedRepoMapping {
        source_repo: RepositoryName::create("rules_cc+").unwrap(),
        apparent_name: "bazel_skylib".to_owned(),
    };
    assert_eq!(
        mapping.describe_change(Some("bazel_skylib+"), None),
        "canonical name for @bazel_skylib in @@rules_cc+ changed: bazel_skylib+ -> <doesn't exist>"
    );
    let file = RepoRecordedInput::File(RepoCacheFriendlyPath::create_outside_workspace(
        PathFragment::create("/usr/bin/chmod"),
    ));
    assert_eq!(
        file.describe_change(Some("a"), Some("b")),
        "file info or contents of /usr/bin/chmod changed"
    );
}

#[test]
#[ignore = "needs a Skyframe evaluator with FILE, DIRECTORY_LISTING, REPOSITORY_ENVIRONMENT_VARIABLE and REPOSITORY_MAPPING functions"]
fn is_any_value_outdated() {}
