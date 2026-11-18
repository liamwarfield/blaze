//! End-to-end tests: MODULE.bazel + .bzl files in a scratch workspace, fetched through the real
//! REPOSITORY_DIRECTORY / REPO_DEFINITION / SINGLE_EXTENSION_EVAL functions.
//!
//! Adapted from StarlarkRepositoryIntegrationTest and ModuleExtensionResolutionTest
//! (src/test/java/com/google/devtools/build/lib/bazel/repository/starlark and bazel/bzlmod):
//! Java builds configured targets from the fetched repos; these tests check the repos' files,
//! marker files and the lockfile instead, with expectations derived from the Java code.

mod common;

use blaze_cmdline::RepositoryName;
use blaze_sky_loading::repos::RepositoryMappingKey;
use blaze_skyframe::EvaluationContext;
use blaze_skyframe::SkyKey;
use common::Scratch;
use common::run;

/// The canonical name of `@apparent` as seen from the main repo.
fn canonical(s: &Scratch, ev: &blaze_repo_rules::setup::FetchEvaluator, apparent: &str) -> String {
    let key = RepositoryMappingKey::key(RepositoryName::create_unvalidated(""));
    let ctx = EvaluationContext {
        keep_going: true,
        event_handler: s.events.clone(),
        ..EvaluationContext::default()
    };
    let result = ev
        .evaluator
        .evaluate(&[SkyKey::new(key.clone())], &ctx)
        .unwrap();
    let v = result
        .get(&key)
        .unwrap_or_else(|| panic!("no main repo mapping: {:?}", s.events()));
    v.repository_mapping()
        .unwrap()
        .get(apparent)
        .get_name()
        .to_owned()
}

#[test]
fn starlark_repository_name_and_file() {
    // Java: StarlarkRepositoryIntegrationTest.testStarlarkRepositoryName (repository_ctx.name,
    // attr, file, template)
    run(|| {
        let s = Scratch::new();
        s.file(
            "MODULE.bazel",
            "my_rule = use_repo_rule('//:rule.bzl', 'my_rule')\n\
             my_rule(name = 'foo', greeting = 'hello')\n",
        );
        s.file("BUILD", "");
        s.file("tpl.txt", "%GREETING% from %NAME%\n");
        s.file(
            "rule.bzl",
            "def _impl(rctx):\n\
             \x20 rctx.file('BUILD', 'filegroup(name = \"bar\")\\n')\n\
             \x20 rctx.file('name.txt', rctx.name + '|' + rctx.original_name + '|' + rctx.attr.greeting)\n\
             \x20 rctx.template('t.txt', rctx.attr._tpl, {'%GREETING%': rctx.attr.greeting, '%NAME%': rctx.original_name}, executable = True)\n\
             my_rule = repository_rule(\n\
             \x20 implementation = _impl,\n\
             \x20 attrs = {'greeting': attr.string(), '_tpl': attr.label(default = Label('//:tpl.txt'))},\n\
             )\n",
        );
        let ev = s.evaluator(&[]);
        let name = canonical(&s, &ev, "foo");
        assert_eq!(name, "+my_rule+foo");
        let dir = s
            .fetch(&ev, &name)
            .unwrap_or_else(|e| panic!("{e}\n{:?}", s.events()));
        assert_eq!(s.read(&dir.join("name.txt")), "+my_rule+foo|foo|hello");
        assert_eq!(s.read(&dir.join("t.txt")), "hello from foo\n");
        assert!(dir.join("REPO.bazel").exists(), "boundary file is created");
        let marker = s.read(&s.output_base.join("external/@+my_rule+foo.marker"));
        let lines: Vec<&str> = marker.lines().collect();
        assert_eq!(lines[0].len(), 64, "predeclared input hash: {marker}");
        assert!(
            lines.iter().any(|l| l.starts_with("FILE:@@//tpl.txt ")),
            "template is watched: {marker}"
        );
    })
}

#[test]
fn starlark_repository_cannot_override_built_in_attribute() {
    // Java: StarlarkRepositoryIntegrationTest.testStarlarkRepositoryCannotOverrideBuiltInAttribute
    run(|| {
        let s = Scratch::new();
        s.file(
            "MODULE.bazel",
            "my_rule = use_repo_rule('//:rule.bzl', 'my_rule')\nmy_rule(name = 'foo')\n",
        );
        s.file("BUILD", "");
        s.file(
            "rule.bzl",
            "def _impl(rctx):\n  pass\n\
             my_rule = repository_rule(implementation = _impl, attrs = {'name': attr.string()})\n",
        );
        let ev = s.evaluator(&[]);
        let key = RepositoryMappingKey::key(RepositoryName::create_unvalidated(""));
        let ctx = EvaluationContext {
            keep_going: true,
            event_handler: s.events.clone(),
            ..EvaluationContext::default()
        };
        let _ = ev.evaluator.evaluate(&[SkyKey::new(key)], &ctx);
        let _ = s.fetch(&ev, "+my_rule+foo");
        let events = s.events().join("\n");
        assert!(
            events.contains(
                "There is already a built-in attribute 'name' which cannot be overridden"
            ),
            "{events}"
        );
    })
}

#[test]
fn module_extension_generates_repos_and_lockfile_entry() {
    // Java: ModuleExtensionResolutionTest.simpleExtension (adapted: checks the generated repo's
    // files and the lockfile instead of building a target).
    run(|| {
        let s = Scratch::new();
        s.file(
            "MODULE.bazel",
            "ext = use_extension('//:defs.bzl', 'ext')\n\
             ext.tag(name = 'foo', data = 'fu')\n\
             ext.tag(name = 'bar', data = 'ba')\n\
             use_repo(ext, 'foo', 'bar')\n",
        );
        s.file("BUILD", "");
        s.file(
            "defs.bzl",
            "def _data_repo_impl(rctx):\n\
             \x20 rctx.file('BUILD', 'filegroup(name = \"x\")')\n\
             \x20 rctx.file('data', rctx.attr.data)\n\
             data_repo = repository_rule(implementation = _data_repo_impl, attrs = {'data': attr.string()})\n\
             def _ext_impl(mctx):\n\
             \x20 for mod in mctx.modules:\n\
             \x20   for tag in mod.tags.tag:\n\
             \x20     data_repo(name = tag.name, data = tag.data)\n\
             tag = tag_class(attrs = {'name': attr.string(), 'data': attr.string()})\n\
             ext = module_extension(implementation = _ext_impl, tag_classes = {'tag': tag})\n",
        );
        let ev = s.evaluator(&[]);
        let foo = canonical(&s, &ev, "foo");
        assert_eq!(foo, "+ext+foo");
        let dir = s
            .fetch(&ev, &foo)
            .unwrap_or_else(|e| panic!("{e}\n{:?}", s.events()));
        assert_eq!(s.read(&dir.join("data")), "fu");
        let bar = s.fetch(&ev, "+ext+bar").unwrap();
        assert_eq!(s.read(&bar.join("data")), "ba");
        s.write_lockfile(&ev);
        let lock = s.read(&s.workspace.join("MODULE.bazel.lock"));
        assert!(lock.contains("\"//:defs.bzl%ext\""), "{lock}");
        assert!(lock.contains("\"generatedRepoSpecs\""), "{lock}");
        assert!(lock.contains("\"data\": \"fu\""), "{lock}");
    })
}

/// `include()` in the root MODULE.bazel resolves its package through the real PACKAGE_LOOKUP
/// (review 2026-11-18, S2-15): a `.bazelignore` in the workspace is honoured rather than refused,
/// and an included file under an ignored directory is not in a package.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleFileFunction.java:466 ModuleFileFunction.advanceHorizon
#[test]
fn include_uses_package_lookup_with_bazelignore() {
    fn workspace(s: &Scratch, include: &str) {
        s.file(
            "MODULE.bazel",
            &format!(
                "module(name = 'root')\n\
                 include('{include}')\n"
            ),
        );
        s.file("BUILD", "");
        s.file(".bazelignore", "ignored\n");
        s.file("sub/BUILD", "");
        s.file(
            "sub/extra.MODULE.bazel",
            "my_rule = use_repo_rule('//:rule.bzl', 'my_rule')\n\
             my_rule(name = 'foo')\n",
        );
        s.file("ignored/BUILD", "");
        s.file("ignored/extra.MODULE.bazel", "");
        s.file(
            "rule.bzl",
            "def _impl(rctx):\n\
             \x20 rctx.file('BUILD', '')\n\
             my_rule = repository_rule(implementation = _impl)\n",
        );
    }
    run(|| {
        let s = Scratch::new();
        workspace(&s, "//sub:extra.MODULE.bazel");
        let ev = s.evaluator(&[]);
        assert_eq!(canonical(&s, &ev, "foo"), "+my_rule+foo");
    });
    run(|| {
        let s = Scratch::new();
        workspace(&s, "//ignored:extra.MODULE.bazel");
        let ev = s.evaluator(&[]);
        let key = SkyKey::new(RepositoryMappingKey::key(
            RepositoryName::create_unvalidated(""),
        ));
        let ctx = EvaluationContext {
            keep_going: true,
            event_handler: s.events.clone(),
            ..EvaluationContext::default()
        };
        let result = ev
            .evaluator
            .evaluate(std::slice::from_ref(&key), &ctx)
            .unwrap();
        let error = result
            .get_error(&key)
            .unwrap_or_else(|| panic!("expected an error: {:?}", s.events()));
        let message = error
            .get_exception()
            .map(|x| x.to_string())
            .unwrap_or_else(|| format!("{error:?}"));
        // errorf(Code.BAD_MODULE, "unable to load package for '%s' included at %s: %s", ...)
        // with PackageLookupValue.DELETED_PACKAGE_VALUE.getErrorMsg().
        assert!(
            message
                .contains("unable to load package for '//ignored:extra.MODULE.bazel' included at ")
                && message.ends_with(": Package is considered deleted due to --deleted_packages"),
            "{message}"
        );
    });
}
