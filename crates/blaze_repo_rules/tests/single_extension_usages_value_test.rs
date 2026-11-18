//! Tests of `SingleExtensionUsagesValue.hashForEvaluation` (the lockfile's `usagesDigest`).
//!
//! There is no JUnit test class for `SingleExtensionUsagesValue`; the expected digests come from
//! the oracle: `~/.cache/blaze/oracle-records/oracle-9.3.2/re2/raw/MODULE.bazel.lock`
//! (`moduleExtensions[*].general.usagesDigest`), with the usages taken from the registry's
//! MODULE.bazel files of rules_kotlin 1.9.6 and rules_python 1.7.0.

use base64::Engine;
use blaze_bzlmod_model::AbridgedModule;
use blaze_bzlmod_model::AttributeValue;
use blaze_bzlmod_model::AttributeValues;
use blaze_bzlmod_model::ModuleExtensionUsage;
use blaze_bzlmod_model::ModuleExtensionUsageProxy;
use blaze_bzlmod_model::ModuleKey;
use blaze_bzlmod_model::Tag;
use blaze_bzlmod_model::Version;
use blaze_cmdline::RepositoryMapping;
use blaze_cmdline::RepositoryName;
use blaze_events::Location;
use blaze_repo_rules::bzlmod::single_extension_usages::SingleExtensionUsagesValue;
use blaze_vfs::PathFragment;
use indexmap::IndexMap;

fn digest(value: &SingleExtensionUsagesValue) -> String {
    base64::engine::general_purpose::STANDARD
        .encode(SingleExtensionUsagesValue::hash_for_evaluation(value))
}

fn module(name: &str, version: &str) -> (ModuleKey, AbridgedModule) {
    let version = Version::parse(version).unwrap();
    let key = ModuleKey::new(name, version.clone());
    (
        key.clone(),
        AbridgedModule {
            name: name.to_owned(),
            version,
            key,
        },
    )
}

fn proxy(file: &str, line: i32, column: i32, imports: &[&str]) -> ModuleExtensionUsageProxy {
    ModuleExtensionUsageProxy {
        location: Location::new(file, line, column),
        proxy_name: "ext".to_owned(),
        containing_module_file_path: PathFragment::create("MODULE.bazel"),
        dev_dependency: false,
        imports: imports
            .iter()
            .map(|i| (i.to_string(), i.to_string()))
            .collect(),
    }
}

fn usages_value(
    key: ModuleKey,
    abridged: AbridgedModule,
    usage: ModuleExtensionUsage,
    unique_name: &str,
) -> SingleExtensionUsagesValue {
    let mapping = RepositoryMapping::create(
        [(
            "bazel_tools",
            RepositoryName::create_unvalidated("bazel_tools"),
        )],
        RepositoryName::create_unvalidated(&format!("{}+", abridged.name)),
    );
    SingleExtensionUsagesValue::create(
        [(key.clone(), usage)].into_iter().collect(),
        unique_name.to_owned(),
        vec![abridged],
        // Dropped by trimForEvaluation.
        [(key, mapping)].into_iter().collect(),
        IndexMap::new(),
    )
}

#[test]
fn hash_for_evaluation_without_tags() {
    let (key, abridged) = module("rules_kotlin", "1.9.6");
    let usage = ModuleExtensionUsage {
        extension_bzl_file: "@rules_kotlin//src/main/starlark/core/repositories:bzlmod_setup.bzl"
            .to_owned(),
        extension_name: "rules_kotlin_extensions".to_owned(),
        isolation_key: None,
        // Dropped by trimForEvaluation.
        proxies: vec![proxy(
            "/out/external/rules_kotlin+/MODULE.bazel",
            14,
            27,
            &["com_github_google_ksp", "rules_android"],
        )],
        tags: Vec::new(),
        repo_overrides: IndexMap::new(),
    };
    let value = usages_value(
        key,
        abridged,
        usage,
        "rules_kotlin++rules_kotlin_extensions",
    );
    assert_eq!(
        SingleExtensionUsagesValue::to_json(&value.trim_for_evaluation()),
        "{\"extensionUsages\":{\"rules_kotlin@1.9.6\":{\"extensionBzlFile\":\
         \"@rules_kotlin//src/main/starlark/core/repositories:bzlmod_setup.bzl\",\
         \"extensionName\":\"rules_kotlin_extensions\",\"proxies\":[],\"tags\":[],\
         \"repoOverrides\":{}}},\"extensionUniqueName\":\"rules_kotlin++rules_kotlin_extensions\",\
         \"abridgedModules\":[{\"name\":\"rules_kotlin\",\"version\":\"1.9.6\",\
         \"key\":\"rules_kotlin@1.9.6\"}],\"repoMappings\":{},\"repoOverrides\":{}}"
    );
    assert_eq!(
        digest(&value),
        "QI2z8ZUR+mqtbwsf2fLqYdJAkPOHdOV+tF2yVAUgRzw="
    );
}

#[test]
fn hash_for_evaluation_with_tags() {
    let (key, abridged) = module("rules_python", "1.7.0");
    let mut tags = vec![Tag {
        tag_name: "default".to_owned(),
        attribute_values: AttributeValues::create([
            (
                "base_url",
                AttributeValue::string("https://github.com/astral-sh/uv/releases/download"),
            ),
            (
                "manifest_filename",
                AttributeValue::string("dist-manifest.json"),
            ),
            ("version", AttributeValue::string("0.6.3")),
        ]),
        dev_dependency: false,
        location: Location::new("/out/external/rules_python+/MODULE.bazel", 386, 11),
    }];
    let platforms = [
        ("macos", "aarch64", "aarch64-apple-darwin"),
        ("linux", "aarch64", "aarch64-unknown-linux-gnu"),
        ("linux", "ppc", "powerpc64-unknown-linux-gnu"),
        ("linux", "ppc64le", "powerpc64le-unknown-linux-gnu"),
        ("linux", "s390x", "s390x-unknown-linux-gnu"),
        ("linux", "riscv64", "riscv64-unknown-linux-gnu"),
        ("macos", "x86_64", "x86_64-apple-darwin"),
        ("windows", "x86_64", "x86_64-pc-windows-msvc"),
        ("linux", "x86_64", "x86_64-unknown-linux-gnu"),
    ];
    for (i, (os, cpu, platform)) in platforms.iter().enumerate() {
        tags.push(Tag {
            tag_name: "default".to_owned(),
            attribute_values: AttributeValues::create([
                (
                    "compatible_with",
                    AttributeValue::string_list([
                        format!("@platforms//os:{os}"),
                        format!("@platforms//cpu:{cpu}"),
                    ]),
                ),
                ("platform", AttributeValue::string(*platform)),
            ]),
            dev_dependency: false,
            location: Location::new(
                "/out/external/rules_python+/MODULE.bazel",
                400 + i as i32,
                11,
            ),
        });
    }
    let usage = ModuleExtensionUsage {
        extension_bzl_file: "@rules_python//python/uv:uv.bzl".to_owned(),
        extension_name: "uv".to_owned(),
        isolation_key: None,
        proxies: vec![proxy(
            "/out/external/rules_python+/MODULE.bazel",
            380,
            21,
            &[],
        )],
        tags,
        repo_overrides: IndexMap::new(),
    };
    let value = usages_value(key, abridged, usage, "rules_python++uv");
    assert_eq!(
        digest(&value),
        "H8dQoNZcoqP+Mu0tHZTi4KHATzvNkM5ePuEqoQdklIU="
    );
}
