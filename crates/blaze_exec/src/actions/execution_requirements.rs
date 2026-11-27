//! Port of the string constants of `lib/actions/ExecutionRequirements.java`.

// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:31 ExecutionRequirements
pub const TIMEOUT: &str = "timeout";
pub const REQUIRES_DARWIN: &str = "requires-darwin";
pub const DISABLE_LOCAL_PREFETCH: &str = "disable-local-prefetch";
pub const SUPPORTS_WORKERS: &str = "supports-workers";
pub const SUPPORTS_MULTIPLEX_WORKERS: &str = "supports-multiplex-workers";
pub const REQUIRES_WORKER_PROTOCOL: &str = "requires-worker-protocol";
pub const SUPPORTS_WORKER_CANCELLATION: &str = "supports-worker-cancellation";
pub const SUPPORTS_MULTIPLEX_SANDBOXING: &str = "supports-multiplex-sandboxing";
pub const WORKER_KEY_MNEMONIC: &str = "worker-key-mnemonic";
pub const LOCAL: &str = "local";
pub const NO_CACHE: &str = "no-cache";
pub const NO_REMOTE_CACHE: &str = "no-remote-cache";
pub const NO_REMOTE_CACHE_UPLOAD: &str = "no-remote-cache-upload";
pub const NO_REMOTE_EXEC: &str = "no-remote-exec";
pub const NO_TESTLOASD: &str = "no-testloasd";
pub const NO_REMOTE: &str = "no-remote";
pub const NO_LOCAL: &str = "no-local";
pub const LEGACY_NOSANDBOX: &str = "nosandbox";
pub const NO_SANDBOX: &str = "no-sandbox";
pub const REQUIREMENTS_SET: &str = "supports-xcode-requirements-set";
pub const REQUIRES_NETWORK: &str = "requires-network";
pub const BLOCK_NETWORK: &str = "block-network";
pub const REQUIRES_FAKEROOT: &str = "requires-fakeroot";
pub const DO_NOT_REPORT: &str = "internal-do-not-report";
pub const REMOTE_EXECUTION_INLINE_OUTPUTS: &str = "internal-inline-outputs";
pub const MEMORY_ESTIMATE: &str = "internal-memory-estimate";
pub const GRACEFUL_TERMINATION: &str = "supports-graceful-termination";
pub const REQUIRES_XCODE: &str = "requires-xcode";
pub const REQUIRES_XCODE_LABEL: &str = "requires-xcode-label";
pub const SUPPORTS_PATH_MAPPING: &str = "supports-path-mapping";
pub const DIFFERENTIATE_WORKSPACE_CACHE: &str = "internal-differentiate-workspace-cache";

/// Denotes what the type of worker protocol the worker uses.
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:286 ExecutionRequirements.WorkerProtocolFormat
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WorkerProtocolFormat {
    Json,
    Proto,
}

impl std::fmt::Display for WorkerProtocolFormat {
    /// `toString()`: the enum constant's name.
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            WorkerProtocolFormat::Json => "JSON",
            WorkerProtocolFormat::Proto => "PROTO",
        })
    }
}

/// Thrown when a [`ParseableRequirement`] feels responsible for a tag, but the validator returns
/// an error.
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:64 ExecutionRequirements.ParseableRequirement.ValidationException
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ValidationException {
    tag_value: String,
    message: String,
}

impl ValidationException {
    /// Creates a new `ValidationException`. `tag_value`: the erroneous value that was parsed from
    /// the tag. `error_msg`: an error message that tells the user what's wrong with the value.
    // Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:70 ExecutionRequirements.ParseableRequirement.ValidationException.ValidationException
    pub fn new(tag_value: &str, error_msg: &str) -> ValidationException {
        ValidationException {
            tag_value: tag_value.to_string(),
            message: error_msg.to_string(),
        }
    }

    /// Returns the erroneous value of the parsed tag. Useful to put in error messages shown to
    /// the user.
    // Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:80 ExecutionRequirements.ParseableRequirement.ValidationException.getTagValue
    pub fn get_tag_value(&self) -> &str {
        &self.tag_value
    }

    /// `getMessage()`.
    pub fn get_message(&self) -> &str {
        &self.message
    }
}

/// An execution requirement that can be split into a key and a value part using a regex.
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:49 ExecutionRequirements.ParseableRequirement
pub struct ParseableRequirement {
    user_friendly_name: &'static str,
    /// Java's `Pattern` used with `matches()`: anchored here.
    detection_pattern: regex::Regex,
    validator: fn(&str) -> Option<String>,
}

impl ParseableRequirement {
    /// Create a new parseable execution requirement definition.
    ///
    /// If a tag doesn't match the detection pattern, it will be ignored. If a tag matches the
    /// detection pattern, but not the validator, it is assumed that the value is somehow wrong
    /// (e.g. the user put a float or random string where we expected an integer).
    ///
    /// `detection_pattern` (anchored, as `Matcher.matches()`) should have one capture group that
    /// grabs the value of the tag. `validator` returns `None` if the value is fine to use or a
    /// human-friendly error message describing why the value is not valid.
    // Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:51 ExecutionRequirements.ParseableRequirement.ParseableRequirement
    // Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:100 ExecutionRequirements.ParseableRequirement.create
    fn create(
        user_friendly_name: &'static str,
        detection_pattern: &str,
        validator: fn(&str) -> Option<String>,
    ) -> ParseableRequirement {
        ParseableRequirement {
            user_friendly_name,
            detection_pattern: regex::Regex::new(detection_pattern).unwrap(),
            validator,
        }
    }

    /// `userFriendlyName()`.
    pub fn user_friendly_name(&self) -> &'static str {
        self.user_friendly_name
    }

    /// Returns the parsed value from a tag, if this requirement detects that it is responsible for
    /// it, otherwise `None`. Fails with [`ValidationException`] if the value parsed out of the tag
    /// doesn't pass the validator.
    // Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:111 ExecutionRequirements.ParseableRequirement.parseIfMatches
    pub fn parse_if_matches(&self, tag: &str) -> Result<Option<String>, ValidationException> {
        let Some(m) = self.detection_pattern.captures(tag) else {
            return Ok(None);
        };
        let tag_value = &m[1];
        if let Some(error_msg) = (self.validator)(tag_value) {
            return Err(ValidationException::new(tag_value, &error_msg));
        }
        Ok(Some(tag_value.to_string()))
    }
}

/// Java's `.` in a `Pattern` without DOTALL: any char but a line terminator.
const JAVA_DOT: &str = r"[^\n\r\u{85}\u{2028}\u{2029}]";

/// How many hardware threads an action requires for execution.
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:136 ExecutionRequirements.CPU
pub static CPU: std::sync::LazyLock<ParseableRequirement> = std::sync::LazyLock::new(|| {
    ParseableRequirement::create("cpu:<int>", &format!("^cpu:({JAVA_DOT}+)$"), |s| {
        // Integer.parseInt.
        let Some(value) = blaze_options::converters::java_parse_int(s) else {
            return Some("can't be parsed as an integer".to_string());
        };

        // De-and-reserialize & compare to only allow canonical integer formats.
        if value.to_string() != s {
            return Some("must be in canonical format (e.g. '4' instead of '+04')".to_string());
        }

        if value < 1 {
            return Some("can't be zero or negative".to_string());
        }

        None
    })
});

/// How many extra resources an action requires for execution.
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:163 ExecutionRequirements.RESOURCES
pub static RESOURCES: std::sync::LazyLock<ParseableRequirement> = std::sync::LazyLock::new(|| {
    ParseableRequirement::create(
        "resources:<str>:<float>",
        &format!("^resources:({JAVA_DOT}+:{JAVA_DOT}+)$"),
        |s| {
            let split_index = s.find(':').expect("the pattern has a ':'");
            let resource_count = &s[split_index + 1..];
            // Float.parseFloat.
            let Some(value) = blaze_options::converters::java_parse_double(resource_count) else {
                return Some("can't be parsed as a float".to_string());
            };
            if (value as f32) < 0.0 {
                return Some("can't be negative".to_string());
            }
            None
        },
    )
});

/// Parses resource requirements from a string map. Handles both execution info (tag format) and
/// exec_properties (key-value format):
///
/// - Tags: key is the full tag (e.g. `"resources:cpu:4"` or `"cpu:2"`), value is empty.
/// - exec_properties: key is the resource prefix (e.g. `"resources:cpu"`), value is the amount
///   (e.g. `"4"`).
///
/// In both cases, the entry is normalized to tag format and parsed with [`RESOURCES`] and
/// [`CPU`]. Returns resource name to amount (in the iteration order of Java's `HashMap`, which
/// `ImmutableMap.copyOf` keeps); empty if no resource entries are found. Fails if a matching
/// entry has an invalid value or a resource is specified more than once.
// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:204 ExecutionRequirements.parseResources
pub fn parse_resources(
    map: &indexmap::IndexMap<String, String>,
) -> Result<indexmap::IndexMap<String, f64>, crate::actions::UserExecException> {
    use blaze_protos::failure_details::test_action::Code;

    if map.is_empty() {
        return Ok(indexmap::IndexMap::new());
    }

    // Java: HashMap<String, Double>; the values are kept as their strings until the end.
    let mut resources = blaze_util::java_hash_map::JavaHashMap::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    for (key, value) in map {
        // Normalize to tag format: tags have the value baked into the key (e.g.
        // "resources:cpu:4" with empty value), exec_properties split it (e.g. key
        // "resources:cpu", value "4").
        let tag = if value.is_empty() {
            key.clone()
        } else {
            format!("{key}:{value}")
        };

        let mut requirement: &ParseableRequirement = &RESOURCES;
        let parsed = (|| -> Result<Option<(String, String)>, ValidationException> {
            if let Some(parsed) = requirement.parse_if_matches(&tag)? {
                let split_index = parsed.find(':').expect("the pattern has a ':'");
                return Ok(Some((
                    parsed[..split_index].to_string(),
                    parsed[split_index + 1..].to_string(),
                )));
            }
            requirement = &CPU;
            match requirement.parse_if_matches(&tag)? {
                Some(cpu_value) => Ok(Some(("cpu".to_string(), cpu_value))),
                None => Ok(None),
            }
        })();
        let (resource, amount) = match parsed {
            Ok(Some(p)) => p,
            Ok(None) => {
                if tag.starts_with("resources:") {
                    // A key clearly intended as a resource that didn't match the expected format
                    // (e.g. "resources:cpu:" with an empty amount).
                    return Err(crate::actions::UserExecException::new(
                        create_failure_detail(
                            &format!(
                                "'{tag}' is not a valid '{}' entry",
                                RESOURCES.user_friendly_name()
                            ),
                            Code::InvalidCpuTag,
                        ),
                    ));
                }
                continue;
            }
            Err(e) => {
                return Err(crate::actions::UserExecException::new(
                    create_failure_detail(
                        &format!(
                            "'{tag}' has a '{}' entry, but its value '{}' didn't pass validation: {}",
                            requirement.user_friendly_name(),
                            e.get_tag_value(),
                            e.get_message()
                        ),
                        Code::InvalidCpuTag,
                    ),
                ));
            }
        };
        if seen.contains(&resource) {
            return Err(crate::actions::UserExecException::new(
                create_failure_detail(
                    &format!(
                        "'{tag}' has more than one entry for resource '{resource}', but duplicates aren't allowed"
                    ),
                    Code::DuplicateCpuTags,
                ),
            ));
        }
        seen.insert(resource.clone());
        resources.put(resource, amount);
    }
    Ok(resources
        .into_entries()
        .into_iter()
        .map(|(resource, amount)| {
            // Double.parseDouble: the validators accepted the amount.
            let amount =
                blaze_options::converters::java_parse_double(&amount).expect("validated amount");
            (resource, amount)
        })
        .collect())
}

// Java: com/google/devtools/build/lib/actions/ExecutionRequirements.java:266 ExecutionRequirements.createFailureDetail
fn create_failure_detail(
    message: &str,
    detailed_code: blaze_protos::failure_details::test_action::Code,
) -> blaze_protos::failure_details::FailureDetail {
    use blaze_protos::failure_details::{FailureDetail, TestAction, failure_detail};
    FailureDetail {
        message: message.to_string(),
        category: Some(failure_detail::Category::TestAction(TestAction {
            code: detailed_code as i32,
        })),
    }
}
