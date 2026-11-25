//! Port of `src/test/java/com/google/devtools/build/lib/skyframe/config/PlatformMappingFunctionParserTest.java`.
//!
//! The Java test evaluates `PlatformMappingFunction.parse` inside a Skyframe function of its own
//! (PARSED_FLAGS is a dependency); here a test key does the same in an evaluator with
//! PARSED_FLAGS registered. The mapping lines set native flags only, so no package is loaded.

use std::fmt;
use std::sync::Arc;

use blaze_build_config::bazel_rule_class_provider::BazelRuleClassProvider;
use blaze_build_config::fragment_options_class::FragmentOptionsClass;
use blaze_build_config::parsed_flags::{NativeAndStarlarkFlags, ParsedFlagsValue};
use blaze_build_config::skyframe::{Mappings, parse};
use blaze_cmdline::{Label, RepositoryMapping, RepositoryName};
use blaze_skyframe::{EvaluationContext, EvaluatorBuilder, Key, SkyFunctionName, SkyKey};

const PLATFORM1: &str = "//platforms:one";
const PLATFORM2: &str = "//platforms:two";
const EXTERNAL_PLATFORM: &str = "@@dep+1.0//platforms:two";

fn label(s: &str) -> Label {
    Label::parse_canonical_unchecked(s)
}

fn empty_mapping() -> RepositoryMapping {
    blaze_cmdline::repository_mapping::EMPTY.clone()
}

#[derive(Clone, PartialEq, Eq, Hash)]
struct ParseKey {
    lines: Vec<String>,
    repo_mapping: RepositoryMapping,
}

impl fmt::Debug for ParseKey {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "ParseKey{:?}", self.lines)
    }
}

/// The result of `parse`: the mappings, or the message of the `PlatformMappingParsingException`.
struct Parsed(Result<Mappings, String>);

impl Key for ParseKey {
    type Value = Parsed;
    const FUNCTION: SkyFunctionName = SkyFunctionName("PLATFORM_MAPPING_PARSE_TEST");
    // Java: PlatformMappingFunctionParserTest PARSE_MAPPINGS (createHermetic)
    const HERMETICITY: blaze_skyframe::FunctionHermeticity =
        blaze_skyframe::FunctionHermeticity::Hermetic;
}

fn options_classes() -> Vec<FragmentOptionsClass> {
    BazelRuleClassProvider::create()
        .get_options_classes()
        .to_vec()
}

/// The parsed mappings (the evaluator keeps its own reference to the value).
struct M(Arc<Parsed>);

impl std::ops::Deref for M {
    type Target = Mappings;
    fn deref(&self) -> &Mappings {
        self.0.0.as_ref().expect("parsed")
    }
}

fn parse_with(repo_mapping: RepositoryMapping, lines: &[&str]) -> Result<M, String> {
    let builder = EvaluatorBuilder::new().typed_function::<ParseKey, _>(|key, env| {
        match parse(env, &key.lines, &key.repo_mapping)? {
            Ok(Some(m)) => Ok(Some(Parsed(Ok(m)))),
            Ok(None) => Ok(None),
            Err(e) => Ok(Some(Parsed(Err(e.get_message().to_string())))),
        }
    });
    let evaluator =
        blaze_sky_analysis::parsed_flags_function::register(builder, options_classes()).build();
    let key = ParseKey {
        lines: lines.iter().map(|s| s.to_string()).collect(),
        repo_mapping,
    };
    let result = evaluator
        .evaluate(&[SkyKey::new(key.clone())], &EvaluationContext::default())
        .expect("not interrupted");
    let value = result
        .get(&key)
        .unwrap_or_else(|| panic!("evaluation failed: {:?}", result.error_map()));
    match &value.0 {
        Ok(_) => Ok(M(value.clone())),
        Err(message) => Err(message.clone()),
    }
}

fn parse_lines(lines: &[&str]) -> Result<M, String> {
    parse_with(empty_mapping(), lines)
}

fn create_flags_with(repo_mapping: RepositoryMapping, native_flags: &[&str]) -> ParsedFlagsValue {
    let flags = NativeAndStarlarkFlags {
        native_flags: native_flags.iter().map(|s| s.to_string()).collect(),
        options_classes: options_classes(),
        repo_mapping: Some(repo_mapping),
        ..NativeAndStarlarkFlags::default()
    };
    ParsedFlagsValue::parse_and_create(flags).expect("parses")
}

fn create_flags(native_flags: &[&str]) -> ParsedFlagsValue {
    create_flags_with(empty_mapping(), native_flags)
}

fn platforms_to_flags_keys(m: &Mappings) -> Vec<Label> {
    m.platforms_to_flags.keys().cloned().collect()
}

fn canonical(m: &Mappings, platform: &str) -> Vec<String> {
    m.platforms_to_flags[&label(platform)]
        .parsing_result()
        .canonicalize()
}

/// `flagsToPlatforms.get(createFlags(...))`.
fn flags_to_platform(m: &Mappings, flags: &ParsedFlagsValue) -> Option<Label> {
    m.flags_to_platforms
        .iter()
        .find(|(f, _)| **f == *flags)
        .map(|(_, l)| l.clone())
}

fn assert_sorted_eq(mut actual: Vec<String>, expected: &[&str]) {
    actual.sort();
    let mut expected: Vec<String> = expected.iter().map(|s| s.to_string()).collect();
    expected.sort();
    assert_eq!(actual, expected);
}

fn assert_standard(m: &Mappings) {
    assert_eq!(
        platforms_to_flags_keys(m),
        vec![label(PLATFORM1), label(PLATFORM2)]
    );
    assert_eq!(canonical(m, PLATFORM1), vec!["--cpu=one"]);
    assert_eq!(canonical(m, PLATFORM2), vec!["--cpu=two"]);
    assert_eq!(m.flags_to_platforms.len(), 2);
    assert_eq!(
        flags_to_platform(m, &create_flags(&["--cpu=one"])),
        Some(label(PLATFORM1))
    );
    assert_eq!(
        flags_to_platform(m, &create_flags(&["--cpu=two"])),
        Some(label(PLATFORM2))
    );
}

#[test]
fn test_parse() {
    let m = parse_lines(&[
        "platforms:",
        "  //platforms:one",
        "    --cpu=one",
        "  //platforms:two",
        "    --cpu=two",
        "flags:",
        "  --cpu=one",
        "    //platforms:one",
        "  --cpu=two",
        "    //platforms:two",
    ])
    .unwrap();
    assert_standard(&m);
}

#[test]
fn test_parse_with_repo_mapping() {
    let repo_mapping = RepositoryMapping::create(
        [
            ("foo", blaze_cmdline::repository_name::MAIN.clone()),
            ("dep", RepositoryName::create("dep+1.0").unwrap()),
        ],
        blaze_cmdline::repository_name::MAIN.clone(),
    );
    let m = parse_with(
        repo_mapping.clone(),
        &[
            "platforms:",
            "  @foo//platforms:one",
            "    --cpu=one",
            "  @dep//platforms:two",
            "    --cpu=two",
            "flags:",
            "  --cpu=one",
            "    @foo//platforms:one",
            "  --cpu=two",
            "    @dep//platforms:two",
        ],
    )
    .unwrap();
    assert_eq!(
        platforms_to_flags_keys(&m),
        vec![label(PLATFORM1), label(EXTERNAL_PLATFORM)]
    );
    assert_eq!(canonical(&m, PLATFORM1), vec!["--cpu=one"]);
    assert_eq!(canonical(&m, EXTERNAL_PLATFORM), vec!["--cpu=two"]);
    assert_eq!(m.flags_to_platforms.len(), 2);
    assert_eq!(
        flags_to_platform(&m, &create_flags_with(repo_mapping.clone(), &["--cpu=one"])),
        Some(label(PLATFORM1))
    );
    assert_eq!(
        flags_to_platform(&m, &create_flags_with(repo_mapping, &["--cpu=two"])),
        Some(label(EXTERNAL_PLATFORM))
    );
}

#[test]
fn test_parse_comment() {
    let m = parse_lines(&[
        "# A mapping file!",
        "platforms:",
        "  # comment1",
        "  //platforms:one",
        "# comment2",
        "    --cpu=one",
        "  //platforms:two",
        "    --cpu=two",
        "flags:",
        "# another comment",
        "  --cpu=one",
        "    //platforms:one",
        "  --cpu=two",
        "    //platforms:two",
    ])
    .unwrap();
    assert_standard(&m);
}

#[test]
fn test_parse_whitespace() {
    let m = parse_lines(&[
        "",
        "platforms:",
        "  ",
        "  //platforms:one",
        "",
        "    --cpu=one",
        "    //platforms:two    ",
        "      --cpu=two ",
        "flags:",
        "           ",
        "",
        "--cpu=one",
        "  //platforms:one",
        "  --cpu=two",
        "  //platforms:two",
    ])
    .unwrap();
    assert_standard(&m);
}

#[test]
fn test_parse_multiple_flags_in_platform() {
    let m = parse_lines(&[
        "platforms:",
        "  //platforms:one",
        "    --cpu=one",
        "    --compilation_mode=dbg",
        "  //platforms:two",
        "    --cpu=two",
    ])
    .unwrap();
    assert_eq!(
        platforms_to_flags_keys(&m),
        vec![label(PLATFORM1), label(PLATFORM2)]
    );
    assert_sorted_eq(
        canonical(&m, PLATFORM1),
        &["--cpu=one", "--compilation_mode=dbg"],
    );
}

#[test]
fn test_parse_multiple_flags_in_flags() {
    let m = parse_lines(&[
        "flags:",
        "  --compilation_mode=dbg",
        "  --cpu=one",
        "    //platforms:one",
        "  --cpu=two",
        "    //platforms:two",
    ])
    .unwrap();
    assert_eq!(m.flags_to_platforms.len(), 2);
    assert_eq!(
        flags_to_platform(&m, &create_flags(&["--compilation_mode=dbg", "--cpu=one"])),
        Some(label(PLATFORM1))
    );
    assert!(flags_to_platform(&m, &create_flags(&["--cpu=two"])).is_some());
}

#[test]
fn test_parse_only_platforms() {
    let m = parse_lines(&["platforms:", "  //platforms:one", "    --cpu=one"]).unwrap();
    assert_eq!(platforms_to_flags_keys(&m), vec![label(PLATFORM1)]);
    assert_eq!(canonical(&m, PLATFORM1), vec!["--cpu=one"]);
    assert!(m.flags_to_platforms.is_empty());
}

#[test]
fn test_parse_only_flags() {
    let m = parse_lines(&["flags:", "  --cpu=one", "    //platforms:one"]).unwrap();
    assert_eq!(m.flags_to_platforms.len(), 1);
    assert_eq!(
        flags_to_platform(&m, &create_flags(&["--cpu=one"])),
        Some(label(PLATFORM1))
    );
    assert!(m.platforms_to_flags.is_empty());
}

#[test]
fn test_parse_empty() {
    let m = parse_lines(&[]).unwrap();
    assert!(m.flags_to_platforms.is_empty());
    assert!(m.platforms_to_flags.is_empty());
}

#[test]
fn test_parse_empty_sections() {
    let m = parse_lines(&["platforms:", "flags:"]).unwrap();
    assert!(m.flags_to_platforms.is_empty());
    assert!(m.platforms_to_flags.is_empty());
}

#[test]
fn test_parse_comment_only() {
    let m = parse_lines(&["#No mappings"]).unwrap();
    assert!(m.flags_to_platforms.is_empty());
    assert!(m.platforms_to_flags.is_empty());
}

fn parse_error(lines: &[&str]) -> String {
    match parse_lines(lines) {
        Ok(_) => panic!("expected a PlatformMappingParsingException"),
        Err(message) => message,
    }
}

#[test]
fn test_parse_extra_platform_in_flags() {
    let message = parse_error(&[
        "flags:",
        "  --cpu=one",
        "    //platforms:one",
        "    //platforms:two",
    ]);
    assert!(message.contains("//platforms:two"), "{message}");
}

#[test]
fn test_parse_platform_without_flags() {
    let message = parse_error(&["platforms:", "  //platforms:one"]);
    assert!(message.contains("end of file"), "{message}");
}

#[test]
fn test_parse_flags_without_platform() {
    let message = parse_error(&["flags:", "  --cpu=one"]);
    assert!(message.contains("end of file"), "{message}");
}

#[test]
fn test_parse_comment_end_of_file() {
    let m = parse_lines(&[
        "platforms:",
        "  //platforms:one",
        "    --cpu=one",
        "# No more mappings",
    ])
    .unwrap();
    assert!(!m.platforms_to_flags.is_empty());
}

#[test]
fn test_parse_unknown_section() {
    let message = parse_error(&["platform:", "  //platforms:one", "    --cpu=one"]);
    assert!(message.contains("platform:"), "{message}");

    let message = parse_error(&[
        "platforms:",
        "  //platforms:one",
        "    --cpu=one",
        "flag:",
        "  --cpu=one",
        "    //platforms:one",
    ]);
    assert!(message.contains("platform"), "{message}");
}

#[test]
fn test_parse_platforms_invalid_platform_label() {
    let message = parse_error(&["platforms:", "  @@@", "    --cpu=one"]);
    assert!(message.contains("@@@"), "{message}");
}

#[test]
fn test_parse_flags_invalid_platform_label() {
    let message = parse_error(&["flags:", "  --cpu=one", "    @@@"]);
    assert!(message.contains("@@@"), "{message}");
}

#[test]
fn test_parse_platforms_invalid_flag() {
    let message = parse_error(&["platforms:", "  //platforms:one", "    -cpu=one"]);
    assert!(message.contains("-cpu"), "{message}");
}

#[test]
fn test_parse_flags_invalid_flag() {
    let message = parse_error(&["flags:", "  -cpu=one", "    //platforms:one"]);
    assert!(message.contains("-cpu"), "{message}");
}

#[test]
fn test_parse_platforms_duplicate_platform() {
    let message = parse_error(&[
        "platforms:",
        "  //platforms:one",
        "    --cpu=one",
        "  //platforms:one",
        "    --cpu=two",
    ]);
    assert!(message.contains("duplicate"), "{message}");
}

#[test]
fn test_parse_flags_duplicate_flags() {
    let message = parse_error(&[
        "flags:",
        "  --compilation_mode=dbg",
        "  --cpu=one",
        "    //platforms:one",
        "  --compilation_mode=dbg",
        "  --cpu=one",
        "    //platforms:two",
    ]);
    assert!(message.contains("duplicate"), "{message}");
}
