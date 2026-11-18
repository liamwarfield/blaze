#![cfg(feature = "wip")]
// Options and protos are filled in field by field, as the Java tests set them.
#![allow(clippy::field_reassign_with_default)]
//! Port of `src/test/java/com/google/devtools/build/lib/remote/ScrubberTest.java`.
//!
//! PORT: `SpawnScrubber.matches` reads `ActionOwner.getTargetKind()` and
//! `ActionOwner.isBuildConfigurationForTool()`, which `blaze_exec::actions::ActionOwner` does not
//! carry (`Scrubber` fails with `unported!` there). Every test that matches a spawn against a
//! rule is therefore ignored; its body is the Java test. Once the two hooks exist, set them in
//! [`create_spawn_with_kind`] and remove the `#[ignore]`s.

use std::sync::Arc;

use blaze_cmdline::Label;
use blaze_collect::NestedSetBuilder;
use blaze_exec::actions::{FakeOwner, ResourceSet, SimpleSpawn};
use blaze_protos::remote_scrubbing::{Config, config};
use blaze_remote::scrubber::Scrubber;
use blaze_vfs::PathFragment;
use indexmap::IndexMap;

/// `Config.newBuilder().addRules(Config.Rule.newBuilder().setMatcher(matcher)).build()`.
fn config_with_matcher(matcher: config::Matcher) -> Config {
    Config {
        rules: vec![config::Rule {
            matcher: Some(matcher),
            transform: None,
        }],
    }
}

/// `Config.newBuilder().addRules(Config.Rule.newBuilder().setTransform(transform)).build()`.
fn config_with_transform(transform: config::Transform) -> Config {
    Config {
        rules: vec![config::Rule {
            matcher: None,
            transform: Some(transform),
        }],
    }
}

/// `Config.Matcher.newBuilder().setMnemonic(mnemonic)`.
fn mnemonic_matcher(mnemonic: &str) -> config::Matcher {
    config::Matcher {
        mnemonic: mnemonic.to_string(),
        ..Default::default()
    }
}

/// `Config.Matcher.newBuilder().setLabel(label)`.
fn label_matcher(label: &str) -> config::Matcher {
    config::Matcher {
        label: label.to_string(),
        ..Default::default()
    }
}

/// `Config.Matcher.newBuilder().setKind(kind)`.
fn kind_matcher(kind: &str) -> config::Matcher {
    config::Matcher {
        kind: kind.to_string(),
        ..Default::default()
    }
}

/// `Config.Replacement.newBuilder().setSource(source).setTarget(target)`.
fn replacement(source: &str, target: &str) -> config::Replacement {
    config::Replacement {
        source: source.to_string(),
        target: target.to_string(),
    }
}

/// `Config.Transform.newBuilder().addOmittedInputs(...)`.
fn omitted_inputs(omitted_inputs: &[&str]) -> config::Transform {
    config::Transform {
        omitted_inputs: omitted_inputs.iter().map(|s| s.to_string()).collect(),
        ..Default::default()
    }
}

/// `Config.Transform.newBuilder().addArgReplacements(...)`.
fn arg_replacements(arg_replacements: Vec<config::Replacement>) -> config::Transform {
    config::Transform {
        arg_replacements,
        ..Default::default()
    }
}

fn new_scrubber(config: Config) -> Scrubber {
    Scrubber::new(&config).unwrap()
}

// Java: ScrubberTest.createSpawn()
fn create_spawn() -> SimpleSpawn {
    create_spawn_with_label("//foo:bar", "Foo")
}

// Java: ScrubberTest.createSpawn(String label, String mnemonic)
fn create_spawn_with_label(label: &str, mnemonic: &str) -> SimpleSpawn {
    create_spawn_with_kind(
        label,
        mnemonic,
        /* rule_kind= */ "dummy-target-kind",
        /* for_tool= */ false,
    )
}

// Java: ScrubberTest.createSpawn(String label, String mnemonic, String ruleKind, boolean forTool)
fn create_spawn_with_kind(
    label: &str,
    mnemonic: &str,
    _rule_kind: &str,
    _for_tool: bool,
) -> SimpleSpawn {
    // Java: new SpawnBuilder("cmd").withOwnerLabel(label).withMnemonic(mnemonic)
    //     .withOwnerRuleKind(ruleKind).setBuiltForToolConfiguration(forTool).build()
    // PORT: `blaze_exec::actions::ActionOwner` has no field for the rule kind or for the tool
    // configuration bit, so `rule_kind` and `for_tool` cannot be set (see the module comment).
    let owner = FakeOwner::new(
        mnemonic,
        Some("progress message"),
        Some(Label::parse_canonical_unchecked(label)),
    );
    SimpleSpawn::new_simple(
        Arc::new(owner),
        vec!["cmd".to_string()],
        IndexMap::new(),
        IndexMap::new(),
        NestedSetBuilder::stable_order().build(),
        vec![],
        Arc::new(ResourceSet::zero()),
    )
}

#[test]
fn no_scrubbing() {
    let scrubber = new_scrubber(Config::default());

    assert!(
        scrubber
            .for_spawn(&create_spawn_with_label("//foo:bar", "Foo"))
            .is_none()
    );
}

#[test]
#[ignore = "needs ActionOwner.getTargetKind and ActionOwner.isBuildConfigurationForTool (missing in blaze_exec::actions::ActionOwner)"]
fn match_exact_mnemonic() {
    let scrubber = new_scrubber(config_with_matcher(mnemonic_matcher("Foo")));

    assert!(
        scrubber
            .for_spawn(&create_spawn_with_label("//foo:bar", "Foo"))
            .is_some()
    );
    assert!(
        scrubber
            .for_spawn(&create_spawn_with_label("//foo:bar", "Foobar"))
            .is_none()
    );
}

#[test]
#[ignore = "needs ActionOwner.getTargetKind and ActionOwner.isBuildConfigurationForTool (missing in blaze_exec::actions::ActionOwner)"]
fn match_union_mnemonic() {
    let scrubber = new_scrubber(config_with_matcher(mnemonic_matcher("Foo|Bar")));

    assert!(
        scrubber
            .for_spawn(&create_spawn_with_label("//foo:bar", "Foo"))
            .is_some()
    );
    assert!(
        scrubber
            .for_spawn(&create_spawn_with_label("//foo:bar", "Bar"))
            .is_some()
    );
    assert!(
        scrubber
            .for_spawn(&create_spawn_with_label("//foo:bar", "Baz"))
            .is_none()
    );
}

#[test]
#[ignore = "needs ActionOwner.getTargetKind and ActionOwner.isBuildConfigurationForTool (missing in blaze_exec::actions::ActionOwner)"]
fn match_wildcard_mnemonic() {
    let scrubber = new_scrubber(config_with_matcher(mnemonic_matcher("Foo.*")));

    assert!(
        scrubber
            .for_spawn(&create_spawn_with_label("//foo:bar", "Foo"))
            .is_some()
    );
    assert!(
        scrubber
            .for_spawn(&create_spawn_with_label("//foo:bar", "Foobar"))
            .is_some()
    );
    assert!(
        scrubber
            .for_spawn(&create_spawn_with_label("//foo:bar", "Bar"))
            .is_none()
    );
}

#[test]
#[ignore = "needs ActionOwner.getTargetKind and ActionOwner.isBuildConfigurationForTool (missing in blaze_exec::actions::ActionOwner)"]
fn match_exact_label() {
    let scrubber = new_scrubber(config_with_matcher(label_matcher("//foo:bar")));

    assert!(
        scrubber
            .for_spawn(&create_spawn_with_label("//foo:bar", "Foo"))
            .is_some()
    );
    assert!(
        scrubber
            .for_spawn(&create_spawn_with_label("//foo:barbaz", "Foo"))
            .is_none()
    );
}

#[test]
#[ignore = "needs ActionOwner.getTargetKind and ActionOwner.isBuildConfigurationForTool (missing in blaze_exec::actions::ActionOwner)"]
fn match_union_label() {
    let scrubber = new_scrubber(config_with_matcher(label_matcher("//foo:bar|//spam:eggs")));

    assert!(
        scrubber
            .for_spawn(&create_spawn_with_label("//foo:bar", "Foo"))
            .is_some()
    );
    assert!(
        scrubber
            .for_spawn(&create_spawn_with_label("//spam:eggs", "Foo"))
            .is_some()
    );
    assert!(
        scrubber
            .for_spawn(&create_spawn_with_label("//quux:xyzzy", "Foo"))
            .is_none()
    );
}

#[test]
#[ignore = "needs ActionOwner.getTargetKind and ActionOwner.isBuildConfigurationForTool (missing in blaze_exec::actions::ActionOwner)"]
fn match_wildcard_label() {
    let scrubber = new_scrubber(config_with_matcher(label_matcher("//foo:.*")));

    assert!(
        scrubber
            .for_spawn(&create_spawn_with_label("//foo:bar", "Foo"))
            .is_some()
    );
    assert!(
        scrubber
            .for_spawn(&create_spawn_with_label("//foo:baz", "Foo"))
            .is_some()
    );
    assert!(
        scrubber
            .for_spawn(&create_spawn_with_label("//spam:eggs", "Foo"))
            .is_none()
    );
}

#[test]
#[ignore = "needs ActionOwner.getTargetKind and ActionOwner.isBuildConfigurationForTool (missing in blaze_exec::actions::ActionOwner)"]
fn match_exact_kind() {
    let scrubber = new_scrubber(config_with_matcher(kind_matcher("java_library")));

    assert!(
        scrubber
            .for_spawn(&create_spawn_with_kind(
                "//foo:bar",
                "Foo",
                "java_library",
                false
            ))
            .is_some()
    );
    assert!(
        scrubber
            .for_spawn(&create_spawn_with_kind(
                "//foo:barbaz",
                "Foo",
                "java_test",
                false
            ))
            .is_none()
    );
}

#[test]
#[ignore = "needs ActionOwner.getTargetKind and ActionOwner.isBuildConfigurationForTool (missing in blaze_exec::actions::ActionOwner)"]
fn match_union_kind() {
    let scrubber = new_scrubber(config_with_matcher(kind_matcher("java_library|java_test")));

    assert!(
        scrubber
            .for_spawn(&create_spawn_with_kind(
                "//foo:bar",
                "Foo",
                "java_library",
                false
            ))
            .is_some()
    );
    assert!(
        scrubber
            .for_spawn(&create_spawn_with_kind(
                "//spam:eggs",
                "Foo",
                "java_test",
                false
            ))
            .is_some()
    );
    assert!(
        scrubber
            .for_spawn(&create_spawn_with_kind(
                "//quux:xyzzy",
                "Foo",
                "go_library",
                false
            ))
            .is_none()
    );
}

#[test]
#[ignore = "needs ActionOwner.getTargetKind and ActionOwner.isBuildConfigurationForTool (missing in blaze_exec::actions::ActionOwner)"]
fn match_wildcard_kind() {
    let scrubber = new_scrubber(config_with_matcher(kind_matcher("java_.*")));

    assert!(
        scrubber
            .for_spawn(&create_spawn_with_kind(
                "//foo:bar",
                "Foo",
                "java_library",
                false
            ))
            .is_some()
    );
    assert!(
        scrubber
            .for_spawn(&create_spawn_with_kind(
                "//foo:baz",
                "Foo",
                "java_test",
                false
            ))
            .is_some()
    );
    assert!(
        scrubber
            .for_spawn(&create_spawn_with_kind(
                "//spam:eggs",
                "Foo",
                "go_library",
                false
            ))
            .is_none()
    );
}

#[test]
#[ignore = "needs ActionOwner.getTargetKind and ActionOwner.isBuildConfigurationForTool (missing in blaze_exec::actions::ActionOwner)"]
fn reject_tool_action() {
    let scrubber = new_scrubber(config_with_matcher(config::Matcher {
        label: "//foo:bar".to_string(),
        mnemonic: "Foo".to_string(),
        ..Default::default()
    }));

    assert!(
        scrubber
            .for_spawn(&create_spawn_with_label("//foo:bar", "Foo"))
            .is_some()
    );
    assert!(
        scrubber
            .for_spawn(&create_spawn_with_kind(
                "//foo:bar",
                "Foo",
                "java_library",
                /* for_tool= */ true
            ))
            .is_none()
    );
}

#[test]
#[ignore = "needs ActionOwner.getTargetKind and ActionOwner.isBuildConfigurationForTool (missing in blaze_exec::actions::ActionOwner)"]
fn accept_tool_action() {
    let scrubber = new_scrubber(config_with_matcher(config::Matcher {
        label: "//foo:bar".to_string(),
        mnemonic: "Foo".to_string(),
        match_tools: true,
        ..Default::default()
    }));

    assert!(
        scrubber
            .for_spawn(&create_spawn_with_label("//foo:bar", "Foo"))
            .is_some()
    );
    assert!(
        scrubber
            .for_spawn(&create_spawn_with_kind(
                "//foo:bar",
                "Foo",
                "java_library",
                /* for_tool= */ true
            ))
            .is_some()
    );
}

#[test]
#[ignore = "needs ActionOwner.getTargetKind and ActionOwner.isBuildConfigurationForTool (missing in blaze_exec::actions::ActionOwner)"]
fn no_omitted_inputs() {
    let scrubber = new_scrubber(Config {
        rules: vec![config::Rule::default()],
    });
    let spawn = create_spawn();
    let spawn_scrubber = scrubber.for_spawn(&spawn).unwrap();

    assert!(!spawn_scrubber.should_omit_input(&PathFragment::create("foo/bar")));
}

#[test]
#[ignore = "needs ActionOwner.getTargetKind and ActionOwner.isBuildConfigurationForTool (missing in blaze_exec::actions::ActionOwner)"]
fn exact_omitted_input() {
    let scrubber = new_scrubber(config_with_transform(omitted_inputs(&["foo/bar"])));
    let spawn = create_spawn();
    let spawn_scrubber = scrubber.for_spawn(&spawn).unwrap();

    assert!(spawn_scrubber.should_omit_input(&PathFragment::create("foo/bar")));
    assert!(!spawn_scrubber.should_omit_input(&PathFragment::create("foo/bar/baz")));
    assert!(!spawn_scrubber.should_omit_input(&PathFragment::create("bazel-out/foo/bar")));
}

#[test]
#[ignore = "needs ActionOwner.getTargetKind and ActionOwner.isBuildConfigurationForTool (missing in blaze_exec::actions::ActionOwner)"]
fn wildcard_omitted_input() {
    let scrubber = new_scrubber(config_with_transform(omitted_inputs(&["foo/bar.*"])));
    let spawn = create_spawn();
    let spawn_scrubber = scrubber.for_spawn(&spawn).unwrap();

    assert!(spawn_scrubber.should_omit_input(&PathFragment::create("foo/bar")));
    assert!(spawn_scrubber.should_omit_input(&PathFragment::create("foo/bar/baz")));
    assert!(!spawn_scrubber.should_omit_input(&PathFragment::create("bazel-out/foo/bar")));
}

#[test]
#[ignore = "needs ActionOwner.getTargetKind and ActionOwner.isBuildConfigurationForTool (missing in blaze_exec::actions::ActionOwner)"]
fn multiple_omitted_inputs() {
    let scrubber = new_scrubber(config_with_transform(omitted_inputs(&[
        "foo/bar",
        "spam/eggs",
    ])));
    let spawn = create_spawn();
    let spawn_scrubber = scrubber.for_spawn(&spawn).unwrap();

    assert!(spawn_scrubber.should_omit_input(&PathFragment::create("foo/bar")));
    assert!(spawn_scrubber.should_omit_input(&PathFragment::create("spam/eggs")));
    assert!(!spawn_scrubber.should_omit_input(&PathFragment::create("foo/bar/baz")));
    assert!(!spawn_scrubber.should_omit_input(&PathFragment::create("bazel-out/foo/bar")));
    assert!(!spawn_scrubber.should_omit_input(&PathFragment::create("spam/eggs/bacon")));
    assert!(!spawn_scrubber.should_omit_input(&PathFragment::create("bazel-out/spam/eggs")));
}

#[test]
#[ignore = "needs ActionOwner.getTargetKind and ActionOwner.isBuildConfigurationForTool (missing in blaze_exec::actions::ActionOwner)"]
fn simple_arg_replacement() {
    let scrubber = new_scrubber(config_with_transform(arg_replacements(vec![replacement(
        "foo", "bar",
    )])));
    let spawn = create_spawn();
    let spawn_scrubber = scrubber.for_spawn(&spawn).unwrap();

    assert_eq!(spawn_scrubber.transform_argument("foo"), "bar");
    assert_eq!(spawn_scrubber.transform_argument("abcfooxyz"), "abcbarxyz");
    assert_eq!(spawn_scrubber.transform_argument("bar"), "bar");
    assert_eq!(spawn_scrubber.transform_argument("foofoo"), "barfoo");
}

#[test]
#[ignore = "needs ActionOwner.getTargetKind and ActionOwner.isBuildConfigurationForTool (missing in blaze_exec::actions::ActionOwner)"]
fn anchored_arg_replacement() {
    let scrubber = new_scrubber(config_with_transform(arg_replacements(vec![replacement(
        "^foo$", "bar",
    )])));
    let spawn = create_spawn();
    let spawn_scrubber = scrubber.for_spawn(&spawn).unwrap();

    assert_eq!(spawn_scrubber.transform_argument("foo"), "bar");
    assert_eq!(spawn_scrubber.transform_argument("abcfooxyz"), "abcfooxyz");
}

#[test]
#[ignore = "needs ActionOwner.getTargetKind and ActionOwner.isBuildConfigurationForTool (missing in blaze_exec::actions::ActionOwner)"]
fn wildcard_arg_replacement() {
    let scrubber = new_scrubber(config_with_transform(arg_replacements(vec![replacement(
        "foo[12]", "bar",
    )])));
    let spawn = create_spawn();
    let spawn_scrubber = scrubber.for_spawn(&spawn).unwrap();

    assert_eq!(spawn_scrubber.transform_argument("foo1"), "bar");
    assert_eq!(spawn_scrubber.transform_argument("foo2"), "bar");
    assert_eq!(spawn_scrubber.transform_argument("foo3"), "foo3");
}

#[test]
#[ignore = "needs ActionOwner.getTargetKind and ActionOwner.isBuildConfigurationForTool (missing in blaze_exec::actions::ActionOwner)"]
fn multiple_arg_replacements() {
    let scrubber = new_scrubber(config_with_transform(arg_replacements(vec![
        replacement("foo", "bar"),
        replacement("spam", "eggs"),
    ])));
    let spawn = create_spawn();
    let spawn_scrubber = scrubber.for_spawn(&spawn).unwrap();

    assert_eq!(
        spawn_scrubber.transform_argument("abcfoo123spamxyz"),
        "abcbar123eggsxyz"
    );
    assert_eq!(spawn_scrubber.transform_argument("abcfoo"), "abcbar");
    assert_eq!(spawn_scrubber.transform_argument("abcspam"), "abceggs");
    assert_eq!(spawn_scrubber.transform_argument("bareggs"), "bareggs");
}

#[test]
#[ignore = "needs ActionOwner.getTargetKind and ActionOwner.isBuildConfigurationForTool (missing in blaze_exec::actions::ActionOwner)"]
fn without_salt() {
    let scrubber = new_scrubber(Config {
        rules: vec![config::Rule::default()],
    });
    let spawn = create_spawn();
    let spawn_scrubber = scrubber.for_spawn(&spawn).unwrap();

    assert!(spawn_scrubber.get_salt().is_empty());
}

#[test]
#[ignore = "needs ActionOwner.getTargetKind and ActionOwner.isBuildConfigurationForTool (missing in blaze_exec::actions::ActionOwner)"]
fn with_salt() {
    let scrubber = new_scrubber(config_with_transform(config::Transform {
        salt: "NaCl".to_string(),
        ..Default::default()
    }));
    let spawn = create_spawn();
    let spawn_scrubber = scrubber.for_spawn(&spawn).unwrap();

    assert_eq!(spawn_scrubber.get_salt(), "NaCl");
}

#[test]
#[ignore = "needs ActionOwner.getTargetKind and ActionOwner.isBuildConfigurationForTool (missing in blaze_exec::actions::ActionOwner)"]
fn orthogonal_rules() {
    let scrubber = new_scrubber(Config {
        rules: vec![
            config::Rule {
                matcher: Some(label_matcher("//foo:bar")),
                transform: Some(arg_replacements(vec![replacement("foo", "bar")])),
            },
            config::Rule {
                matcher: Some(label_matcher("//spam:eggs")),
                transform: Some(arg_replacements(vec![replacement("spam", "eggs")])),
            },
        ],
    });

    let foo_bar = create_spawn_with_label("//foo:bar", "Foo");
    let spawn_scrubber_for_foo_bar = scrubber.for_spawn(&foo_bar);
    assert!(spawn_scrubber_for_foo_bar.is_some());
    assert_eq!(
        spawn_scrubber_for_foo_bar
            .unwrap()
            .transform_argument("foospam"),
        "barspam"
    );

    let spam_eggs = create_spawn_with_label("//spam:eggs", "Spam");
    let spawn_scrubber_for_spam_eggs = scrubber.for_spawn(&spam_eggs);
    assert!(spawn_scrubber_for_spam_eggs.is_some());
    assert_eq!(
        spawn_scrubber_for_spam_eggs
            .unwrap()
            .transform_argument("foospam"),
        "fooeggs"
    );
}

#[test]
#[ignore = "needs ActionOwner.getTargetKind and ActionOwner.isBuildConfigurationForTool (missing in blaze_exec::actions::ActionOwner)"]
fn last_rule_wins() {
    let scrubber = new_scrubber(Config {
        rules: vec![
            config::Rule {
                matcher: None,
                transform: Some(arg_replacements(vec![replacement("foo", "bar")])),
            },
            config::Rule {
                matcher: None,
                transform: Some(arg_replacements(vec![replacement("spam", "eggs")])),
            },
        ],
    });
    let spawn = create_spawn();
    let spawn_scrubber = scrubber.for_spawn(&spawn).unwrap();

    assert_eq!(spawn_scrubber.transform_argument("foospam"), "fooeggs");
}
