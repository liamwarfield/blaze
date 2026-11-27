//! Port of `src/test/java/com/google/devtools/build/lib/runtime/commands/ConfigCommandTest.java`:
//! `build --nobuild` and then `config` on one runtime, reading the configurations the build left
//! in the server's graph.
//!
//! PORT: Java parses the JSON output with Gson into `ConfigurationForOutput` and
//! `ConfigurationDiffForOutput`; here the same fields are read from `serde_json` values.

use std::collections::BTreeSet;

use blaze_buildtool_testing::BuildIntegrationTestCase;
use serde_json::Value;

// Java: src/test/java/com/google/devtools/build/lib/runtime/commands/ConfigCommandTest.java:62 ConfigCommandTest.init
fn init() -> BuildIntegrationTestCase {
    let t = BuildIntegrationTestCase::new();
    t.write(
        "tools/allowlists/function_transition_allowlist/BUILD",
        &[
            "package_group(",
            "    name = \"function_transition_allowlist\",",
            "    packages = [\"//...\"],",
            ")",
        ],
    );
    t.write(
        "test/defs.bzl",
        &[
            "def _simple_rule_impl(ctx):",
            "    pass",
            "",
            "simple_rule = rule(",
            "    implementation = _simple_rule_impl,",
            "    attrs = {},",
            ")",
            "",
            "def _sometransition_impl(settings, attr):",
            "    _ignore = (settings, attr)",
            "    return {\"//command_line_option:platform_suffix\": \"transitioned\"}",
            "",
            "_sometransition = transition(",
            "    implementation = _sometransition_impl,",
            "    inputs = [],",
            "    outputs = [\"//command_line_option:platform_suffix\"],",
            ")",
            "rule_with_transition = rule(",
            "    implementation = _simple_rule_impl,",
            "    cfg = _sometransition,",
            ")",
        ],
    );
    t.write(
        "test/BUILD",
        &[
            "load(\"//test:defs.bzl\", \"rule_with_transition\", \"simple_rule\")",
            "",
            "simple_rule(name = \"buildme\")",
            "",
            "rule_with_transition(name = \"buildme_with_transition\")",
        ],
    );
    t
}

/// Performs loading and analysis on the fixed rule `//test:buildme` with the given build options
/// (as they'd appear on the command line).
// Java: src/test/java/com/google/devtools/build/lib/runtime/commands/ConfigCommandTest.java:113 ConfigCommandTest.analyzeTarget
fn analyze_target(t: &mut BuildIntegrationTestCase, args: &[&str]) {
    analyze(t, "//test:buildme", args);
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/commands/ConfigCommandTest.java:128 ConfigCommandTest.analyzeTargetWithTransition
fn analyze_target_with_transition(t: &mut BuildIntegrationTestCase, args: &[&str]) {
    analyze(t, "//test:buildme_with_transition", args);
}

fn analyze(t: &mut BuildIntegrationTestCase, target: &str, args: &[&str]) {
    let mut params: Vec<String> = vec!["build".to_owned()];
    // Ideally we'd directly use BlazeRuntimeWrapper, but it's explicitly documented as not
    // exercising the command dispatcher, which is exactly what we need here.
    params.extend(t.get_options().iter().cloned());
    params.push(target.to_owned());
    params.push("--nobuild".to_owned()); // Execution phase isn't necessary to collect configurations.
    params.extend(args.iter().map(|a| a.to_string()));
    let params: Vec<&str> = params.iter().map(String::as_str).collect();
    let result = t.dispatch(&params);
    // Java ignores the build's result; a failed analysis would leave no configurations and the
    // assertions below would fail less clearly.
    assert!(result.is_success(), "{}", t.out_err().err_as_latin1());
}

/// Calls `blaze config --output=json` with the given flags and returns (stdout, stderr).
// Java: src/test/java/com/google/devtools/build/lib/runtime/commands/ConfigCommandTest.java:144 ConfigCommandTest.callConfigCommand
fn call_config_command(t: &mut BuildIntegrationTestCase, args: &[&str]) -> (String, String) {
    let mut params = vec!["config", "--output=json"];
    params.extend_from_slice(args);
    t.dispatch(&params);
    (t.out_err().out_as_latin1(), t.out_err().err_as_latin1())
}

fn config_json(t: &mut BuildIntegrationTestCase, args: &[&str]) -> Value {
    let (out, err) = call_config_command(t, args);
    serde_json::from_str(&out).unwrap_or_else(|e| panic!("{e}: {out:?} (stderr {err})"))
}

/// Returns the value of an option under a configuration's `FragmentOptions`.
// Java: src/test/java/com/google/devtools/build/lib/runtime/commands/ConfigCommandTest.java:158 ConfigCommandTest.getOptionValue
fn get_option_value(config: &Value, fragment_options: &str, option_name: &str) -> String {
    let ans: Vec<String> = config["fragmentOptions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["name"].as_str().unwrap().ends_with(fragment_options))
        .flat_map(|f| f["options"].as_object().unwrap().iter())
        .filter(|(k, _)| k.as_str() == option_name)
        .map(|(_, v)| v.as_str().unwrap().to_owned())
        .collect();
    match ans.len() {
        1 => ans[0].clone(),
        0 => panic!("No matches for fragment={fragment_options}, option={option_name}"),
        _ => panic!("Multiple matches for fragment={fragment_options}, option={option_name}"),
    }
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/commands/ConfigCommandTest.java:178 ConfigCommandTest.isTargetConfig
fn is_target_config(config: &Value) -> bool {
    if config["mnemonic"].as_str().unwrap().ends_with("-noconfig") {
        return false;
    }
    get_option_value(config, "CoreOptions", "is exec configuration") != "true"
}

/// Converts `a.b.d` to `d`.
// Java: src/test/java/com/google/devtools/build/lib/runtime/commands/ConfigCommandTest.java:186 ConfigCommandTest.getBaseName
fn get_base_name(s: &str) -> &str {
    &s[s.rfind('.').map_or(0, |i| i + 1)..]
}

fn config_ids(t: &mut BuildIntegrationTestCase) -> Vec<String> {
    config_json(t, &[])["configuration-IDs"]
        .as_array()
        .unwrap()
        .iter()
        .map(|v| v.as_str().unwrap().to_owned())
        .collect()
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/commands/ConfigCommandTest.java:196 ConfigCommandTest.showConfigIds
#[test]
fn show_config_ids() {
    let mut t = init();
    analyze_target(&mut t, &[]);
    let full_json = config_json(&mut t, &[]);
    // Should be: target configuration, target configuration without test.
    assert!(full_json.get("configuration-IDs").is_some());
    assert_eq!(full_json["configuration-IDs"].as_array().unwrap().len(), 3);
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/commands/ConfigCommandTest.java:206 ConfigCommandTest.skipNoConfig
fn skip_no_config(t: &mut BuildIntegrationTestCase, config_hash: &str) -> bool {
    !config_json(t, &[config_hash])["mnemonic"]
        .as_str()
        .unwrap()
        .contains("-noconfig")
}

/// Calls the config command to return all config hashes currently available.
// Java: src/test/java/com/google/devtools/build/lib/runtime/commands/ConfigCommandTest.java:228 ConfigCommandTest.getConfigHashes
fn get_config_hashes(t: &mut BuildIntegrationTestCase, include_no_config: bool) -> Vec<String> {
    config_ids(t)
        .into_iter()
        .filter(|c| include_no_config || skip_no_config(t, c))
        .collect()
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/commands/ConfigCommandTest.java:241 ConfigCommandTest.showSingleConfig
#[test]
fn show_single_config() {
    let mut t = init();
    analyze_target(&mut t, &[]);
    // Find the first non-noconfig configuration (see NoConfigTransition). noconfig is a special
    // configuration that strips away most of its structure, so not a good candidate for this
    // test.
    let config_hash = get_config_hashes(&mut t, false)[0].clone();
    let config = config_json(&mut t, &[&config_hash]);

    // Verify config metadata:
    assert_eq!(config["configHash"].as_str().unwrap(), config_hash);
    assert_eq!(
        config["skyKey"].as_str().unwrap(),
        format!("BuildConfigurationKey[{config_hash}]")
    );
    // Verify the existence of a couple of expected fragments:
    let fragments: Vec<(String, Vec<String>)> = config["fragments"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| {
            (
                get_base_name(f["name"].as_str().unwrap()).to_owned(),
                f["fragmentOptions"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .map(|o| get_base_name(o.as_str().unwrap()).to_owned())
                    .collect(),
            )
        })
        .collect();
    for expected in [
        ("PlatformConfiguration", vec!["PlatformOptions"]),
        ("TestConfiguration", vec!["TestConfiguration$TestOptions"]),
    ] {
        let expected = (
            expected.0.to_owned(),
            expected.1.into_iter().map(str::to_owned).collect(),
        );
        assert!(
            fragments.contains(&expected),
            "{expected:?} in {fragments:?}"
        );
    }
    // Verify the existence of a couple of expected fragment options:
    let fragment_options: Vec<String> = config["fragmentOptions"]
        .as_array()
        .unwrap()
        .iter()
        .map(|f| get_base_name(f["name"].as_str().unwrap()).to_owned())
        .collect();
    for expected in ["PlatformOptions", "CoreOptions", "user-defined"] {
        assert!(
            fragment_options.iter().any(|f| f == expected),
            "{expected} in {fragment_options:?}"
        );
    }
    // Verify the existence of a couple of expected option names:
    let core_option_names: BTreeSet<String> = config["fragmentOptions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["name"].as_str().unwrap().ends_with("CoreOptions"))
        .flat_map(|f| f["options"].as_object().unwrap().keys().cloned())
        .collect();
    for expected in ["run_under", "check_visibility", "stamp"] {
        assert!(core_option_names.contains(expected), "{expected}");
    }
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/commands/ConfigCommandTest.java:283 ConfigCommandTest.showSingleConfigHashPrefix
#[test]
fn show_single_config_hash_prefix() {
    let mut t = init();
    analyze_target(&mut t, &[]);
    let config_hash = config_ids(&mut t)[0].clone();
    let hash_prefix = &config_hash[..config_hash.len() / 2];
    let config = config_json(&mut t, &[hash_prefix]);
    assert!(
        config["configHash"]
            .as_str()
            .unwrap()
            .starts_with(hash_prefix)
    );
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/commands/ConfigCommandTest.java:301 ConfigCommandTest.unknownHashPrefix
#[test]
fn unknown_hash_prefix() {
    let mut t = init();
    analyze_target(&mut t, &[]);
    let config_hash = config_ids(&mut t)[0].clone();
    // No valid hash has spaces.
    let hash_prefix = format!("{} ", &config_hash[..config_hash.len() / 2]);
    let (_, err) = call_config_command(&mut t, &[&hash_prefix]);
    assert!(
        err.contains(&format!(
            "No configuration found with ID prefix {hash_prefix}"
        )),
        "{err}"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/commands/ConfigCommandTest.java:317 ConfigCommandTest.showAllConfigs
#[test]
fn show_all_configs() {
    let mut t = init();
    analyze_target(&mut t, &[]);
    let all = config_json(&mut t, &["--dump_all"]);
    let num_configs = all.as_array().unwrap().len();
    assert_eq!(num_configs, 3); // Target + target w/o test + nonConfig.
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/commands/ConfigCommandTest.java:380 ConfigCommandTest.getTargetConfig
fn get_target_config(t: &mut BuildIntegrationTestCase, excluded_hashes: &[String]) -> Value {
    // Find a target configuration hash.
    for config_hash in config_ids(t) {
        if excluded_hashes.contains(&config_hash) {
            continue;
        }
        let config = config_json(t, &[&config_hash]);
        if is_target_config(&config) {
            return config;
        }
    }
    panic!("Should have found config hash");
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/commands/ConfigCommandTest.java:331 ConfigCommandTest.compareConfigs
#[test]
fn compare_configs() {
    let mut t = init();
    // Do not trim test configuration for now to make 'finding' the configurations easier.
    analyze_target_with_transition(
        &mut t,
        &["--platform_suffix=pure", "--notrim_test_configuration"],
    );
    let target_config1_hash = get_target_config(&mut t, &[])["configHash"]
        .as_str()
        .unwrap()
        .to_owned();
    let target_config2_hash =
        get_target_config(&mut t, &[target_config1_hash.clone()])["configHash"]
            .as_str()
            .unwrap()
            .to_owned();

    // Get their diff.
    let diff = config_json(&mut t, &[&target_config1_hash, &target_config2_hash]);
    assert_eq!(diff["configHash1"].as_str().unwrap(), target_config1_hash);
    assert_eq!(diff["configHash2"].as_str().unwrap(), target_config2_hash);
    let fragments_diff = diff["fragmentsDiff"].as_array().unwrap();
    assert_eq!(fragments_diff.len(), 1);
    let fragment_diff = &fragments_diff[0];
    assert!(
        fragment_diff["name"]
            .as_str()
            .unwrap()
            .ends_with("CoreOptions")
    );
    let options_diff: Vec<(&String, &Value)> = fragment_diff["optionsDiff"]
        .as_object()
        .unwrap()
        .iter()
        .filter(|(k, _)| k.as_str() != "affected by starlark transition")
        .collect();
    assert_eq!(options_diff.len(), 1);
    let (key, value) = options_diff[0];
    assert_eq!(key, "platform_suffix");
    // The ordering of the difference depends on which configuration comes first, which depends
    // on the configuration hash name, which we can't predict statically.
    let mut values = vec![
        value["first"].as_str().unwrap().to_owned(),
        value["second"].as_str().unwrap().to_owned(),
    ];
    values.sort();
    assert_eq!(values, vec!["pure", "transitioned"]);
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/commands/ConfigCommandTest.java:360 ConfigCommandTest.compareConfigsHashPrefix
#[test]
fn compare_configs_hash_prefix() {
    let mut t = init();
    // Do not trim test configuration for now to make 'finding' the configurations easier.
    analyze_target_with_transition(
        &mut t,
        &["--platform_suffix=pure", "--notrim_test_configuration"],
    );
    let target_config1_hash = get_target_config(&mut t, &[])["configHash"]
        .as_str()
        .unwrap()
        .to_owned();
    let target_config2_hash =
        get_target_config(&mut t, &[target_config1_hash.clone()])["configHash"]
            .as_str()
            .unwrap()
            .to_owned();

    let hash_prefix1 = &target_config1_hash[..target_config1_hash.len() / 2];
    let hash_prefix2 = &target_config2_hash[..target_config2_hash.len() / 2];

    let diff = config_json(&mut t, &[hash_prefix1, hash_prefix2]);
    assert!(
        diff["configHash1"]
            .as_str()
            .unwrap()
            .starts_with(hash_prefix1)
    );
    assert!(
        diff["configHash2"]
            .as_str()
            .unwrap()
            .starts_with(hash_prefix2)
    );
}

fn first_target_config_of_dump_all(t: &mut BuildIntegrationTestCase) -> Value {
    let all = config_json(t, &["--dump_all"]);
    all.as_array()
        .unwrap()
        .iter()
        .find(|c| is_target_config(c))
        .cloned()
        .expect("a target configuration")
}

fn core_options_named<'a>(config: &'a Value, name: &str) -> Vec<&'a String> {
    config["fragmentOptions"]
        .as_array()
        .unwrap()
        .iter()
        .filter(|f| f["name"].as_str().unwrap().ends_with("CoreOptions"))
        .flat_map(|f| f["options"].as_object().unwrap().keys())
        .filter(|n| n.as_str() == name)
        .collect()
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/commands/ConfigCommandTest.java:407 ConfigCommandTest.starlarkFlagsInUserDefinedFragment
#[test]
fn starlark_flags_in_user_defined_fragment() {
    let mut t = init();
    t.write(
        "test/flagdef.bzl",
        &[
            "def _rule_impl(ctx):",
            "    return []",
            "",
            "string_flag = rule(",
            "    implementation = _rule_impl,",
            "    build_setting = config.string(flag = True),",
            ")",
            "simple_rule = rule(",
            "    implementation = _rule_impl,",
            "    attrs = {},",
            ")",
        ],
    );
    t.write(
        "custom_flags/BUILD",
        &[
            "load(\"//test:flagdef.bzl\", \"string_flag\")",
            "",
            "string_flag(",
            "    name = \"my_flag\",",
            "    build_setting_default = \"\",",
            ")",
        ],
    );

    analyze_target(&mut t, &["--//custom_flags:my_flag=hello"]);

    let target_config = first_target_config_of_dump_all(&mut t);
    assert_eq!(
        get_option_value(&target_config, "user-defined", "//custom_flags:my_flag"),
        "hello"
    );
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/commands/ConfigCommandTest.java:452 ConfigCommandTest.defineFlagsIndividuallyListedInUserDefinedFragment
#[test]
fn define_flags_individually_listed_in_user_defined_fragment() {
    let mut t = init();
    analyze_target(&mut t, &["--define", "a=1", "--define", "b=2"]);

    let target_config = first_target_config_of_dump_all(&mut t);
    assert_eq!(
        get_option_value(&target_config, "user-defined", "--define:a"),
        "1"
    );
    assert_eq!(
        get_option_value(&target_config, "user-defined", "--define:b"),
        "2"
    );
    assert!(core_options_named(&target_config, "define").is_empty());
}

// Java: src/test/java/com/google/devtools/build/lib/runtime/commands/ConfigCommandTest.java:478 ConfigCommandTest.conflictingDefinesLastWins
#[test]
fn conflicting_defines_last_wins() {
    let mut t = init();
    analyze_target(&mut t, &["--define", "a=1", "--define", "a=2"]);

    let target_config = first_target_config_of_dump_all(&mut t);
    assert_eq!(
        get_option_value(&target_config, "user-defined", "--define:a"),
        "2"
    );
    assert!(core_options_named(&target_config, "define").is_empty());
}
