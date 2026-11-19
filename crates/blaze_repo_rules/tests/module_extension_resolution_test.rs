//! Port of `ModuleExtensionResolutionTest`
//! (src/test/java/com/google/devtools/build/lib/bazel/bzlmod), a `BuildViewTestCase`: module
//! files, a fake registry, .bzl evaluation through BZL_LOAD and SINGLE_EXTENSION(_EVAL), on the
//! analysis test harness of `blaze_sky_top::testing`.
//!
//! The test bodies were transcribed from the Java statement by statement; tests whose Java uses
//! something the harness does not offer stay ignored with the reason.

use std::sync::Arc;

use blaze_bzlmod::skyframe::keys::{SingleExtensionKey, SingleExtensionValue};
use blaze_bzlmod_model::LockfileModuleExtensionMetadata;
use blaze_bzlmod_model::ModuleExtensionId;
use blaze_bzlmod_model::ModuleExtensionUsage;
use blaze_bzlmod_model::ModuleExtensionUsageProxy;
use blaze_bzlmod_model::UseAllRepos;
use blaze_bzlmod_model::{IsolationKey, ModuleKey};
use blaze_cmdline::Label;
use blaze_events::{EventKind, EventKindSet, Location};
use blaze_repo_rules::bzlmod::module_extension_metadata::generate_fixup;
use blaze_repo_rules::precomputed::RequireRepoExtensionMetadataMode;
use blaze_sky_loading::bzl_load::BzlLoadKey;
use blaze_sky_top::testing::{BuildViewTestCase, InjectedPrecomputedValue, create_module_key};
use blaze_skyframe::{EvaluationResult, Key, SkyKey};
use blaze_vfs::PathFragment;
use indexmap::IndexMap;
use indexmap::IndexSet;

/// A `BuildViewTestCase` after the class's `@Before setUpSimpleRepoRule()`.
// Java: ModuleExtensionResolutionTest#setUpSimpleRepoRule
fn new_test() -> BuildViewTestCase {
    let t = BuildViewTestCase::new();
    // Set up a simple repo rule.
    t.registry().add_module(
        &create_module_key("data_repo", "1.0"),
        &["module(name='data_repo',version='1.0')"],
    );
    t.scratch
        .file(&module_path(&t, "data_repo+1.0/REPO.bazel"), &[]);
    t.scratch.file(&module_path(&t, "data_repo+1.0/BUILD"), &[]);
    t.scratch.file(
        &module_path(&t, "data_repo+1.0/defs.bzl"),
        &[
            "def _data_repo_impl(ctx):",
            "  ctx.file('BUILD')",
            "  ctx.file('data.bzl', 'data = '+json.encode(ctx.attr.data))",
            "  ctx.file(",
            "    'names.bzl',",
            "    'names='+json.encode({",
            "      'name': ctx.name,",
            "      'original_name': ctx.original_name,",
            "    })",
            "  )",
            "data_repo = repository_rule(",
            "  implementation=_data_repo_impl,",
            "  attrs={'data':attr.string()})",
        ],
    );
    t
}

/// `moduleRoot.getRelative(path).getPathString()`.
fn module_path(t: &BuildViewTestCase, path: &str) -> String {
    t.module_root
        .get_relative(&PathFragment::create(path))
        .get_path_string()
        .to_string()
}

/// An `EvaluationResult` of one key (`SkyframeExecutorTestUtils.evaluate(skyframeExecutor, key,
/// false, reporter)`).
struct Evaluated<K: Key> {
    result: EvaluationResult,
    key: K,
}

impl<K: Key + Clone> Evaluated<K> {
    fn has_error(&self) -> bool {
        self.result.has_error()
    }

    /// `result.getError().getException().getMessage()`.
    fn error_message(&self) -> String {
        let error = self.result.get_first_error().expect("an error");
        error
            .get_exception()
            .unwrap_or_else(|| panic!("no exception: {error:?}"))
            .to_string()
    }

    /// `!result.getError().getCycleInfo().isEmpty()`.
    #[allow(dead_code)]
    fn has_cycle_info(&self) -> bool {
        !self
            .result
            .get_first_error()
            .expect("an error")
            .get_cycle_info()
            .is_empty()
    }

    /// `if (result.hasError()) { throw result.getError().getException(); }`.
    fn throw_if_error(&self) {
        if self.has_error() {
            let error = self.result.get_first_error().expect("an error");
            match error.get_exception() {
                Some(e) => panic!("{e}"),
                None => panic!("{error:?}"),
            }
        }
    }

    fn get(&self) -> Arc<K::Value> {
        self.result
            .get(&self.key)
            .unwrap_or_else(|| panic!("no value for {:?}", self.key))
    }
}

impl Evaluated<BzlLoadKey> {
    /// `result.get(skyKey).getModule().getGlobal(name)` as a string.
    fn global_str(&self, name: &str) -> String {
        let value = self.get();
        let global = value
            .get_module()
            .get(name)
            .unwrap_or_else(|e| panic!("no global {name}: {e}"));
        global
            .value()
            .unpack_str()
            .unwrap_or_else(|| panic!("global {name} is not a string: {}", global.value()))
            .to_string()
    }

    /// `result.get(skyKey).getModule().getGlobal(name)`.
    fn global_value(&self, name: &str) -> starlark::values::OwnedFrozenValue {
        self.get()
            .get_module()
            .get(name)
            .unwrap_or_else(|e| panic!("no global {name}: {e}"))
    }

    /// `(List<?>) result.get(skyKey).getModule().getGlobal(name)` of strings.
    fn global_str_list(&self, name: &str) -> Vec<String> {
        let value = self.get();
        let global = value
            .get_module()
            .get(name)
            .unwrap_or_else(|e| panic!("no global {name}: {e}"));
        let list = starlark::values::list::ListRef::from_value(global.value())
            .unwrap_or_else(|| panic!("global {name} is not a list: {}", global.value()));
        list.iter()
            .map(|v| v.unpack_str().expect("a string").to_string())
            .collect()
    }
}

/// `SkyframeExecutorTestUtils.evaluate(skyframeExecutor,
/// BzlLoadValue.keyForBuild(Label.parseCanonical(label)), false, reporter)`.
fn evaluate_bzl(t: &mut BuildViewTestCase, label: &str) -> Evaluated<BzlLoadKey> {
    let key = BzlLoadKey::key_for_build(
        Label::parse_canonical(label).unwrap_or_else(|e| panic!("{e:?}")),
    );
    let result = t.evaluate(&[SkyKey::new(key.clone())], false);
    Evaluated { result, key }
}

/// Java `String.indent(n)`: every line indented by `n` spaces, each ending with `\n`.
fn indent(s: &str, n: usize) -> String {
    if s.is_empty() {
        return String::new();
    }
    let pad = " ".repeat(n);
    s.lines().map(|l| format!("{pad}{l}\n")).collect()
}

// Java: ModuleExtensionResolutionTest#evaluateSimpleModuleExtension
fn evaluate_simple_module_extension(
    t: &mut BuildViewTestCase,
    return_statement: &str,
    dev_dependency: bool,
) -> Evaluated<SingleExtensionKey> {
    let dev_dependency_str = if dev_dependency { "True" } else { "False" };
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[&format!(
            "ext = use_extension('//:defs.bzl', 'ext', dev_dependency = {dev_dependency_str})"
        )],
    );
    t.scratch.file(
        "defs.bzl",
        &[
            "repo = repository_rule(lambda ctx: True)",
            "def _ext_impl(ctx):",
            "  repo(name = 'dep1')",
            "  repo(name = 'dep2')",
            &indent(return_statement, 2),
            "ext = module_extension(implementation=_ext_impl)",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.invalidate_packages();

    let extension_id = ModuleExtensionId::create(
        Label::parse_canonical("//:defs.bzl").unwrap_or_else(|e| panic!("{e:?}")),
        "ext",
        None,
    );
    t.remove_fail_fast_handler();
    t.invalidate_packages();
    let key = SingleExtensionValue::key(extension_id);
    let result = t.evaluate(&[SkyKey::new(key.clone())], false);
    Evaluated { result, key }
}

/// `(SingleExtensionValue) skyframeExecutor.getEvaluator().getDoneValues().get(
/// SingleExtensionValue.evalKey(ModuleExtensionId.create(Label.parseCanonical(label), name,
/// Optional.empty())))`.
fn done_eval_value(
    t: &mut BuildViewTestCase,
    label: &str,
    name: &str,
) -> Arc<SingleExtensionValue> {
    let key = SingleExtensionValue::eval_key(ModuleExtensionId::create(
        Label::parse_canonical(label).unwrap_or_else(|e| panic!("{e:?}")),
        name,
        None,
    ));
    t.get_evaluator()
        .get_existing_value(&SkyKey::new(key))
        .expect("a done SINGLE_EXTENSION_EVAL value")
        .downcast::<SingleExtensionValue>()
        .unwrap_or_else(|_| panic!("not a SingleExtensionValue"))
}

/// Truth `assertThat(fixup.moduleFilePathToBuildozerCommands()).containsExactly(k1, v1, ...)`
/// on a multimap: the same entries, in any order.
fn assert_buildozer_commands(value: &SingleExtensionValue, expected: &[(&str, &str)]) {
    let fixup = value.fixup.as_ref().expect("a fixup");
    let mut actual: Vec<(String, String)> = fixup
        .module_file_path_to_buildozer_commands
        .iter()
        .flat_map(|(path, commands)| {
            commands
                .iter()
                .map(move |c| (path.get_path_string().to_string(), c.clone()))
        })
        .collect();
    let mut expected: Vec<(String, String)> = expected
        .iter()
        .map(|(p, c)| (p.to_string(), c.to_string()))
        .collect();
    actual.sort();
    expected.sort();
    assert_eq!(actual, expected);
}

/// `MoreAsserts.assertEventCount(n, eventCollector)`.
fn assert_event_count(t: &BuildViewTestCase, n: i32) {
    assert_eq!(
        t.event_collector.count(),
        n,
        "{:?}",
        t.event_collector.iter().collect::<Vec<_>>()
    );
}

/// Truth `assertThat(actual).contains(expected)` on a string.
fn assert_contains(actual: &str, expected: &str) {
    assert!(
        actual.contains(expected),
        "<{actual}> does not contain <{expected}>"
    );
}

fn warning_kinds() -> EventKindSet {
    EventKindSet::of(&[EventKind::Warning])
}

// Java: ModuleExtensionResolutionTest#setRequireRepoExtensionMetadataMode
fn set_require_repo_extension_metadata_mode(
    t: &mut BuildViewTestCase,
    mode: RequireRepoExtensionMetadataMode,
) {
    t.inject_extra_precomputed_values(&[InjectedPrecomputedValue::RequireRepoExtensionMetadata(
        mode,
    )]);
}

// Java: ModuleExtensionResolutionTest#setupRootUseRepoRuleReturning
fn setup_root_use_repo_rule_returning(t: &mut BuildViewTestCase, return_statement: &str) {
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "repo = use_repo_rule('//:repo.bzl', 'repo')",
            "repo(name = 'foo')",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &[
            "load('@foo//:data.bzl', repo_data = 'data')",
            "data = repo_data",
        ],
    );
    t.scratch.file(
        "repo.bzl",
        &[
            "def _repo_impl(ctx):",
            "  ctx.file('BUILD')",
            "  ctx.file('data.bzl', \"data = 'ok'\")",
            &indent(return_statement, 2),
            "repo = repository_rule(implementation = _repo_impl)",
        ],
    );
    t.invalidate_packages();
}

/// `SkyframeExecutorTestUtils.evaluate(skyframeExecutor, SingleExtensionValue.key(
/// ModuleExtensionId.create(Label.parseCanonical(label), name, Optional.empty())), false,
/// reporter)`.
fn evaluate_extension(
    t: &mut BuildViewTestCase,
    label: &str,
    name: &str,
) -> Evaluated<SingleExtensionKey> {
    let key = SingleExtensionValue::key(ModuleExtensionId::create(
        Label::parse_canonical(label).unwrap_or_else(|e| panic!("{e:?}")),
        name,
        None,
    ));
    let result = t.evaluate(&[SkyKey::new(key.clone())], false);
    Evaluated { result, key }
}

/// [`done_eval_value`] with `Optional.of(ModuleExtensionId.IsolationKey.create(ModuleKey.ROOT,
/// usage_exported_name))`.
fn done_isolated_eval_value(
    t: &mut BuildViewTestCase,
    label: &str,
    name: &str,
    usage_exported_name: &str,
) -> Arc<SingleExtensionValue> {
    let key = SingleExtensionValue::eval_key(ModuleExtensionId::create(
        Label::parse_canonical(label).unwrap_or_else(|e| panic!("{e:?}")),
        name,
        Some(IsolationKey::create(ModuleKey::root(), usage_exported_name)),
    ));
    t.get_evaluator()
        .get_existing_value(&SkyKey::new(key))
        .expect("a done SINGLE_EXTENSION_EVAL value")
        .downcast::<SingleExtensionValue>()
        .unwrap_or_else(|_| panic!("not a SingleExtensionValue"))
}

// Java: ModuleExtensionResolutionTest.simpleExtension
#[test]
fn simple_extension() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "bazel_dep(name='data_repo', version='1.0')",
            "ext = use_extension('//:defs.bzl', 'ext')",
            "ext.tag(name='foo', data='fu')",
            "ext.tag(name='bar', data='ba')",
            "use_repo(ext, 'foo', 'bar')",
        ],
    );
    t.scratch.file("defs.bzl", &["load('@data_repo//:defs.bzl','data_repo')", "tag = tag_class(attrs = {'name':attr.string(),'data':attr.string()})", "def _ext_impl(ctx):", "  for mod in ctx.modules:", "    for tag in mod.tags.tag:", "      data_repo(name=tag.name,data=tag.data)", "ext = module_extension(implementation=_ext_impl, tag_classes={'tag':tag}, os_dependent=True, arch_dependent=True)"]);
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &[
            "load('@foo//:data.bzl', foo_data='data')",
            "load('@foo//:names.bzl', foo_names='names')",
            "load('@bar//:data.bzl', bar_data='data')",
            "load('@bar//:names.bzl', bar_names='names')",
            "data = 'foo:'+foo_data+' bar:'+bar_data",
            "names = 'foo:'+foo_names['name']+' bar:'+bar_names['name']",
            "original_names = 'foo:'+foo_names['original_name']+' bar:'+bar_names['original_name']",
        ],
    );
    t.invalidate_packages();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    result.throw_if_error();
    assert_eq!(result.global_str("data"), "foo:fu bar:ba");
    assert_eq!(result.global_str("names"), "foo:+ext+foo bar:+ext+bar");
    assert_eq!(result.global_str("original_names"), "foo:foo bar:bar");
}

// Java: ModuleExtensionResolutionTest.simpleExtension_nonCanonicalLabel
#[test]
fn simple_extension_non_canonical_label() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "module(name='my_module', version = '1.0')",
            "bazel_dep(name='data_repo', version='1.0')",
            "ext1 = use_extension('//:defs.bzl', 'ext')",
            "ext1.tag(name='foo', data='fu')",
            "use_repo(ext1, 'foo')",
            "ext2 = use_extension('@my_module//:defs.bzl', 'ext')",
            "ext2.tag(name='bar', data='ba')",
            "use_repo(ext2, 'bar')",
            "ext3 = use_extension('@//:defs.bzl', 'ext')",
            "ext3.tag(name='quz', data='qu')",
            "use_repo(ext3, 'quz')",
            "ext4 = use_extension('defs.bzl', 'ext')",
            "ext4.tag(name='qor', data='qo')",
            "use_repo(ext4, 'qor')",
        ],
    );
    t.scratch.file(
        "defs.bzl",
        &[
            "load('@data_repo//:defs.bzl','data_repo')",
            "tag = tag_class(attrs = {'name':attr.string(),'data':attr.string()})",
            "def _ext_impl(ctx):",
            "  for mod in ctx.modules:",
            "    for tag in mod.tags.tag:",
            "      data_repo(name=tag.name,data=tag.data)",
            "ext = module_extension(implementation=_ext_impl, tag_classes={'tag':tag})",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &[
            "load('@foo//:data.bzl', foo_data='data')",
            "load('@bar//:data.bzl', bar_data='data')",
            "load('@quz//:data.bzl', quz_data='data')",
            "load('@qor//:data.bzl', qor_data='data')",
            "data = 'foo:'+foo_data+' bar:'+bar_data+' quz:'+quz_data+' qor:'+qor_data",
        ],
    );
    t.invalidate_packages();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    result.throw_if_error();
    assert_eq!(result.global_str("data"), "foo:fu bar:ba quz:qu qor:qo");
}

// Java: ModuleExtensionResolutionTest.simpleExtension_nonCanonicalLabel_repoName
#[test]
fn simple_extension_non_canonical_label_repo_name() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "module(name='my_module', version = '1.0', repo_name='my_name')",
            "bazel_dep(name='data_repo', version='1.0')",
            "ext1 = use_extension('//:defs.bzl', 'ext')",
            "ext1.tag(name='foo', data='fu')",
            "use_repo(ext1, 'foo')",
            "ext2 = use_extension('@my_name//:defs.bzl', 'ext')",
            "ext2.tag(name='bar', data='ba')",
            "use_repo(ext2, 'bar')",
            "ext3 = use_extension('@//:defs.bzl', 'ext')",
            "ext3.tag(name='quz', data='qu')",
            "use_repo(ext3, 'quz')",
        ],
    );
    t.scratch.file(
        "defs.bzl",
        &[
            "load('@data_repo//:defs.bzl','data_repo')",
            "tag = tag_class(attrs = {'name':attr.string(),'data':attr.string()})",
            "def _ext_impl(ctx):",
            "  for mod in ctx.modules:",
            "    for tag in mod.tags.tag:",
            "      data_repo(name=tag.name,data=tag.data)",
            "ext = module_extension(implementation=_ext_impl, tag_classes={'tag':tag})",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &[
            "load('@foo//:data.bzl', foo_data='data')",
            "load('@bar//:data.bzl', bar_data='data')",
            "load('@quz//:data.bzl', quz_data='data')",
            "data = 'foo:'+foo_data+' bar:'+bar_data+' quz:'+quz_data",
        ],
    );
    t.invalidate_packages();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    result.throw_if_error();
    assert_eq!(result.global_str("data"), "foo:fu bar:ba quz:qu");
}

// Java: ModuleExtensionResolutionTest.multipleExtensions_sameName
#[test]
fn multiple_extensions_same_name() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "bazel_dep(name='data_repo', version='1.0')",
            "first_ext = use_extension('//first_ext:defs.bzl', 'ext')",
            "first_ext.tag(name='foo', data='first_fu')",
            "first_ext.tag(name='bar', data='first_ba')",
            "use_repo(first_ext, first_foo='foo', first_bar='bar')",
            "second_ext = use_extension('//second_ext:defs.bzl', 'ext')",
            "second_ext.tag(name='foo', data='second_fu')",
            "second_ext.tag(name='bar', data='second_ba')",
            "use_repo(second_ext, second_foo='foo', second_bar='bar')",
        ],
    );
    t.scratch.file("first_ext/BUILD", &[]);
    t.scratch.file(
        "first_ext/defs.bzl",
        &[
            "load('@data_repo//:defs.bzl','data_repo')",
            "tag = tag_class(attrs = {'name':attr.string(),'data':attr.string()})",
            "def _ext_impl(ctx):",
            "  for mod in ctx.modules:",
            "    for tag in mod.tags.tag:",
            "      data_repo(name=tag.name,data=tag.data)",
            "ext = module_extension(implementation=_ext_impl, tag_classes={'tag':tag})",
        ],
    );
    t.scratch.file("second_ext/BUILD", &[]);
    t.scratch.file(
        "second_ext/defs.bzl",
        &["load('//first_ext:defs.bzl', _ext = 'ext')", "ext = _ext"],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file("data.bzl", &["load('@first_foo//:data.bzl', first_foo_data='data')", "load('@first_bar//:data.bzl', first_bar_data='data')", "load('@second_foo//:data.bzl', second_foo_data='data')", "load('@second_bar//:data.bzl', second_bar_data='data')", "data = 'first_foo:'+first_foo_data+' first_bar:'+first_bar_data+' second_foo:'+second_foo_data+' second_bar:'+second_bar_data"]);
    t.invalidate_packages();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    result.throw_if_error();
    assert_eq!(
        result.global_str("data"),
        "first_foo:first_fu first_bar:first_ba second_foo:second_fu second_bar:second_ba"
    );
}

// Java: ModuleExtensionResolutionTest.multipleModules
#[test]
fn multiple_modules() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "module(name='root',version='1.0')",
            "bazel_dep(name='ext',version='1.0')",
            "bazel_dep(name='foo',version='1.0')",
            "bazel_dep(name='bar',version='2.0')",
            "ext = use_extension('@ext//:defs.bzl','ext')",
            "ext.tag(data='root')",
            "use_repo(ext,'ext_repo')",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &[
            "load('@ext_repo//:data.bzl', ext_data='data')",
            "data=ext_data",
        ],
    );
    t.registry().add_module(
        &create_module_key("foo", "1.0"),
        &[
            "module(name='foo',version='1.0')",
            "bazel_dep(name='ext',version='1.0')",
            "bazel_dep(name='quux',version='1.0')",
            "ext = use_extension('@ext//:defs.bzl','ext')",
            "ext.tag(data='foo@1.0')",
        ],
    );
    t.registry().add_module(
        &create_module_key("bar", "2.0"),
        &[
            "module(name='bar',version='2.0')",
            "bazel_dep(name='ext',version='1.0')",
            "bazel_dep(name='quux',version='2.0')",
            "ext = use_extension('@ext//:defs.bzl','ext')",
            "ext.tag(data='bar@2.0')",
        ],
    );
    t.registry().add_module(
        &create_module_key("quux", "1.0"),
        &[
            "module(name='quux',version='1.0')",
            "bazel_dep(name='ext',version='1.0')",
            "ext = use_extension('@ext//:defs.bzl','ext')",
            "ext.tag(data='quux@1.0')",
        ],
    );
    t.registry().add_module(
        &create_module_key("quux", "2.0"),
        &[
            "module(name='quux',version='2.0')",
            "bazel_dep(name='ext',version='1.0')",
            "ext = use_extension('@ext//:defs.bzl','ext')",
            "ext.tag(data='quux@2.0')",
        ],
    );
    t.registry().add_module(
        &create_module_key("ext", "1.0"),
        &[
            "module(name='ext',version='1.0')",
            "bazel_dep(name='data_repo',version='1.0')",
        ],
    );
    t.scratch.file(&module_path(&t, "ext+1.0/REPO.bazel"), &[]);
    t.scratch.file(&module_path(&t, "ext+1.0/BUILD"), &[]);
    t.scratch.file(
        &module_path(&t, "ext+1.0/defs.bzl"),
        &[
            "load('@data_repo//:defs.bzl','data_repo')",
            "def _ext_impl(ctx):",
            "  data_str = ''",
            "  for mod in ctx.modules:",
            "    data_str += mod.name + '@' + mod.version + (' (root): ' if mod.is_root else ': ')",
            "    for tag in mod.tags.tag:",
            "      data_str += tag.data",
            r#"    data_str += '\n'"#,
            "  data_repo(name='ext_repo',data=data_str)",
            "tag=tag_class(attrs={'data':attr.string()})",
            "ext=module_extension(implementation=_ext_impl,tag_classes={'tag':tag})",
        ],
    );
    t.invalidate_packages();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    result.throw_if_error();
    assert_eq!(
        result.global_str("data"),
        "root@1.0 (root): root
foo@1.0: foo@1.0
bar@2.0: bar@2.0
quux@2.0: quux@2.0
"
    );
}

// Java: ModuleExtensionResolutionTest.multipleModules_devDependency
#[test]
fn multiple_modules_dev_dependency() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "bazel_dep(name='ext',version='1.0')",
            "bazel_dep(name='foo',version='1.0')",
            "bazel_dep(name='bar',version='2.0')",
            "ext = use_extension('@ext//:defs.bzl','ext',dev_dependency=True)",
            "ext.tag(data='root')",
            "use_repo(ext,'ext_repo')",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &[
            "load('@ext_repo//:data.bzl', ext_data='data')",
            "data=ext_data",
        ],
    );
    t.registry().add_module(
        &create_module_key("foo", "1.0"),
        &[
            "module(name='foo',version='1.0')",
            "bazel_dep(name='ext',version='1.0')",
            "ext = use_extension('@ext//:defs.bzl','ext',dev_dependency=True)",
            "ext.tag(data='foo@1.0')",
        ],
    );
    t.registry().add_module(
        &create_module_key("bar", "2.0"),
        &[
            "module(name='bar',version='2.0')",
            "bazel_dep(name='ext',version='1.0')",
            "ext = use_extension('@ext//:defs.bzl','ext')",
            "ext.tag(data='bar@2.0')",
        ],
    );
    t.registry().add_module(
        &create_module_key("ext", "1.0"),
        &[
            "module(name='ext',version='1.0')",
            "bazel_dep(name='data_repo',version='1.0')",
        ],
    );
    t.scratch.file(&module_path(&t, "ext+1.0/REPO.bazel"), &[]);
    t.scratch.file(&module_path(&t, "ext+1.0/BUILD"), &[]);
    t.scratch.file(
        &module_path(&t, "ext+1.0/defs.bzl"),
        &[
            "load('@data_repo//:defs.bzl','data_repo')",
            "def _ext_impl(ctx):",
            "  data_str = 'modules:'",
            "  for mod in ctx.modules:",
            "    for tag in mod.tags.tag:",
            "      data_str += ' ' + tag.data + ' ' + str(ctx.is_dev_dependency(tag))",
            "  data_repo(name='ext_repo',data=data_str)",
            "tag=tag_class(attrs={'data':attr.string()})",
            "ext=module_extension(implementation=_ext_impl,tag_classes={'tag':tag})",
        ],
    );
    t.invalidate_packages();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    result.throw_if_error();
    assert_eq!(
        result.global_str("data"),
        "modules: root True bar@2.0 False"
    );
}

// Java: ModuleExtensionResolutionTest.multipleModules_ignoreDevDependency
#[test]
fn multiple_modules_ignore_dev_dependency() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "bazel_dep(name='ext',version='1.0')",
            "bazel_dep(name='foo',version='1.0')",
            "bazel_dep(name='bar',version='2.0')",
            "ext = use_extension('@ext//:defs.bzl','ext',dev_dependency=True)",
            "ext.tag(data='root')",
            "use_repo(ext,'ext_repo')",
        ],
    );
    t.registry().add_module(
        &create_module_key("foo", "1.0"),
        &[
            "module(name='foo',version='1.0')",
            "bazel_dep(name='ext',version='1.0')",
            "ext = use_extension('@ext//:defs.bzl','ext',dev_dependency=True)",
            "ext.tag(data='foo@1.0')",
        ],
    );
    t.registry().add_module(
        &create_module_key("bar", "2.0"),
        &[
            "module(name='bar',version='2.0')",
            "bazel_dep(name='ext',version='1.0')",
            "ext = use_extension('@ext//:defs.bzl','ext')",
            "ext.tag(data='bar@2.0')",
        ],
    );
    t.registry().add_module(
        &create_module_key("ext", "1.0"),
        &[
            "module(name='ext',version='1.0')",
            "bazel_dep(name='data_repo',version='1.0')",
        ],
    );
    t.scratch.file(&module_path(&t, "ext+1.0/REPO.bazel"), &[]);
    t.scratch.file(&module_path(&t, "ext+1.0/BUILD"), &[]);
    t.scratch.file(
        &module_path(&t, "ext+1.0/defs.bzl"),
        &[
            "load('@data_repo//:defs.bzl','data_repo')",
            "def _ext_impl(ctx):",
            "  data_str = 'modules:'",
            "  for mod in ctx.modules:",
            "    for tag in mod.tags.tag:",
            "      data_str += ' ' + tag.data + ' ' + str(ctx.is_dev_dependency(tag))",
            "  data_repo(name='ext_repo',data=data_str)",
            "tag=tag_class(attrs={'data':attr.string()})",
            "ext=module_extension(implementation=_ext_impl,tag_classes={'tag':tag})",
        ],
    );
    t.inject_extra_precomputed_values(&[InjectedPrecomputedValue::IgnoreDevDeps(true)]);
    t.invalidate_packages();
    let result = evaluate_bzl(&mut t, "@@ext++ext+ext_repo//:data.bzl");
    result.throw_if_error();
    assert_eq!(result.global_str("data"), "modules: bar@2.0 False");
}

// Java: ModuleExtensionResolutionTest.multipleModules_isolatedUsages
#[test]
fn multiple_modules_isolated_usages() {
    let mut t = new_test();
    t.scratch.overwrite_file("MODULE.bazel", &["module(name='root',version='1.0')", "bazel_dep(name='ext',version='1.0')", "bazel_dep(name='foo',version='1.0')", "ext = use_extension('@ext//:defs.bzl','ext')", "ext.tag(data='root',expect_isolated=False)", "use_repo(ext,'ext_repo')", "isolated_ext = use_extension('@ext//:defs.bzl','ext',isolate=True)", "isolated_ext.tag(data='root_isolated',expect_isolated=True)", "use_repo(isolated_ext,isolated_ext_repo='ext_repo')", "isolated_dev_ext = use_extension('@ext//:defs.bzl','ext',isolate=True,dev_dependency=True)", "isolated_dev_ext.tag(data='root_isolated_dev',expect_isolated=True)", "use_repo(isolated_dev_ext,isolated_dev_ext_repo='ext_repo')", "ext2 = use_extension('@ext//:defs.bzl','ext')", "ext2.tag(data='root_2',expect_isolated=False)"]);
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &[
            "load('@ext_repo//:data.bzl', ext_data='data')",
            "load('@isolated_ext_repo//:data.bzl', isolated_ext_data='data')",
            "load('@isolated_dev_ext_repo//:data.bzl', isolated_dev_ext_data='data')",
            "data=ext_data",
            "isolated_data=isolated_ext_data",
            "isolated_dev_data=isolated_dev_ext_data",
        ],
    );
    t.registry().add_module(&create_module_key("foo", "1.0"), &["module(name='foo',version='1.0')", "bazel_dep(name='ext',version='1.0')", "isolated_ext = use_extension('@ext//:defs.bzl','ext',isolate=True)", "isolated_ext.tag(data='foo@1.0_isolated',expect_isolated=True)", "use_repo(isolated_ext,isolated_ext_repo='ext_repo')", "isolated_dev_ext = use_extension('@ext//:defs.bzl','ext',isolate=True,dev_dependency=True)", "isolated_dev_ext.tag(data='foo@1.0_isolated_dev',expect_isolated=True)", "use_repo(isolated_dev_ext,isolated_dev_ext_repo='ext_repo')", "ext = use_extension('@ext//:defs.bzl','ext')", "ext.tag(data='foo@1.0',expect_isolated=False)", "use_repo(ext,'ext_repo')"]);
    t.scratch.file(&module_path(&t, "foo+1.0/REPO.bazel"), &[]);
    t.scratch.file(&module_path(&t, "foo+1.0/BUILD"), &[]);
    t.scratch.file(
        &module_path(&t, "foo+1.0/data.bzl"),
        &[
            "load('@ext_repo//:data.bzl', ext_data='data')",
            "load('@isolated_ext_repo//:data.bzl', isolated_ext_data='data')",
            "data=ext_data",
            "isolated_data=isolated_ext_data",
        ],
    );
    t.registry().add_module(
        &create_module_key("ext", "1.0"),
        &[
            "module(name='ext',version='1.0')",
            "bazel_dep(name='data_repo',version='1.0')",
        ],
    );
    t.scratch.file(&module_path(&t, "ext+1.0/REPO.bazel"), &[]);
    t.scratch.file(&module_path(&t, "ext+1.0/BUILD"), &[]);
    t.scratch.file(
        &module_path(&t, "ext+1.0/defs.bzl"),
        &[
            "load('@data_repo//:defs.bzl','data_repo')",
            "def _ext_impl(ctx):",
            "  data_str = ''",
            "  for mod in ctx.modules:",
            "    data_str += mod.name + '@' + mod.version + (' (root): ' if mod.is_root else ': ')",
            "    for tag in mod.tags.tag:",
            "      data_str += tag.data",
            "      if tag.expect_isolated != ctx.is_isolated:",
            "        fail()",
            r#"    data_str += '\n'"#,
            "  data_repo(name='ext_repo',data=data_str)",
            "tag=tag_class(attrs={'data':attr.string(),'expect_isolated':attr.bool()})",
            "ext=module_extension(implementation=_ext_impl,tag_classes={'tag':tag})",
        ],
    );
    t.set_build_language_options(&["--experimental_isolated_extension_usages"]);
    t.invalidate_packages();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    result.throw_if_error();
    assert_eq!(
        result.global_str("data"),
        "root@1.0 (root): rootroot_2
foo@1.0: foo@1.0
"
    );
    assert_eq!(
        result.global_str("isolated_data"),
        "root@1.0 (root): root_isolated
"
    );
    assert_eq!(
        result.global_str("isolated_dev_data"),
        "root@1.0 (root): root_isolated_dev
"
    );
    let result = evaluate_bzl(&mut t, "@foo+//:data.bzl");
    result.throw_if_error();
    assert_eq!(
        result.global_str("data"),
        "root@1.0 (root): rootroot_2
foo@1.0: foo@1.0
"
    );
    assert_eq!(
        result.global_str("isolated_data"),
        "foo@1.0: foo@1.0_isolated
"
    );
}

// Java: ModuleExtensionResolutionTest.labels_readInModuleExtension
#[test]
fn labels_read_in_module_extension() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "bazel_dep(name='ext',version='1.0')",
            "bazel_dep(name='foo',version='1.0')",
            "ext = use_extension('@ext//:defs.bzl','ext')",
            "ext.tag(file='//:requirements.txt')",
            "use_repo(ext,'ext_repo')",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &[
            "load('@ext_repo//:data.bzl', ext_data='data')",
            "data=ext_data",
        ],
    );
    t.scratch.file("requirements.txt", &["get up at 6am."]);
    t.registry().add_module(
        &create_module_key("foo", "1.0"),
        &[
            "module(name='foo',version='1.0')",
            "bazel_dep(name='ext',version='1.0')",
            "bazel_dep(name='bar',version='2.0')",
            "ext = use_extension('@ext//:defs.bzl','ext')",
            "ext.tag(file='@bar//:requirements.txt')",
        ],
    );
    t.registry().add_module(
        &create_module_key("bar", "2.0"),
        &["module(name='bar',version='2.0')"],
    );
    t.scratch.file(&module_path(&t, "bar+2.0/REPO.bazel"), &[]);
    t.scratch.file(&module_path(&t, "bar+2.0/BUILD"), &[]);
    t.scratch.file(
        &module_path(&t, "bar+2.0/requirements.txt"),
        &["go to bed at 11pm."],
    );
    t.registry().add_module(
        &create_module_key("ext", "1.0"),
        &[
            "module(name='ext',version='1.0')",
            "bazel_dep(name='data_repo',version='1.0')",
        ],
    );
    t.scratch.file(&module_path(&t, "ext+1.0/REPO.bazel"), &[]);
    t.scratch.file(&module_path(&t, "ext+1.0/BUILD"), &[]);
    t.scratch.file(
        &module_path(&t, "ext+1.0/defs.bzl"),
        &[
            "load('@data_repo//:defs.bzl','data_repo')",
            "def _ext_impl(ctx):",
            "  data_str = 'requirements:'",
            "  for mod in ctx.modules:",
            "    for tag in mod.tags.tag:",
            "      data_str += ' ' + ctx.read(tag.file).strip()",
            "  data_repo(name='ext_repo',data=data_str)",
            "tag=tag_class(attrs={'file':attr.label()})",
            "ext=module_extension(implementation=_ext_impl,tag_classes={'tag':tag})",
        ],
    );
    t.invalidate_packages();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    result.throw_if_error();
    assert_eq!(
        result.global_str("data"),
        "requirements: get up at 6am. go to bed at 11pm."
    );
}

// Java: ModuleExtensionResolutionTest.labels_passedOnToRepoRule
#[test]
#[ignore = "not ported yet: asserts the lockfile's recorded inputs (RepoRecordedInput.WithValue of a RecordedRepoMapping)"]
fn labels_passed_on_to_repo_rule() {}

// Java: ModuleExtensionResolutionTest.labels_fromExtensionGeneratedRepo
#[test]
fn labels_from_extension_generated_repo() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "bazel_dep(name='ext',version='1.0')",
            "myext = use_extension('//:defs.bzl','myext')",
            "use_repo(myext,'myrepo')",
            "ext = use_extension('@ext//:defs.bzl','ext')",
            "ext.tag(file='@myrepo//:requirements.txt')",
            "use_repo(ext,'ext_repo')",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &[
            "load('@ext_repo//:data.bzl', ext_data='data')",
            "data=ext_data",
        ],
    );
    t.scratch.file(
        "defs.bzl",
        &[
            "def _myrepo_impl(ctx):",
            "  ctx.file('BUILD')",
            "  ctx.file('requirements.txt', 'get up at 6am.')",
            "myrepo = repository_rule(implementation=_myrepo_impl)",
            "",
            "def _myext_impl(ctx):",
            "  myrepo(name='myrepo')",
            "myext=module_extension(implementation=_myext_impl)",
        ],
    );
    t.scratch.file("requirements.txt", &["get up at 6am."]);
    t.registry().add_module(
        &create_module_key("ext", "1.0"),
        &["module(name='ext',version='1.0')"],
    );
    t.scratch.file(&module_path(&t, "ext+1.0/REPO.bazel"), &[]);
    t.scratch.file(&module_path(&t, "ext+1.0/BUILD"), &[]);
    t.scratch.file(
        &module_path(&t, "ext+1.0/defs.bzl"),
        &[
            "def _data_repo_impl(ctx):",
            "  ctx.file('BUILD')",
            "  content = ' '.join([ctx.read(l).strip() for l in ctx.attr.files])",
            "  ctx.file('data.bzl', 'data='+json.encode(content))",
            "data_repo = repository_rule(",
            "  implementation=_data_repo_impl, attrs={'files':attr.label_list()})",
            "",
            "def _ext_impl(ctx):",
            "  data_files = []",
            "  for mod in ctx.modules:",
            "    for tag in mod.tags.tag:",
            "      data_files.append(tag.file)",
            "  data_repo(name='ext_repo',files=data_files)",
            "tag=tag_class(attrs={'file':attr.label()})",
            "ext=module_extension(implementation=_ext_impl,tag_classes={'tag':tag})",
        ],
    );
    t.invalidate_packages();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    result.throw_if_error();
    assert_eq!(result.global_str("data"), "get up at 6am.");
}

// Java: ModuleExtensionResolutionTest.labels_constructedInModuleExtension_readInModuleExtension
#[test]
fn labels_constructed_in_module_extension_read_in_module_extension() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "bazel_dep(name='ext',version='1.0')",
            "ext = use_extension('@ext//:defs.bzl','ext')",
            "ext.tag()",
            "use_repo(ext,'ext_repo')",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &[
            "load('@ext_repo//:data.bzl', ext_data='data')",
            "data=ext_data",
        ],
    );
    t.registry().add_module(
        &create_module_key("foo", "1.0"),
        &["module(name='foo',version='1.0')"],
    );
    t.scratch.file(&module_path(&t, "foo+1.0/REPO.bazel"), &[]);
    t.scratch.file(&module_path(&t, "foo+1.0/BUILD"), &[]);
    t.scratch.file(
        &module_path(&t, "foo+1.0/requirements.txt"),
        &["get up at 6am."],
    );
    t.registry().add_module(
        &create_module_key("bar", "2.0"),
        &["module(name='bar',version='2.0')"],
    );
    t.scratch.file(&module_path(&t, "bar+2.0/REPO.bazel"), &[]);
    t.scratch.file(&module_path(&t, "bar+2.0/BUILD"), &[]);
    t.scratch.file(
        &module_path(&t, "bar+2.0/requirements.txt"),
        &["go to bed at 11pm."],
    );
    t.registry().add_module(
        &create_module_key("ext", "1.0"),
        &[
            "module(name='ext',version='1.0')",
            "bazel_dep(name='foo',version='1.0')",
            "bazel_dep(name='bar',version='2.0')",
            "bazel_dep(name='data_repo',version='1.0')",
        ],
    );
    t.scratch.file(&module_path(&t, "ext+1.0/REPO.bazel"), &[]);
    t.scratch.file(&module_path(&t, "ext+1.0/BUILD"), &[]);
    // The Label() call on the following line should work, using ext.1.0's repo mapping.
    // So should the attr.label default value on the following line.
    t.scratch.file(
        &module_path(&t, "ext+1.0/defs.bzl"),
        &[
            "load('@data_repo//:defs.bzl','data_repo')",
            "def _ext_impl(ctx):",
            "  data_str = 'requirements: ' + ctx.read(Label('@foo//:requirements.txt')).strip()",
            "  for mod in ctx.modules:",
            "    for tag in mod.tags.tag:",
            "      data_str += ' ' + ctx.read(tag.file).strip()",
            "  data_repo(name='ext_repo',data=data_str)",
            "tag=tag_class(attrs={'file':attr.label(default='@bar//:requirements.txt')})",
            "ext=module_extension(implementation=_ext_impl,tag_classes={'tag':tag})",
        ],
    );
    t.invalidate_packages();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    result.throw_if_error();
    assert_eq!(
        result.global_str("data"),
        "requirements: get up at 6am. go to bed at 11pm."
    );
}

// Java: ModuleExtensionResolutionTest.labels_constructedInModuleExtensionAsString_passedOnToRepoRule
#[test]
fn labels_constructed_in_module_extension_as_string_passed_on_to_repo_rule() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "bazel_dep(name='ext',version='1.0')",
            "ext = use_extension('@ext//:defs.bzl','ext')",
            "use_repo(ext,'ext_repo')",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &[
            "load('@ext_repo//:data.bzl', ext_data='data')",
            "data=ext_data",
        ],
    );
    t.registry().add_module(
        &create_module_key("foo", "1.0"),
        &["module(name='foo',version='1.0')"],
    );
    t.scratch.file(&module_path(&t, "foo+1.0/REPO.bazel"), &[]);
    t.scratch.file(&module_path(&t, "foo+1.0/BUILD"), &[]);
    t.scratch.file(
        &module_path(&t, "foo+1.0/requirements.txt"),
        &["get up at 6am."],
    );
    t.registry().add_module(
        &create_module_key("ext", "1.0"),
        &[
            "module(name='ext',version='1.0')",
            "bazel_dep(name='foo',version='1.0')",
            "bazel_dep(name='data_repo',version='1.0')",
        ],
    );
    t.scratch.file(&module_path(&t, "ext+1.0/REPO.bazel"), &[]);
    t.scratch.file(&module_path(&t, "ext+1.0/BUILD"), &[]);
    // The label literal on the following line should be interpreted using ext.1.0's repo
    // mapping.
    t.scratch.file(
        &module_path(&t, "ext+1.0/defs.bzl"),
        &[
            "def _data_repo_impl(ctx):",
            "  ctx.file('BUILD')",
            "  content = ctx.read(ctx.attr.file).strip()",
            "  ctx.file('data.bzl', 'data='+json.encode(content))",
            "data_repo = repository_rule(",
            "  implementation=_data_repo_impl, attrs={'file':attr.label()})",
            "",
            "def _ext_impl(ctx):",
            "  data_repo(name='ext_repo',file='@foo//:requirements.txt')",
            "ext=module_extension(implementation=_ext_impl)",
        ],
    );
    t.invalidate_packages();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    result.throw_if_error();
    assert_eq!(result.global_str("data"), "get up at 6am.");
}

// Java: ModuleExtensionResolutionTest.complexTypedAttribute
#[test]
fn complex_typed_attribute() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "bazel_dep(name='data_repo', version='1.0')",
            "ext = use_extension('//:defs.bzl', 'ext')",
            "ext.tag(data={'foo':['val1','val2'],'bar':['val3','val4']})",
            "use_repo(ext, 'foo', 'bar')",
        ],
    );
    t.scratch.file(
        "defs.bzl",
        &[
            "load('@data_repo//:defs.bzl','data_repo')",
            "tag = tag_class(attrs = {'data':attr.string_list_dict()})",
            "def _ext_impl(ctx):",
            "  for mod in ctx.modules:",
            "    for tag in mod.tags.tag:",
            "      for key in tag.data:",
            "        data_repo(name=key,data=','.join(tag.data[key]))",
            "ext = module_extension(implementation=_ext_impl, tag_classes={'tag':tag})",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &[
            "load('@foo//:data.bzl', foo_data='data')",
            "load('@bar//:data.bzl', bar_data='data')",
            "data = 'foo:'+foo_data+' bar:'+bar_data",
        ],
    );
    t.invalidate_packages();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    result.throw_if_error();
    assert_eq!(result.global_str("data"), "foo:val1,val2 bar:val3,val4");
}

// Java: ModuleExtensionResolutionTest.complexTypedAttribute_default
#[test]
fn complex_typed_attribute_default() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "bazel_dep(name='data_repo', version='1.0')",
            "ext = use_extension('//:defs.bzl', 'ext')",
            "ext.tag()",
            "use_repo(ext, 'foo', 'bar')",
        ],
    );
    t.scratch.file(
        "defs.bzl",
        &[
            "load('@data_repo//:defs.bzl','data_repo')",
            "tag = tag_class(attrs = {",
            "  'data': attr.string_list_dict(",
            "    default = {'foo':['val1','val2'],'bar':['val3','val4']},",
            ")})",
            "def _ext_impl(ctx):",
            "  for mod in ctx.modules:",
            "    for tag in mod.tags.tag:",
            "      for key in tag.data:",
            "        data_repo(name=key,data=','.join(tag.data[key]))",
            "ext = module_extension(implementation=_ext_impl, tag_classes={'tag':tag})",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &[
            "load('@foo//:data.bzl', foo_data='data')",
            "load('@bar//:data.bzl', bar_data='data')",
            "data = 'foo:'+foo_data+' bar:'+bar_data",
        ],
    );
    t.invalidate_packages();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    result.throw_if_error();
    assert_eq!(result.global_str("data"), "foo:val1,val2 bar:val3,val4");
}

// Java: ModuleExtensionResolutionTest.generatedReposHaveCorrectMappings
#[test]
fn generated_repos_have_correct_mappings() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "bazel_dep(name='foo',version='1.0')",
            "ext = use_extension('//:defs.bzl','ext')",
            "use_repo(ext,'ext')",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &["load('@ext//:data.bzl', ext_data='data')", "data=ext_data"],
    );
    t.scratch.file(
        "defs.bzl",
        &[
            "def _ext_repo_impl(ctx):",
            "  ctx.file('BUILD')",
            r#"  ctx.file('data.bzl', """load('@foo//:data.bzl', foo_data='data')"#,
            "load('@internal//:data.bzl', internal_data='data')",
            "data = 'foo: '+foo_data+' internal: '+internal_data",
            r#"""")"#,
            "ext_repo = repository_rule(implementation=_ext_repo_impl)",
            "",
            "def _internal_repo_impl(ctx):",
            "  ctx.file('BUILD')",
            "  ctx.file('data.bzl', 'data='+json.encode('internal-stuff'))",
            "internal_repo = repository_rule(implementation=_internal_repo_impl)",
            "",
            "def _ext_impl(ctx):",
            "  internal_repo(name='internal')",
            "  ext_repo(name='ext')",
            "ext=module_extension(implementation=_ext_impl)",
        ],
    );
    t.registry().add_module(
        &create_module_key("foo", "1.0"),
        &["module(name='foo',version='1.0')"],
    );
    t.scratch.file(&module_path(&t, "foo+1.0/REPO.bazel"), &[]);
    t.scratch.file(&module_path(&t, "foo+1.0/BUILD"), &[]);
    t.scratch.file(
        &module_path(&t, "foo+1.0/data.bzl"),
        &["data = 'foo-stuff'"],
    );
    t.invalidate_packages();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    result.throw_if_error();
    assert_eq!(
        result.global_str("data"),
        "foo: foo-stuff internal: internal-stuff"
    );
}

// Java: ModuleExtensionResolutionTest.generatedReposHaveCorrectMappings_moduleOwnRepoName
#[test]
fn generated_repos_have_correct_mappings_module_own_repo_name() {
    let mut t = new_test();
    // tests that things work correctly when the module specifies its own repo name (via
    // `module(repo_name=...)`).
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "module(name='foo',version='1.0',repo_name='bar')",
            "ext = use_extension('//:defs.bzl','ext')",
            "use_repo(ext,'ext')",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file("data.bzl", &["data='hello world'"]);
    t.scratch.file(
        "defs.bzl",
        &[
            "def _ext_repo_impl(ctx):",
            "  ctx.file('BUILD')",
            r#"  ctx.file('data.bzl', """load('@bar//:data.bzl', bar_data='data')"#,
            "data = 'bar: '+bar_data",
            r#"""")"#,
            "ext_repo = repository_rule(implementation=_ext_repo_impl)",
            "",
            "ext=module_extension(implementation=lambda ctx: ext_repo(name='ext'))",
        ],
    );
    t.scratch.file(
        "ext_data.bzl",
        &[
            "load('@ext//:data.bzl', ext_data='data')",
            "data='ext: ' + ext_data",
        ],
    );
    t.invalidate_packages();
    let result = evaluate_bzl(&mut t, "//:ext_data.bzl");
    result.throw_if_error();
    assert_eq!(result.global_str("data"), "ext: bar: hello world");
}

// Java: ModuleExtensionResolutionTest.generatedReposHaveCorrectMappings_internalRepoWins
#[test]
fn generated_repos_have_correct_mappings_internal_repo_wins() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "bazel_dep(name='foo',version='1.0')",
            "ext = use_extension('//:defs.bzl','ext')",
            "use_repo(ext,'ext')",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &["load('@ext//:data.bzl', ext_data='data')", "data=ext_data"],
    );
    t.scratch.file(
        "defs.bzl",
        &[
            "def _ext_repo_impl(ctx):",
            "  ctx.file('BUILD')",
            r#"  ctx.file('data.bzl', """load('@foo//:data.bzl', foo_data='data')"#,
            "data = 'the foo I see is '+foo_data",
            r#"""")"#,
            "ext_repo = repository_rule(implementation=_ext_repo_impl)",
            "",
            "def _internal_repo_impl(ctx):",
            "  ctx.file('BUILD')",
            "  ctx.file('data.bzl', 'data='+json.encode('inner-foo'))",
            "internal_repo = repository_rule(implementation=_internal_repo_impl)",
            "",
            "def _ext_impl(ctx):",
            "  internal_repo(name='foo')",
            "  ext_repo(name='ext')",
            "tag=tag_class(attrs={'file':attr.label()})",
            "ext=module_extension(implementation=_ext_impl,tag_classes={'tag':tag})",
        ],
    );
    t.registry().add_module(
        &create_module_key("foo", "1.0"),
        &["module(name='foo',version='1.0')"],
    );
    t.scratch.file(&module_path(&t, "foo+1.0/REPO.bazel"), &[]);
    t.scratch.file(&module_path(&t, "foo+1.0/BUILD"), &[]);
    t.scratch.file(
        &module_path(&t, "foo+1.0/data.bzl"),
        &["data = 'outer-foo'"],
    );
    t.invalidate_packages();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    result.throw_if_error();
    assert_eq!(result.global_str("data"), "the foo I see is inner-foo");
}

// Java: ModuleExtensionResolutionTest.generatedReposHaveCorrectMappings_strictDepsViolation
#[test]
fn generated_repos_have_correct_mappings_strict_deps_violation() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "ext = use_extension('//:defs.bzl','ext')",
            "use_repo(ext,'ext')",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &["load('@ext//:data.bzl', ext_data='data')", "data=ext_data"],
    );
    t.scratch.file(
        "defs.bzl",
        &[
            "def _ext_repo_impl(ctx):",
            "  ctx.file('BUILD')",
            r#"  ctx.file('data.bzl', """load('@foo//:data.bzl', 'data')""")"#,
            "ext_repo = repository_rule(implementation=_ext_repo_impl)",
            "",
            "def _ext_impl(ctx):",
            "  ext_repo(name='ext')",
            "tag=tag_class(attrs={'file':attr.label()})",
            "ext=module_extension(implementation=_ext_impl,tag_classes={'tag':tag})",
        ],
    );
    t.invalidate_packages();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    assert!(result.has_error());
    assert_contains(
        &result.error_message(),
        "No repository visible as '@foo' from repository '@@+ext+ext'",
    );
}

// Java: ModuleExtensionResolutionTest.wrongModuleExtensionLabel
#[test]
fn wrong_module_extension_label() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "ext = use_extension('//foo/defs.bzl','ext')",
            "use_repo(ext,'ext')",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &["load('@ext//:data.bzl', ext_data='data')", "data=ext_data"],
    );
    t.invalidate_packages();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    assert!(result.has_error());
    assert_contains(
        &result.error_message(),
        "Label '//foo/defs.bzl:defs.bzl' is invalid because 'foo/defs.bzl' is not a package",
    );
}

// Java: ModuleExtensionResolutionTest.importNonExistentRepo
#[test]
fn import_non_existent_repo() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "ext = use_extension('//:defs.bzl','ext')",
            "bazel_dep(name='data_repo', version='1.0')",
            "use_repo(ext,my_repo='missing_repo')",
        ],
    );
    t.scratch.file(
        "defs.bzl",
        &[
            "load('@data_repo//:defs.bzl','data_repo')",
            "def _ext_impl(ctx):",
            "  data_repo(name='ext',data='void')",
            "ext = module_extension(implementation=_ext_impl)",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &[
            "load('@@+ext+ext//:data.bzl', ext_data='data')",
            "data=ext_data",
        ],
    );
    t.invalidate_packages();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    assert!(result.has_error());
    assert_contains(&result.error_message(), &t.ws(r#"module extension @@//:defs.bzl%ext does not generate repository "missing_repo", yet it is imported as "my_repo" in the usage at /workspace/MODULE.bazel:1:20"#));
}

// Java: ModuleExtensionResolutionTest.invalidAttributeValue
#[test]
fn invalid_attribute_value() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "ext = use_extension('//:defs.bzl','ext')",
            "bazel_dep(name='data_repo', version='1.0')",
            "use_repo(ext,'ext')",
        ],
    );
    t.scratch.file(
        "defs.bzl",
        &[
            "load('@data_repo//:defs.bzl','data_repo')",
            "def _ext_impl(ctx):",
            "  data_repo(name='ext',data=42)",
            "ext = module_extension(implementation=_ext_impl)",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &["load('@ext//:data.bzl', ext_data='data')", "data=ext_data"],
    );
    t.invalidate_packages();
    t.remove_fail_fast_handler();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    assert!(result.has_error());
    t.assert_contains_event(&t.ws(r#"ERROR /workspace/defs.bzl:3:12: Traceback (most recent call last):
	File "/workspace/defs.bzl", line 3, column 12, in _ext_impl
		data_repo(name='ext',data=42)
Error: in call to 'data_repo' repo rule with name 'ext', expected value of type 'string' for attribute 'data', but got 42 (int)"#));
}

// Java: ModuleExtensionResolutionTest.badRepoNameInExtensionImplFunction
#[test]
fn bad_repo_name_in_extension_impl_function() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "ext = use_extension('//:defs.bzl','ext')",
            "bazel_dep(name='data_repo', version='1.0')",
            "use_repo(ext,'ext')",
        ],
    );
    t.scratch.file(
        "defs.bzl",
        &[
            "load('@data_repo//:defs.bzl','data_repo')",
            "def _ext_impl(ctx):",
            "  data_repo(name='_something',data='void')",
            "ext = module_extension(implementation=_ext_impl)",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &["load('@ext//:data.bzl', ext_data='data')", "data=ext_data"],
    );
    t.invalidate_packages();
    t.remove_fail_fast_handler();
    evaluate_bzl(&mut t, "//:data.bzl");
    t.assert_contains_event("invalid user-provided repo name '_something'");
}

// Java: ModuleExtensionResolutionTest.nonVisibleLabelInLabelAttr
#[test]
fn non_visible_label_in_label_attr() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "ext = use_extension('//:defs.bzl','ext')",
            "use_repo(ext,'ext')",
        ],
    );
    t.scratch.file(
        "defs.bzl",
        &[
            "def _data_repo_impl(ctx):",
            "  ctx.file('BUILD')",
            "data_repo = repository_rule(",
            "  implementation=_data_repo_impl,",
            "  attrs={'data':attr.label()})",
            "def _ext_impl(ctx):",
            "  data_repo(name='other_repo')",
            "  data_repo(name='ext',data='@not_other_repo//:foo')",
            "ext = module_extension(implementation=_ext_impl)",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &["load('@ext//:data.bzl', ext_data='data')", "data=ext_data"],
    );
    t.invalidate_packages();
    t.remove_fail_fast_handler();
    evaluate_bzl(&mut t, "//:data.bzl");
    t.assert_contains_event(&t.ws(r#"ERROR /workspace/defs.bzl:8:12: Traceback (most recent call last):
	File "/workspace/defs.bzl", line 8, column 12, in _ext_impl
		data_repo(name='ext',data='@not_other_repo//:foo')
Error: in call to 'data_repo' repo rule with name 'ext', no repository visible as '@not_other_repo' in the extension '@@//:defs.bzl%ext', but referenced by label '@not_other_repo//:foo' in attribute 'data'"#));
}

// Java: ModuleExtensionResolutionTest.nonVisibleLabelInLabelAttrNonRootModule
#[test]
fn non_visible_label_in_label_attr_non_root_module() {
    let mut t = new_test();
    t.registry().add_module(
        &create_module_key("ext_module", "1.0"),
        &["module(name='ext_module',version='1.0')"],
    );
    t.scratch
        .file(&module_path(&t, "ext_module+1.0/REPO.bazel"), &[]);
    t.scratch
        .file(&module_path(&t, "ext_module+1.0/BUILD"), &[]);
    t.scratch.file(
        &module_path(&t, "ext_module+1.0/defs.bzl"),
        &[
            "def _data_repo_impl(ctx):",
            "  ctx.file('BUILD')",
            "data_repo = repository_rule(",
            "  implementation=_data_repo_impl,",
            "  attrs={'data':attr.label()})",
            "def _ext_impl(ctx):",
            "  data_repo(name='other_repo')",
            "  data_repo(name='ext',data='@not_other_repo//:foo')",
            "ext = module_extension(implementation=_ext_impl)",
        ],
    );
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "bazel_dep(name = 'ext_module', version = '1.0')",
            "ext = use_extension('@ext_module//:defs.bzl','ext')",
            "use_repo(ext,'ext')",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &["load('@ext//:data.bzl', ext_data='data')", "data=ext_data"],
    );
    t.invalidate_packages();
    t.remove_fail_fast_handler();
    evaluate_bzl(&mut t, "//:data.bzl");
    t.assert_contains_event(&t.ws(r#"ERROR /usr/local/google/_blaze_jrluser/FAKEMD5/external/ext_module+/defs.bzl:8:12: Traceback (most recent call last):
	File "/usr/local/google/_blaze_jrluser/FAKEMD5/external/ext_module+/defs.bzl", line 8, column 12, in _ext_impl
		data_repo(name='ext',data='@not_other_repo//:foo')
Error: in call to 'data_repo' repo rule with name 'ext', no repository visible as '@not_other_repo' in the extension '@@ext_module+//:defs.bzl%ext', but referenced by label '@not_other_repo//:foo' in attribute 'data'"#));
}

// Java: ModuleExtensionResolutionTest.nonVisibleLabelInLabelAttrForwardedFromTag
#[test]
fn non_visible_label_in_label_attr_forwarded_from_tag() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "ext = use_extension('//:defs.bzl','ext')",
            "ext.label(label = '@other_repo//:foo')",
            "use_repo(ext,'ext')",
        ],
    );
    t.scratch.file(
        "defs.bzl",
        &[
            "def _data_repo_impl(ctx):",
            "  ctx.file('BUILD')",
            "data_repo = repository_rule(",
            "  implementation=_data_repo_impl,",
            "  attrs={'data':attr.label()})",
            "def _ext_impl(ctx):",
            "  data_repo(name='other_repo')",
            "  data_repo(name='ext',data=ctx.modules[0].tags.label[0].label)",
            "label=tag_class(attrs={'label':attr.label()})",
            "ext = module_extension(",
            "  implementation=_ext_impl,",
            "  tag_classes={'label':label},",
            ")",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &["load('@ext//:data.bzl', ext_data='data')", "data=ext_data"],
    );
    t.invalidate_packages();
    t.remove_fail_fast_handler();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    assert!(result.has_error());
    assert_eq!(result.error_message(), t.ws(r#"Traceback (most recent call last):
	File "/workspace/MODULE.bazel", line 2, column 10, in <toplevel>
		ext.label(label = '@other_repo//:foo')
Error: in 'label' tag, no repository visible as '@other_repo' to the root module, but referenced by label '@other_repo//:foo' in attribute 'label'"#));
}

// Java: ModuleExtensionResolutionTest.nonVisibleLabelInLabelListAttr
#[test]
fn non_visible_label_in_label_list_attr() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "ext = use_extension('//:defs.bzl','ext')",
            "use_repo(ext,'ext')",
        ],
    );
    t.scratch.file(
        "defs.bzl",
        &[
            "def _data_repo_impl(ctx):",
            "  ctx.file('BUILD')",
            "data_repo = repository_rule(",
            "  implementation=_data_repo_impl,",
            "  attrs={'data':attr.label_list()})",
            "def _ext_impl(ctx):",
            "  data_repo(name='other_repo')",
            "  data_repo(name='ext',data=['@not_other_repo//:foo'])",
            "ext = module_extension(implementation=_ext_impl)",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &["load('@ext//:data.bzl', ext_data='data')", "data=ext_data"],
    );
    t.invalidate_packages();
    t.remove_fail_fast_handler();
    evaluate_bzl(&mut t, "//:data.bzl");
    t.assert_contains_event(&t.ws(r#"ERROR /workspace/defs.bzl:8:12: Traceback (most recent call last):
	File "/workspace/defs.bzl", line 8, column 12, in _ext_impl
		data_repo(name='ext',data=['@not_other_repo//:foo'])
Error: in call to 'data_repo' repo rule with name 'ext', no repository visible as '@not_other_repo' in the extension '@@//:defs.bzl%ext', but referenced by label '@not_other_repo//:foo' in attribute 'data'"#));
}

// Java: ModuleExtensionResolutionTest.nonVisibleLabelInLabelKeyedStringDictAttr
#[test]
fn non_visible_label_in_label_keyed_string_dict_attr() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "ext = use_extension('//:defs.bzl','ext')",
            "use_repo(ext,'ext')",
        ],
    );
    t.scratch.file(
        "defs.bzl",
        &[
            "def _data_repo_impl(ctx):",
            "  ctx.file('BUILD')",
            "data_repo = repository_rule(",
            "  implementation=_data_repo_impl,",
            "  attrs={'data':attr.label_keyed_string_dict()})",
            "def _ext_impl(ctx):",
            "  data_repo(name='other_repo')",
            "  data_repo(name='ext',data={'@not_other_repo//:foo':'bar'})",
            "ext = module_extension(implementation=_ext_impl)",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &["load('@ext//:data.bzl', ext_data='data')", "data=ext_data"],
    );
    t.invalidate_packages();
    t.remove_fail_fast_handler();
    evaluate_bzl(&mut t, "//:data.bzl");
    t.assert_contains_event(&t.ws(r#"ERROR /workspace/defs.bzl:8:12: Traceback (most recent call last):
	File "/workspace/defs.bzl", line 8, column 12, in _ext_impl
		data_repo(name='ext',data={'@not_other_repo//:foo':'bar'})
Error: in call to 'data_repo' repo rule with name 'ext', no repository visible as '@not_other_repo' in the extension '@@//:defs.bzl%ext', but referenced by label '@not_other_repo//:foo' in attribute 'data'"#));
}

// Java: ModuleExtensionResolutionTest.nonVisibleLabelInLabelListDictAttr
#[test]
fn non_visible_label_in_label_list_dict_attr() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "ext = use_extension('//:defs.bzl','ext')",
            "use_repo(ext,'ext')",
        ],
    );
    t.scratch.file(
        "defs.bzl",
        &[
            "def _data_repo_impl(ctx):",
            "  ctx.file('BUILD')",
            "data_repo = repository_rule(",
            "  implementation=_data_repo_impl,",
            "  attrs={'data':attr.label_list_dict()})",
            "def _ext_impl(ctx):",
            "  data_repo(name='other_repo')",
            "  data_repo(name='ext',data={'bar':['@not_other_repo//:foo']})",
            "ext = module_extension(implementation=_ext_impl)",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &["load('@ext//:data.bzl', ext_data='data')", "data=ext_data"],
    );
    t.invalidate_packages();
    t.remove_fail_fast_handler();
    evaluate_bzl(&mut t, "//:data.bzl");
    t.assert_contains_event(&t.ws(r#"ERROR /workspace/defs.bzl:8:12: Traceback (most recent call last):
	File "/workspace/defs.bzl", line 8, column 12, in _ext_impl
		data_repo(name='ext',data={'bar':['@not_other_repo//:foo']})
Error: in call to 'data_repo' repo rule with name 'ext', no repository visible as '@not_other_repo' in the extension '@@//:defs.bzl%ext', but referenced by label '@not_other_repo//:foo' in attribute 'data'"#));
}

// Java: ModuleExtensionResolutionTest.nativeExistingRuleIsEmpty
#[test]
fn native_existing_rule_is_empty() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "bazel_dep(name='data_repo', version='1.0')",
            "ext = use_extension('//:defs.bzl', 'ext')",
            "use_repo(ext, 'ext')",
        ],
    );
    t.scratch.file(
        "defs.bzl",
        &[
            "load('@data_repo//:defs.bzl','data_repo')",
            "def _ext_impl(ctx):",
            "  if not native.existing_rules():",
            "    data_repo(name='ext',data='haha')",
            "ext = module_extension(implementation=_ext_impl)",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &[
            "load('@ext//:data.bzl', ext_data='data')",
            "data = ext_data",
        ],
    );
    t.invalidate_packages();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    result.throw_if_error();
    assert_eq!(result.global_str("data"), "haha");
}

// Java: ModuleExtensionResolutionTest.extensionLoadsRepoFromAnotherExtension
#[test]
fn extension_loads_repo_from_another_extension() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "bazel_dep(name='ext', version='1.0')",
            "bazel_dep(name='data_repo',version='1.0')",
            "my_ext = use_extension('@//:defs.bzl', 'my_ext')",
            "use_repo(my_ext, 'summarized_candy')",
            "ext = use_extension('@ext//:defs.bzl', 'ext')",
            "use_repo(ext, 'exposed_candy')",
        ],
    );
    t.scratch.file(
        "defs.bzl",
        &[
            "load('@data_repo//:defs.bzl','data_repo')",
            "load('@@ext++ext+candy//:data.bzl', candy='data')",
            "load('@exposed_candy//:data.bzl', exposed_candy='data')",
            "def _ext_impl(ctx):",
            "  data_str = exposed_candy + ' (and ' + candy + ')'",
            "  data_repo(name='summarized_candy', data=data_str)",
            "my_ext=module_extension(implementation=_ext_impl)",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &[
            "load('@summarized_candy//:data.bzl', data='data')",
            "candy_data = 'candy: ' + data",
        ],
    );
    t.registry().add_module(
        &create_module_key("ext", "1.0"),
        &[
            "module(name='ext',version='1.0')",
            "bazel_dep(name='data_repo',version='1.0')",
        ],
    );
    t.scratch.file(&module_path(&t, "ext+1.0/REPO.bazel"), &[]);
    t.scratch.file(&module_path(&t, "ext+1.0/BUILD"), &[]);
    t.scratch.file(
        &module_path(&t, "ext+1.0/defs.bzl"),
        &[
            "load('@data_repo//:defs.bzl','data_repo')",
            "def _ext_impl(ctx):",
            "  data_repo(name='candy', data='cotton candy')",
            "  data_repo(name='exposed_candy', data='lollipops')",
            "ext = module_extension(implementation=_ext_impl)",
        ],
    );
    t.invalidate_packages();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    result.throw_if_error();
    assert_eq!(
        result.global_str("candy_data"),
        "candy: lollipops (and cotton candy)"
    );
}

// Java: ModuleExtensionResolutionTest.extensionRepoCtxReadsFromAnotherExtensionRepo
#[test]
fn extension_repo_ctx_reads_from_another_extension_repo() {
    let mut t = new_test();
    // Repos from this extension (i.e. my_ext2) can still be used if their canonical name is
    // somehow known
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "bazel_dep(name='data_repo',version='1.0')",
            "my_ext = use_extension('@//:defs.bzl', 'my_ext')",
            "use_repo(my_ext, 'candy1')",
            "my_ext2 = use_extension('@//:defs.bzl', 'my_ext2')",
        ],
    );
    t.scratch.file(
        "defs.bzl",
        &[
            "load('@data_repo//:defs.bzl','data_repo')",
            "def _ext_impl(ctx):",
            "  data_file = ctx.read(Label('@@+my_ext2+candy2//:data.bzl'))",
            "  data_repo(name='candy1',data=data_file)",
            "my_ext=module_extension(implementation=_ext_impl)",
            "def _ext_impl2(ctx):",
            "  data_repo(name='candy2',data='lollipops')",
            "my_ext2=module_extension(implementation=_ext_impl2)",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &[
            "load('@candy1//:data.bzl', data='data')",
            "candy_data_file = data",
        ],
    );
    t.invalidate_packages();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    result.throw_if_error();
    assert_eq!(
        result.global_str("candy_data_file"),
        r#"data = "lollipops""#
    );
}

// Java: ModuleExtensionResolutionTest.testReportRepoAndBzlCycles_circularExtReposCtxRead
#[test]
#[ignore = "not ported yet: evaluates a PackageValue key and reports cycles with CyclesReporter (BzlLoadCycleReporter, BzlmodRepoCycleReporter)"]
fn test_report_repo_and_bzl_cycles_circular_ext_repos_ctx_read() {}

// Java: ModuleExtensionResolutionTest.testReportRepoAndBzlCycles_circularExtReposLoadInDefFile
#[test]
#[ignore = "not ported yet: evaluates a PackageValue key and reports cycles with CyclesReporter (BzlLoadCycleReporter, BzlmodRepoCycleReporter)"]
fn test_report_repo_and_bzl_cycles_circular_ext_repos_load_in_def_file() {}

// Java: ModuleExtensionResolutionTest.testReportRepoAndBzlCycles_extRepoLoadSelfCycle
#[test]
#[ignore = "not ported yet: evaluates a PackageValue key and reports cycles with CyclesReporter (BzlLoadCycleReporter, BzlmodRepoCycleReporter)"]
fn test_report_repo_and_bzl_cycles_ext_repo_load_self_cycle() {}

// Java: ModuleExtensionResolutionTest.extensionMetadata_exactlyOneArgIsNone
#[test]
fn extension_metadata_exactly_one_arg_is_none() {
    let mut t = new_test();
    let result = evaluate_simple_module_extension(
        &mut t,
        "return ctx.extension_metadata(root_module_direct_deps=['foo'])",
        false,
    );
    assert!(result.has_error());
    t.assert_contains_event("root_module_direct_deps and root_module_direct_dev_deps must both be specified or both be unspecified");
}

// Java: ModuleExtensionResolutionTest.extensionMetadata_exactlyOneArgIsNoneDev
#[test]
fn extension_metadata_exactly_one_arg_is_none_dev() {
    let mut t = new_test();
    let result = evaluate_simple_module_extension(
        &mut t,
        "return ctx.extension_metadata(root_module_direct_dev_deps=['foo'])",
        false,
    );
    assert!(result.has_error());
    t.assert_contains_event("root_module_direct_deps and root_module_direct_dev_deps must both be specified or both be unspecified");
}

// Java: ModuleExtensionResolutionTest.extensionMetadata_allUsedTwice
#[test]
fn extension_metadata_all_used_twice() {
    let mut t = new_test();
    let result = evaluate_simple_module_extension(
        &mut t,
        "return ctx.extension_metadata(root_module_direct_deps='all',root_module_direct_dev_deps='all')",
        false,
    );
    assert!(result.has_error());
    t.assert_contains_event(r#"if one of root_module_direct_deps and root_module_direct_dev_deps is "all", the other must be an empty list"#);
}

// Java: ModuleExtensionResolutionTest.extensionMetadata_allAndNone
#[test]
fn extension_metadata_all_and_none() {
    let mut t = new_test();
    let result = evaluate_simple_module_extension(
        &mut t,
        "return ctx.extension_metadata(root_module_direct_deps='all')",
        false,
    );
    assert!(result.has_error());
    t.assert_contains_event(r#"if one of root_module_direct_deps and root_module_direct_dev_deps is "all", the other must be an empty list"#);
}

// Java: ModuleExtensionResolutionTest.extensionMetadata_unsupportedString
#[test]
fn extension_metadata_unsupported_string() {
    let mut t = new_test();
    let result = evaluate_simple_module_extension(
        &mut t,
        "return ctx.extension_metadata(root_module_direct_deps='not_all')",
        false,
    );
    assert!(result.has_error());
    t.assert_contains_event(r#"root_module_direct_deps and root_module_direct_dev_deps must be None, "all", or a list of strings"#);
}

// Java: ModuleExtensionResolutionTest.extensionMetadata_unsupportedStringDev
#[test]
fn extension_metadata_unsupported_string_dev() {
    let mut t = new_test();
    let result = evaluate_simple_module_extension(
        &mut t,
        "return ctx.extension_metadata(root_module_direct_dev_deps='not_all')",
        false,
    );
    assert!(result.has_error());
    t.assert_contains_event(r#"root_module_direct_deps and root_module_direct_dev_deps must be None, "all", or a list of strings"#);
}

// Java: ModuleExtensionResolutionTest.extensionMetadata_invalidRepoName
#[test]
fn extension_metadata_invalid_repo_name() {
    let mut t = new_test();
    let result = evaluate_simple_module_extension(
        &mut t,
        "return ctx.extension_metadata(root_module_direct_deps=['+invalid'],root_module_direct_dev_deps=[])",
        false,
    );
    assert!(result.has_error());
    t.assert_contains_event("in root_module_direct_deps: invalid user-provided repo name '+invalid': valid names may contain only A-Z, a-z, 0-9, '-', '_', '.', and must start with a letter");
}

// Java: ModuleExtensionResolutionTest.extensionMetadata_invalidDevRepoName
#[test]
fn extension_metadata_invalid_dev_repo_name() {
    let mut t = new_test();
    let result = evaluate_simple_module_extension(
        &mut t,
        "return ctx.extension_metadata(root_module_direct_dev_deps=['+invalid'],root_module_direct_deps=[])",
        false,
    );
    assert!(result.has_error());
    t.assert_contains_event("in root_module_direct_dev_deps: invalid user-provided repo name '+invalid': valid names may contain only A-Z, a-z, 0-9, '-', '_', '.', and must start with a letter");
}

// Java: ModuleExtensionResolutionTest.extensionMetadata_duplicateRepo
#[test]
fn extension_metadata_duplicate_repo() {
    let mut t = new_test();
    let result = evaluate_simple_module_extension(
        &mut t,
        "return ctx.extension_metadata(root_module_direct_deps=['dep','dep'],root_module_direct_dev_deps=[])",
        false,
    );
    assert!(result.has_error());
    t.assert_contains_event("in root_module_direct_deps: duplicate entry 'dep'");
}

// Java: ModuleExtensionResolutionTest.extensionMetadata_duplicateDevRepo
#[test]
fn extension_metadata_duplicate_dev_repo() {
    let mut t = new_test();
    let result = evaluate_simple_module_extension(
        &mut t,
        "return ctx.extension_metadata(root_module_direct_deps=[],root_module_direct_dev_deps=['dep','dep'])",
        false,
    );
    assert!(result.has_error());
    t.assert_contains_event("in root_module_direct_dev_deps: duplicate entry 'dep'");
}

// Java: ModuleExtensionResolutionTest.extensionMetadata_duplicateRepoAcrossTypes
#[test]
fn extension_metadata_duplicate_repo_across_types() {
    let mut t = new_test();
    let result = evaluate_simple_module_extension(
        &mut t,
        "return ctx.extension_metadata(root_module_direct_deps=['dep'],root_module_direct_dev_deps=['dep'])",
        false,
    );
    assert!(result.has_error());
    t.assert_contains_event(
        "in root_module_direct_dev_deps: entry 'dep' is also in root_module_direct_deps",
    );
}

// Java: ModuleExtensionResolutionTest.extensionMetadata_devUsageWithAllDirectNonDevDeps
#[test]
fn extension_metadata_dev_usage_with_all_direct_non_dev_deps() {
    let mut t = new_test();
    let result = evaluate_simple_module_extension(
        &mut t,
        r#"return ctx.extension_metadata(root_module_direct_deps="all",root_module_direct_dev_deps=[])"#,
        true,
    );
    assert!(result.has_error());
    t.assert_contains_event("root_module_direct_deps must be empty if the root module contains no usages with dev_dependency = False");
}

// Java: ModuleExtensionResolutionTest.extensionMetadata_nonDevUsageWithAllDirectDevDeps
#[test]
fn extension_metadata_non_dev_usage_with_all_direct_dev_deps() {
    let mut t = new_test();
    let result = evaluate_simple_module_extension(
        &mut t,
        r#"return ctx.extension_metadata(root_module_direct_deps=[],root_module_direct_dev_deps="all")"#,
        false,
    );
    assert!(result.has_error());
    t.assert_contains_event("root_module_direct_dev_deps must be empty if the root module contains no usages with dev_dependency = True");
}

// Java: ModuleExtensionResolutionTest.extensionMetadata_devUsageWithDirectNonDevDeps
#[test]
fn extension_metadata_dev_usage_with_direct_non_dev_deps() {
    let mut t = new_test();
    let result = evaluate_simple_module_extension(
        &mut t,
        "return ctx.extension_metadata(root_module_direct_deps=['dep1'],root_module_direct_dev_deps=['dep2'])",
        true,
    );
    assert!(result.has_error());
    t.assert_contains_event("root_module_direct_deps must be empty if the root module contains no usages with dev_dependency = False");
}

// Java: ModuleExtensionResolutionTest.extensionMetadata_nonDevUsageWithDirectDevDeps
#[test]
fn extension_metadata_non_dev_usage_with_direct_dev_deps() {
    let mut t = new_test();
    let result = evaluate_simple_module_extension(
        &mut t,
        "return ctx.extension_metadata(root_module_direct_deps=['dep1'],root_module_direct_dev_deps=['dep2'])",
        false,
    );
    assert!(result.has_error());
    t.assert_contains_event("root_module_direct_dev_deps must be empty if the root module contains no usages with dev_dependency = True");
}

// Java: ModuleExtensionResolutionTest.extensionMetadata
#[test]
fn extension_metadata() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "bazel_dep(name='ext', version='1.0')",
            "bazel_dep(name='data_repo',version='1.0')",
            "ext = use_extension('@ext//:defs.bzl', 'ext')",
            "use_repo(",
            "  ext,",
            "  'indirect_dep',",
            "  'invalid_dep',",
            "  'dev_as_non_dev_dep',",
            "  my_direct_dep = 'direct_dep',",
            ")",
            "inject_repo(ext, my_data_repo = 'data_repo')",
            "ext_dev = use_extension('@ext//:defs.bzl', 'ext', dev_dependency = True)",
            "use_repo(",
            "  ext_dev,",
            "  'indirect_dev_dep',",
            "  'invalid_dev_dep',",
            "  'non_dev_as_dev_dep',",
            "  my_direct_dev_dep = 'direct_dev_dep',",
            ")",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &[
            "load('@my_direct_dep//:data.bzl', direct_dep_data='data')",
            "data = direct_dep_data",
        ],
    );
    t.registry().add_module(
        &create_module_key("ext", "1.0"),
        &[
            "module(name='ext',version='1.0')",
            "bazel_dep(name='data_repo',version='1.0')",
            "ext = use_extension('//:defs.bzl', 'ext')",
            "use_repo(ext, 'indirect_dep')",
            "ext_dev = use_extension('//:defs.bzl', 'ext', dev_dependency = True)",
            "use_repo(ext_dev, 'indirect_dev_dep')",
        ],
    );
    t.scratch.file(&module_path(&t, "ext+1.0/REPO.bazel"), &[]);
    t.scratch.file(&module_path(&t, "ext+1.0/BUILD"), &[]);
    t.scratch.file(&module_path(&t, "ext+1.0/defs.bzl"), &["load('@data_repo//:defs.bzl','data_repo')", "def _ext_impl(ctx):", "  data_repo(name='direct_dep')", "  data_repo(name='direct_dev_dep')", "  data_repo(name='missing_direct_dep')", "  data_repo(name='missing_direct_dev_dep')", "  data_repo(name='indirect_dep')", "  data_repo(name='indirect_dev_dep')", "  data_repo(name='dev_as_non_dev_dep')", "  data_repo(name='non_dev_as_dev_dep')", "  return ctx.extension_metadata(", "    root_module_direct_deps=['direct_dep', 'missing_direct_dep', 'non_dev_as_dev_dep'],", "    root_module_direct_dev_deps=['direct_dev_dep', 'missing_direct_dev_dep', 'dev_as_non_dev_dep'],", "  )", "ext=module_extension(implementation=_ext_impl)"]);
    t.invalidate_packages();
    // Evaluation fails due to the import of a repository not generated by the extension, but we
    // only want to assert that the warning is emitted.
    t.remove_fail_fast_handler();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    assert!(result.has_error());
    assert_event_count(&t, 1);
    t.assert_contains_event_of_kinds(&t.ws("WARNING /workspace/MODULE.bazel:3:20: The module extension ext defined in @ext//:defs.bzl reported incorrect imports of repositories via use_repo():

Imported, but not created by the extension (will cause the build to fail):
    invalid_dep, invalid_dev_dep

Not imported, but reported as direct dependencies by the extension (may cause the build to fail):
    missing_direct_dep, missing_direct_dev_dep

Imported as a regular dependency, but reported as a dev dependency by the extension (may cause the build to fail when used by other modules):
    dev_as_non_dev_dep

Imported as a dev dependency, but reported as a regular dependency by the extension (may cause the build to fail when used by other modules):
    non_dev_as_dev_dep

Imported, but reported as indirect dependencies by the extension:
    indirect_dep, indirect_dev_dep

Fix the use_repo calls by running 'bazel mod tidy'."), warning_kinds());
    let eval_value = done_eval_value(&mut t, "@@ext+//:defs.bzl", "ext");
    assert!(eval_value.fixup.is_some());
    assert_buildozer_commands(
        &eval_value,
        &[
            (
                "MODULE.bazel",
                "use_repo_add ext missing_direct_dep non_dev_as_dev_dep",
            ),
            (
                "MODULE.bazel",
                "use_repo_remove ext dev_as_non_dev_dep indirect_dep invalid_dep",
            ),
            (
                "MODULE.bazel",
                "use_repo_add ext_dev dev_as_non_dev_dep missing_direct_dev_dep",
            ),
            (
                "MODULE.bazel",
                "use_repo_remove ext_dev indirect_dev_dep invalid_dev_dep non_dev_as_dev_dep",
            ),
        ],
    );
    assert_eq!(
        eval_value.fixup.as_ref().unwrap().get_success_message(),
        "Updated use_repo calls for @ext//:defs.bzl%ext"
    );
}

// Java: ModuleExtensionResolutionTest.extensionMetadata_includes
#[test]
fn extension_metadata_includes() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "bazel_dep(name='ext', version='1.0')",
            "bazel_dep(name='data_repo',version='1.0')",
            "include('//:firstProd.MODULE.bazel')",
            "include('//:second.MODULE.bazel')",
        ],
    );
    t.scratch.file(
        "firstProd.MODULE.bazel",
        &[
            "ext = use_extension('@ext//:defs.bzl', 'ext')",
            "use_repo(",
            "  ext,",
            "  'indirect_dep',",
            "  'invalid_dep',",
            "  'dev_as_non_dev_dep',",
            "  my_direct_dep = 'direct_dep',",
            ")",
            "include('//:firstDev.MODULE.bazel')",
        ],
    );
    t.scratch.file(
        "firstDev.MODULE.bazel",
        &[
            "ext_dev = use_extension('@ext//:defs.bzl', 'ext', dev_dependency = True)",
            "use_repo(",
            "  ext_dev,",
            "  'indirect_dev_dep',",
            "  'invalid_dev_dep',",
            "  'non_dev_as_dev_dep',",
            "  my_direct_dev_dep = 'direct_dev_dep',",
            ")",
        ],
    );
    t.scratch.file(
        "second.MODULE.bazel",
        &[
            "ext = use_extension('@ext//:defs.bzl', 'ext')",
            "use_repo(ext, 'invalid_dep2')",
            "ext_dev = use_extension('@ext//:defs.bzl', 'ext', dev_dependency = True)",
            "use_repo(ext_dev, 'invalid_dev_dep2')",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &[
            "load('@my_direct_dep//:data.bzl', direct_dep_data='data')",
            "data = direct_dep_data",
        ],
    );
    t.registry().add_module(
        &create_module_key("ext", "1.0"),
        &[
            "module(name='ext',version='1.0')",
            "bazel_dep(name='data_repo',version='1.0')",
            "ext = use_extension('//:defs.bzl', 'ext')",
            "use_repo(ext, 'indirect_dep')",
            "ext_dev = use_extension('//:defs.bzl', 'ext', dev_dependency = True)",
            "use_repo(ext_dev, 'indirect_dev_dep')",
        ],
    );
    t.scratch.file(&module_path(&t, "ext+1.0/REPO.bazel"), &[]);
    t.scratch.file(&module_path(&t, "ext+1.0/BUILD"), &[]);
    t.scratch.file(&module_path(&t, "ext+1.0/defs.bzl"), &["load('@data_repo//:defs.bzl','data_repo')", "def _ext_impl(ctx):", "  data_repo(name='direct_dep')", "  data_repo(name='direct_dev_dep')", "  data_repo(name='missing_direct_dep')", "  data_repo(name='missing_direct_dev_dep')", "  data_repo(name='indirect_dep')", "  data_repo(name='indirect_dev_dep')", "  data_repo(name='dev_as_non_dev_dep')", "  data_repo(name='non_dev_as_dev_dep')", "  return ctx.extension_metadata(", "    root_module_direct_deps=['direct_dep', 'missing_direct_dep', 'non_dev_as_dev_dep'],", "    root_module_direct_dev_deps=['direct_dev_dep', 'missing_direct_dev_dep', 'dev_as_non_dev_dep'],", "  )", "ext=module_extension(implementation=_ext_impl)"]);
    t.invalidate_packages();
    // Evaluation fails due to the import of a repository not generated by the extension, but we
    // only want to assert that the warning is emitted.
    t.remove_fail_fast_handler();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    assert!(result.has_error());
    assert_event_count(&t, 1);
    t.assert_contains_event_of_kinds(&t.ws("WARNING /workspace/firstProd.MODULE.bazel:1:20: The module extension ext defined in @ext//:defs.bzl reported incorrect imports of repositories via use_repo():

Imported, but not created by the extension (will cause the build to fail):
    invalid_dep, invalid_dep2, invalid_dev_dep, invalid_dev_dep2

Not imported, but reported as direct dependencies by the extension (may cause the build to fail):
    missing_direct_dep, missing_direct_dev_dep

Imported as a regular dependency, but reported as a dev dependency by the extension (may cause the build to fail when used by other modules):
    dev_as_non_dev_dep

Imported as a dev dependency, but reported as a regular dependency by the extension (may cause the build to fail when used by other modules):
    non_dev_as_dev_dep

Imported, but reported as indirect dependencies by the extension:
    indirect_dep, indirect_dev_dep

Fix the use_repo calls by running 'bazel mod tidy'."), warning_kinds());
    let eval_value = done_eval_value(&mut t, "@@ext+//:defs.bzl", "ext");
    assert!(eval_value.fixup.is_some());
    assert_buildozer_commands(
        &eval_value,
        &[
            (
                "firstProd.MODULE.bazel",
                "use_repo_add ext missing_direct_dep non_dev_as_dev_dep",
            ),
            (
                "firstProd.MODULE.bazel",
                "use_repo_remove ext dev_as_non_dev_dep indirect_dep invalid_dep",
            ),
            ("second.MODULE.bazel", "use_repo_remove ext invalid_dep2"),
            (
                "firstDev.MODULE.bazel",
                "use_repo_add ext_dev dev_as_non_dev_dep missing_direct_dev_dep",
            ),
            (
                "firstDev.MODULE.bazel",
                "use_repo_remove ext_dev indirect_dev_dep invalid_dev_dep non_dev_as_dev_dep",
            ),
            (
                "second.MODULE.bazel",
                "use_repo_remove ext_dev invalid_dev_dep2",
            ),
        ],
    );
    assert_eq!(
        eval_value.fixup.as_ref().unwrap().get_success_message(),
        "Updated use_repo calls for @ext//:defs.bzl%ext"
    );
}

// Java: ModuleExtensionResolutionTest.extensionMetadata_all
#[test]
fn extension_metadata_all() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "bazel_dep(name='ext', version='1.0')",
            "bazel_dep(name='data_repo',version='1.0')",
            "ext = use_extension('@ext//:defs.bzl', 'ext')",
            "use_repo(ext, 'direct_dep', 'indirect_dep', 'invalid_dep')",
            "ext_dev = use_extension('@ext//:defs.bzl', 'ext', dev_dependency = True)",
            "use_repo(ext_dev, 'direct_dev_dep', 'indirect_dev_dep', 'invalid_dev_dep')",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &[
            "load('@direct_dep//:data.bzl', direct_dep_data='data')",
            "data = direct_dep_data",
        ],
    );
    t.registry().add_module(
        &create_module_key("ext", "1.0"),
        &[
            "module(name='ext',version='1.0')",
            "bazel_dep(name='data_repo',version='1.0')",
            "ext = use_extension('//:defs.bzl', 'ext')",
            "use_repo(ext, 'indirect_dep')",
            "ext_dev = use_extension('//:defs.bzl', 'ext', dev_dependency = True)",
            "use_repo(ext_dev, 'indirect_dev_dep')",
        ],
    );
    t.scratch.file(&module_path(&t, "ext+1.0/REPO.bazel"), &[]);
    t.scratch.file(&module_path(&t, "ext+1.0/BUILD"), &[]);
    t.scratch.file(
        &module_path(&t, "ext+1.0/defs.bzl"),
        &[
            "load('@data_repo//:defs.bzl','data_repo')",
            "def _ext_impl(ctx):",
            "  data_repo(name='direct_dep')",
            "  data_repo(name='direct_dev_dep')",
            "  data_repo(name='missing_direct_dep')",
            "  data_repo(name='missing_direct_dev_dep')",
            "  data_repo(name='indirect_dep')",
            "  data_repo(name='indirect_dev_dep')",
            "  return ctx.extension_metadata(",
            "    root_module_direct_deps='all',",
            "    root_module_direct_dev_deps=[],",
            "  )",
            "ext=module_extension(implementation=_ext_impl)",
        ],
    );
    t.invalidate_packages();
    t.remove_fail_fast_handler();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    assert!(result.has_error());
    assert_eq!(result.error_message(), t.ws(r#"module extension @@ext+//:defs.bzl%ext does not generate repository "invalid_dep", yet it is imported as "invalid_dep" in the usage at /workspace/MODULE.bazel:3:20"#));
    assert_event_count(&t, 1);
    t.assert_contains_event_of_kinds(&t.ws("WARNING /workspace/MODULE.bazel:3:20: The module extension ext defined in @ext//:defs.bzl reported incorrect imports of repositories via use_repo():

Imported, but not created by the extension (will cause the build to fail):
    invalid_dep, invalid_dev_dep

Not imported, but reported as direct dependencies by the extension (may cause the build to fail):
    missing_direct_dep, missing_direct_dev_dep

Imported as a dev dependency, but reported as a regular dependency by the extension (may cause the build to fail when used by other modules):
    direct_dev_dep, indirect_dev_dep

Fix the use_repo calls by running 'bazel mod tidy'."), warning_kinds());
    let eval_value = done_eval_value(&mut t, "@@ext+//:defs.bzl", "ext");
    assert!(eval_value.fixup.is_some());
    assert_buildozer_commands(
        &eval_value,
        &[
            (
                "MODULE.bazel",
                "use_repo_add ext direct_dev_dep indirect_dev_dep missing_direct_dep missing_direct_dev_dep",
            ),
            ("MODULE.bazel", "use_repo_remove ext invalid_dep"),
            (
                "MODULE.bazel",
                "use_repo_remove ext_dev direct_dev_dep indirect_dev_dep invalid_dev_dep",
            ),
        ],
    );
    assert_eq!(
        eval_value.fixup.as_ref().unwrap().get_success_message(),
        "Updated use_repo calls for @ext//:defs.bzl%ext"
    );
}

// Java: ModuleExtensionResolutionTest.extensionMetadata_allDev
#[test]
fn extension_metadata_all_dev() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "bazel_dep(name='ext', version='1.0')",
            "bazel_dep(name='data_repo',version='1.0')",
            "ext = use_extension('@ext//:defs.bzl', 'ext')",
            "use_repo(ext, 'direct_dep', 'indirect_dep', 'invalid_dep')",
            "ext_dev = use_extension('@ext//:defs.bzl', 'ext', dev_dependency = True)",
            "use_repo(ext_dev, 'direct_dev_dep', 'indirect_dev_dep', 'invalid_dev_dep')",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &[
            "load('@direct_dep//:data.bzl', direct_dep_data='data')",
            "data = direct_dep_data",
        ],
    );
    t.registry().add_module(
        &create_module_key("ext", "1.0"),
        &[
            "module(name='ext',version='1.0')",
            "bazel_dep(name='data_repo',version='1.0')",
            "ext = use_extension('//:defs.bzl', 'ext')",
            "use_repo(ext, 'indirect_dep')",
            "ext_dev = use_extension('//:defs.bzl', 'ext', dev_dependency = True)",
            "use_repo(ext_dev, 'indirect_dev_dep')",
        ],
    );
    t.scratch.file(&module_path(&t, "ext+1.0/REPO.bazel"), &[]);
    t.scratch.file(&module_path(&t, "ext+1.0/BUILD"), &[]);
    t.scratch.file(
        &module_path(&t, "ext+1.0/defs.bzl"),
        &[
            "load('@data_repo//:defs.bzl','data_repo')",
            "def _ext_impl(ctx):",
            "  data_repo(name='direct_dep')",
            "  data_repo(name='direct_dev_dep')",
            "  data_repo(name='missing_direct_dep')",
            "  data_repo(name='missing_direct_dev_dep')",
            "  data_repo(name='indirect_dep')",
            "  data_repo(name='indirect_dev_dep')",
            "  return ctx.extension_metadata(",
            "    root_module_direct_deps=[],",
            "    root_module_direct_dev_deps='all',",
            "  )",
            "ext=module_extension(implementation=_ext_impl)",
        ],
    );
    t.invalidate_packages();
    // Evaluation fails due to the import of a repository not generated by the extension, but we
    // only want to assert that the warning is emitted.
    t.remove_fail_fast_handler();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    assert!(result.has_error());
    assert_eq!(result.error_message(), t.ws(r#"module extension @@ext+//:defs.bzl%ext does not generate repository "invalid_dep", yet it is imported as "invalid_dep" in the usage at /workspace/MODULE.bazel:3:20"#));
    assert_event_count(&t, 1);
    t.assert_contains_event_of_kinds(&t.ws("WARNING /workspace/MODULE.bazel:3:20: The module extension ext defined in @ext//:defs.bzl reported incorrect imports of repositories via use_repo():

Imported, but not created by the extension (will cause the build to fail):
    invalid_dep, invalid_dev_dep

Not imported, but reported as direct dependencies by the extension (may cause the build to fail):
    missing_direct_dep, missing_direct_dev_dep

Imported as a regular dependency, but reported as a dev dependency by the extension (may cause the build to fail when used by other modules):
    direct_dep, indirect_dep

Fix the use_repo calls by running 'bazel mod tidy'."), warning_kinds());
    let eval_value = done_eval_value(&mut t, "@@ext+//:defs.bzl", "ext");
    assert!(eval_value.fixup.is_some());
    assert_buildozer_commands(
        &eval_value,
        &[
            (
                "MODULE.bazel",
                "use_repo_remove ext direct_dep indirect_dep invalid_dep",
            ),
            (
                "MODULE.bazel",
                "use_repo_add ext_dev direct_dep indirect_dep missing_direct_dep missing_direct_dev_dep",
            ),
            ("MODULE.bazel", "use_repo_remove ext_dev invalid_dev_dep"),
        ],
    );
    assert_eq!(
        eval_value.fixup.as_ref().unwrap().get_success_message(),
        "Updated use_repo calls for @ext//:defs.bzl%ext"
    );
}

// Java: ModuleExtensionResolutionTest.extensionMetadata_noRootUsage
#[test]
fn extension_metadata_no_root_usage() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "bazel_dep(name='ext', version='1.0')",
            "bazel_dep(name='data_repo',version='1.0')",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.registry().add_module(
        &create_module_key("ext", "1.0"),
        &[
            "module(name='ext',version='1.0')",
            "bazel_dep(name='data_repo',version='1.0')",
            "ext = use_extension('//:defs.bzl', 'ext')",
            "use_repo(ext, 'indirect_dep')",
            "ext_dev = use_extension('//:defs.bzl', 'ext', dev_dependency = True)",
            "use_repo(ext_dev, 'indirect_dev_dep')",
        ],
    );
    t.scratch.file(&module_path(&t, "ext+1.0/REPO.bazel"), &[]);
    t.scratch.file(&module_path(&t, "ext+1.0/BUILD"), &[]);
    t.scratch.file(
        &module_path(&t, "ext+1.0/defs.bzl"),
        &[
            "load('@data_repo//:defs.bzl','data_repo')",
            "def _ext_impl(ctx):",
            "  data_repo(name='direct_dep')",
            "  data_repo(name='direct_dev_dep')",
            "  data_repo(name='missing_direct_dep')",
            "  data_repo(name='missing_direct_dev_dep')",
            "  data_repo(name='indirect_dep', data='indirect_dep_data')",
            "  data_repo(name='indirect_dev_dep')",
            "  return ctx.extension_metadata(",
            "    root_module_direct_deps='all',",
            "    root_module_direct_dev_deps=[],",
            "  )",
            "ext=module_extension(implementation=_ext_impl)",
        ],
    );
    t.scratch.file(
        &module_path(&t, "ext+1.0/data.bzl"),
        &[
            "load('@indirect_dep//:data.bzl', indirect_dep_data='data')",
            "data = indirect_dep_data",
        ],
    );
    t.invalidate_packages();
    let result = evaluate_bzl(&mut t, "@ext+//:data.bzl");
    assert_eq!(result.global_str("data"), "indirect_dep_data");
    assert_event_count(&t, 0);
    let eval_value = done_eval_value(&mut t, "@@ext+//:defs.bzl", "ext");
    assert!(eval_value.fixup.is_none());
}

// Java: ModuleExtensionResolutionTest.extensionMetadata_isolated
#[test]
fn extension_metadata_isolated() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "bazel_dep(name='ext', version='1.0')",
            "bazel_dep(name='data_repo',version='1.0')",
            "ext1 = use_extension('@ext//:defs.bzl', 'ext', isolate = True)",
            "use_repo(",
            "  ext1,",
            "  'indirect_dep',",
            ")",
            "ext2 = use_extension('@ext//:defs.bzl', 'ext', isolate = True)",
            "use_repo(",
            "  ext2,",
            "  'direct_dep',",
            ")",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &[
            "load('@direct_dep//:data.bzl', data_1='data')",
            "load('@indirect_dep//:data.bzl', data_2='data')",
        ],
    );
    t.registry().add_module(
        &create_module_key("ext", "1.0"),
        &[
            "module(name='ext',version='1.0')",
            "bazel_dep(name='data_repo',version='1.0')",
            "ext = use_extension('//:defs.bzl', 'ext')",
            "use_repo(ext, 'indirect_dep')",
        ],
    );
    t.scratch.file(&module_path(&t, "ext+1.0/REPO.bazel"), &[]);
    t.scratch.file(&module_path(&t, "ext+1.0/BUILD"), &[]);
    t.scratch.file(
        &module_path(&t, "ext+1.0/defs.bzl"),
        &[
            "load('@data_repo//:defs.bzl','data_repo')",
            "def _ext_impl(ctx):",
            "  data_repo(name='direct_dep')",
            "  data_repo(name='missing_direct_dep')",
            "  data_repo(name='indirect_dep')",
            "  return ctx.extension_metadata(",
            "    root_module_direct_deps=['direct_dep', 'missing_direct_dep'],",
            "    root_module_direct_dev_deps=[],",
            "  )",
            "ext=module_extension(implementation=_ext_impl)",
        ],
    );
    t.set_build_language_options(&["--experimental_isolated_extension_usages"]);
    t.invalidate_packages();
    // Evaluation fails due to the import of a repository not generated by the extension, but we
    // only want to assert that the warning is emitted.
    t.remove_fail_fast_handler();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    result.throw_if_error();
    assert_event_count(&t, 2);
    t.assert_contains_event_of_kinds(&t.ws("WARNING /workspace/MODULE.bazel:3:21: The module extension ext defined in @ext//:defs.bzl reported incorrect imports of repositories via use_repo():

Not imported, but reported as direct dependencies by the extension (may cause the build to fail):
    direct_dep, missing_direct_dep

Imported, but reported as indirect dependencies by the extension:
    indirect_dep

Fix the use_repo calls by running 'bazel mod tidy'."), warning_kinds());
    t.assert_contains_event_of_kinds(&t.ws("WARNING /workspace/MODULE.bazel:8:21: The module extension ext defined in @ext//:defs.bzl reported incorrect imports of repositories via use_repo():

Not imported, but reported as direct dependencies by the extension (may cause the build to fail):
    missing_direct_dep

Fix the use_repo calls by running 'bazel mod tidy'."), warning_kinds());
    let ext1_value = done_isolated_eval_value(&mut t, "@@ext+//:defs.bzl", "ext", "ext1");
    assert!(ext1_value.fixup.is_some());
    assert_buildozer_commands(
        &ext1_value,
        &[
            (
                "MODULE.bazel",
                "use_repo_add ext1 direct_dep missing_direct_dep",
            ),
            ("MODULE.bazel", "use_repo_remove ext1 indirect_dep"),
        ],
    );
    assert_eq!(
        ext1_value.fixup.as_ref().unwrap().get_success_message(),
        "Updated use_repo calls for isolated usage 'ext1' of @ext//:defs.bzl%ext"
    );
    let ext2_value = done_isolated_eval_value(&mut t, "@@ext+//:defs.bzl", "ext", "ext2");
    assert!(ext2_value.fixup.is_some());
    assert_buildozer_commands(
        &ext2_value,
        &[("MODULE.bazel", "use_repo_add ext2 missing_direct_dep")],
    );
    assert_eq!(
        ext2_value.fixup.as_ref().unwrap().get_success_message(),
        "Updated use_repo calls for isolated usage 'ext2' of @ext//:defs.bzl%ext"
    );
}

// Java: ModuleExtensionResolutionTest.extensionMetadata_isolatedDev
#[test]
fn extension_metadata_isolated_dev() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "bazel_dep(name='ext', version='1.0')",
            "bazel_dep(name='data_repo',version='1.0')",
            "ext1 = use_extension('@ext//:defs.bzl', 'ext', isolate = True, dev_dependency = True)",
            "use_repo(",
            "  ext1,",
            "  'indirect_dep',",
            ")",
            "ext2 = use_extension('@ext//:defs.bzl', 'ext', isolate = True, dev_dependency = True)",
            "use_repo(",
            "  ext2,",
            "  'direct_dep',",
            ")",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &[
            "load('@direct_dep//:data.bzl', data_1='data')",
            "load('@indirect_dep//:data.bzl', data_2='data')",
        ],
    );
    t.registry().add_module(
        &create_module_key("ext", "1.0"),
        &[
            "module(name='ext',version='1.0')",
            "bazel_dep(name='data_repo',version='1.0')",
            "ext = use_extension('//:defs.bzl', 'ext')",
            "use_repo(ext, 'indirect_dep')",
        ],
    );
    t.scratch.file(&module_path(&t, "ext+1.0/REPO.bazel"), &[]);
    t.scratch.file(&module_path(&t, "ext+1.0/BUILD"), &[]);
    t.scratch.file(
        &module_path(&t, "ext+1.0/defs.bzl"),
        &[
            "load('@data_repo//:defs.bzl','data_repo')",
            "def _ext_impl(ctx):",
            "  data_repo(name='direct_dep')",
            "  data_repo(name='missing_direct_dep')",
            "  data_repo(name='indirect_dep')",
            "  return ctx.extension_metadata(",
            "    root_module_direct_deps=[],",
            "    root_module_direct_dev_deps=['direct_dep', 'missing_direct_dep'],",
            "  )",
            "ext=module_extension(implementation=_ext_impl)",
        ],
    );
    t.set_build_language_options(&["--experimental_isolated_extension_usages"]);
    t.invalidate_packages();
    // Evaluation fails due to the import of a repository not generated by the extension, but we
    // only want to assert that the warning is emitted.
    t.remove_fail_fast_handler();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    result.throw_if_error();
    assert_event_count(&t, 2);
    t.assert_contains_event_of_kinds(&t.ws("WARNING /workspace/MODULE.bazel:3:21: The module extension ext defined in @ext//:defs.bzl reported incorrect imports of repositories via use_repo():

Not imported, but reported as direct dependencies by the extension (may cause the build to fail):
    direct_dep, missing_direct_dep

Imported, but reported as indirect dependencies by the extension:
    indirect_dep

Fix the use_repo calls by running 'bazel mod tidy'."), warning_kinds());
    t.assert_contains_event_of_kinds(&t.ws("WARNING /workspace/MODULE.bazel:8:21: The module extension ext defined in @ext//:defs.bzl reported incorrect imports of repositories via use_repo():

Not imported, but reported as direct dependencies by the extension (may cause the build to fail):
    missing_direct_dep

Fix the use_repo calls by running 'bazel mod tidy'."), warning_kinds());
    let ext1_value = done_isolated_eval_value(&mut t, "@@ext+//:defs.bzl", "ext", "ext1");
    assert!(ext1_value.fixup.is_some());
    assert_buildozer_commands(
        &ext1_value,
        &[
            (
                "MODULE.bazel",
                "use_repo_add ext1 direct_dep missing_direct_dep",
            ),
            ("MODULE.bazel", "use_repo_remove ext1 indirect_dep"),
        ],
    );
    assert_eq!(
        ext1_value.fixup.as_ref().unwrap().get_success_message(),
        "Updated use_repo calls for isolated usage 'ext1' of @ext//:defs.bzl%ext"
    );
    let ext2_value = done_isolated_eval_value(&mut t, "@@ext+//:defs.bzl", "ext", "ext2");
    assert!(ext2_value.fixup.is_some());
    assert_buildozer_commands(
        &ext2_value,
        &[("MODULE.bazel", "use_repo_add ext2 missing_direct_dep")],
    );
    assert_eq!(
        ext2_value.fixup.as_ref().unwrap().get_success_message(),
        "Updated use_repo calls for isolated usage 'ext2' of @ext//:defs.bzl%ext"
    );
}

// Java: ModuleExtensionResolutionTest.facts_supportedTypes
#[test]
#[ignore = "not ported yet: compares the facts Dict with a Java map of Starlark values (StarlarkInt, StarlarkFloat, StarlarkList, Dict)"]
fn facts_supported_types() {}

// Java: ModuleExtensionResolutionTest.facts_unsupportedType
#[test]
fn facts_unsupported_type() {
    let mut t = new_test();
    let result = evaluate_simple_module_extension(
        &mut t,
        r#"return ctx.extension_metadata(
    facts = {
        "unsupported": set([1, 2, 3]),
    }
)"#,
        false,
    );
    assert!(result.has_error());
    t.assert_contains_event("'set([1, 2, 3])' (set) is not supported in facts");
}

// Java: ModuleExtensionResolutionTest.facts_nonStringKeys
#[test]
fn facts_non_string_keys() {
    let mut t = new_test();
    let result = evaluate_simple_module_extension(
        &mut t,
        r#"return ctx.extension_metadata(
    facts = {
        "top_level": {
            1: "one",
        },
    }
)"#,
        false,
    );
    assert!(result.has_error());
    t.assert_contains_event(r#"Facts keys must be strings, got '1: "one"' (int)"#);
}

// Java: ModuleExtensionResolutionTest.facts_nestedTooDeeply
#[test]
fn facts_nested_too_deeply() {
    let mut t = new_test();
    let result = evaluate_simple_module_extension(
        &mut t,
        r#"return ctx.extension_metadata(
    facts = {
        "nested": {
            "too": {
                "deep": {
                    "to": {
                        "be": {
                            "considered": {
                                "valid": [1, 2, 3]
                            }
                        }
                    }
                }
            }
        }
    }
)"#,
        false,
    );
    assert!(result.has_error());
    t.assert_contains_event("Facts cannot be nested more than 7 levels deep");
}

// Java: ModuleExtensionResolutionTest.requireRepoExtensionMetadata_defaultAllowsModuleExtensionDefault
#[test]
fn require_repo_extension_metadata_default_allows_module_extension_default() {
    let mut t = new_test();
    let result = evaluate_simple_module_extension(&mut t, "", false);
    result.throw_if_error();
}

// Java: ModuleExtensionResolutionTest.requireRepoExtensionMetadata_allErrorsForModuleExtensionDefault
#[test]
fn require_repo_extension_metadata_all_errors_for_module_extension_default() {
    let mut t = new_test();
    set_require_repo_extension_metadata_mode(&mut t, RequireRepoExtensionMetadataMode::All);
    t.remove_fail_fast_handler();
    let result = evaluate_simple_module_extension(&mut t, "", false);
    assert!(result.has_error());
    assert_contains(&result.error_message(), "did not return extension_metadata");
    assert_contains(
        &result.error_message(),
        "--incompatible_require_repo_extension_metadata=all requires it",
    );
}

// Java: ModuleExtensionResolutionTest.requireRepoExtensionMetadata_rootErrorsForRootModuleExtensionDefault
#[test]
fn require_repo_extension_metadata_root_errors_for_root_module_extension_default() {
    let mut t = new_test();
    set_require_repo_extension_metadata_mode(&mut t, RequireRepoExtensionMetadataMode::Root);
    t.remove_fail_fast_handler();
    let result = evaluate_simple_module_extension(&mut t, "", false);
    assert!(result.has_error());
    assert_contains(&result.error_message(), "did not return extension_metadata");
    assert_contains(
        &result.error_message(),
        "--incompatible_require_repo_extension_metadata=root requires it",
    );
}

// Java: ModuleExtensionResolutionTest.requireRepoExtensionMetadata_rootAllowsDepDefinedModuleExtensionUsedByRootModule
#[test]
fn require_repo_extension_metadata_root_allows_dep_defined_module_extension_used_by_root_module() {
    let mut t = new_test();
    set_require_repo_extension_metadata_mode(&mut t, RequireRepoExtensionMetadataMode::Root);
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "module(name='root')",
            "bazel_dep(name='foo', version='1.0')",
            "ext = use_extension('@foo//:defs.bzl', 'ext')",
        ],
    );
    t.registry().add_module(
        &create_module_key("foo", "1.0"),
        &["module(name='foo', version='1.0')"],
    );
    t.scratch.file(&module_path(&t, "foo+1.0/REPO.bazel"), &[]);
    t.scratch.file(&module_path(&t, "foo+1.0/BUILD"), &[]);
    t.scratch.file(
        &module_path(&t, "foo+1.0/defs.bzl"),
        &[
            "def _ext_impl(ctx):",
            "  pass",
            "ext = module_extension(implementation=_ext_impl)",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.invalidate_packages();
    let result = evaluate_extension(&mut t, "@@foo+//:defs.bzl", "ext");
    result.throw_if_error();
}

// Java: ModuleExtensionResolutionTest.requireRepoExtensionMetadata_allAllowsExplicitModuleExtensionMetadata
#[test]
fn require_repo_extension_metadata_all_allows_explicit_module_extension_metadata() {
    let mut t = new_test();
    set_require_repo_extension_metadata_mode(&mut t, RequireRepoExtensionMetadataMode::All);
    let result = evaluate_simple_module_extension(&mut t, "return ctx.extension_metadata()", false);
    result.throw_if_error();
}

// Java: ModuleExtensionResolutionTest.requireRepoExtensionMetadata_allErrorsForRepoRuleDefault
#[test]
fn require_repo_extension_metadata_all_errors_for_repo_rule_default() {
    let mut t = new_test();
    set_require_repo_extension_metadata_mode(&mut t, RequireRepoExtensionMetadataMode::All);
    setup_root_use_repo_rule_returning(&mut t, "");
    t.remove_fail_fast_handler();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    assert!(result.has_error());
    t.assert_contains_event("did not return repo_metadata");
    t.assert_contains_event("--incompatible_require_repo_extension_metadata=all requires it");
}

// Java: ModuleExtensionResolutionTest.requireRepoExtensionMetadata_rootErrorsForRootRepoRuleDefault
#[test]
fn require_repo_extension_metadata_root_errors_for_root_repo_rule_default() {
    let mut t = new_test();
    set_require_repo_extension_metadata_mode(&mut t, RequireRepoExtensionMetadataMode::Root);
    setup_root_use_repo_rule_returning(&mut t, "");
    t.remove_fail_fast_handler();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    assert!(result.has_error());
    t.assert_contains_event("did not return repo_metadata");
    t.assert_contains_event("--incompatible_require_repo_extension_metadata=root requires it");
}

// Java: ModuleExtensionResolutionTest.requireRepoExtensionMetadata_rootAllowsDepDefinedRepoRuleUsedByRootModule
#[test]
fn require_repo_extension_metadata_root_allows_dep_defined_repo_rule_used_by_root_module() {
    let mut t = new_test();
    set_require_repo_extension_metadata_mode(&mut t, RequireRepoExtensionMetadataMode::Root);
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "module(name='root')",
            "bazel_dep(name='foo', version='1.0')",
            "repo = use_repo_rule('@foo//:repo.bzl', 'repo')",
            "repo(name = 'gen')",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &[
            "load('@gen//:data.bzl', repo_data = 'data')",
            "data = repo_data",
        ],
    );
    t.registry().add_module(
        &create_module_key("foo", "1.0"),
        &["module(name='foo', version='1.0')"],
    );
    t.scratch.file(&module_path(&t, "foo+1.0/REPO.bazel"), &[]);
    t.scratch.file(&module_path(&t, "foo+1.0/BUILD"), &[]);
    t.scratch.file(
        &module_path(&t, "foo+1.0/repo.bzl"),
        &[
            "def _repo_impl(ctx):",
            "  ctx.file('BUILD')",
            r#"  ctx.file('data.bzl', "data = 'ok'")"#,
            "repo = repository_rule(implementation = _repo_impl)",
        ],
    );
    t.invalidate_packages();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    result.throw_if_error();
}

// Java: ModuleExtensionResolutionTest.requireRepoExtensionMetadata_allAllowsExplicitRepoMetadata
#[test]
fn require_repo_extension_metadata_all_allows_explicit_repo_metadata() {
    let mut t = new_test();
    set_require_repo_extension_metadata_mode(&mut t, RequireRepoExtensionMetadataMode::All);
    setup_root_use_repo_rule_returning(&mut t, "return ctx.repo_metadata(reproducible = True)");
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    result.throw_if_error();
}

// Java: ModuleExtensionResolutionTest.requireRepoExtensionMetadata_allAllowsLegacyRepoMetadataDict
#[test]
fn require_repo_extension_metadata_all_allows_legacy_repo_metadata_dict() {
    let mut t = new_test();
    set_require_repo_extension_metadata_mode(&mut t, RequireRepoExtensionMetadataMode::All);
    setup_root_use_repo_rule_returning(&mut t, "return {}");
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    result.throw_if_error();
}

// Java: ModuleExtensionResolutionTest.isDevDependency_usages
#[test]
fn is_dev_dependency_usages() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "module(name='root',version='1.0')",
            "bazel_dep(name='data_repo',version='1.0')",
            "ext1 = use_extension('//:defs.bzl','ext1')",
            "use_repo(ext1,ext1_repo='ext_repo')",
            "ext2 = use_extension('//:defs.bzl','ext2',dev_dependency=True)",
            "use_repo(ext2,ext2_repo='ext_repo')",
            "ext3a = use_extension('//:defs.bzl','ext3')",
            "use_repo(ext3a,ext3_repo='ext_repo')",
            "ext3b = use_extension('//:defs.bzl','ext3',dev_dependency=True)",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &[
            "load('@ext1_repo//:data.bzl', _ext1_data='data')",
            "load('@ext2_repo//:data.bzl', _ext2_data='data')",
            "load('@ext3_repo//:data.bzl', _ext3_data='data')",
            "ext1_data=_ext1_data",
            "ext2_data=_ext2_data",
            "ext3_data=_ext3_data",
        ],
    );
    t.scratch.file(
        "defs.bzl",
        &[
            "load('@data_repo//:defs.bzl','data_repo')",
            "def _ext_impl(id,ctx):",
            "  data_str = id + ': ' + str(ctx.root_module_has_non_dev_dependency)",
            "  data_repo(name='ext_repo',data=data_str)",
            "ext1=module_extension(implementation=lambda ctx: _ext_impl('ext1', ctx))",
            "ext2=module_extension(implementation=lambda ctx: _ext_impl('ext2', ctx))",
            "ext3=module_extension(implementation=lambda ctx: _ext_impl('ext3', ctx))",
        ],
    );
    t.invalidate_packages();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    result.throw_if_error();
    assert_eq!(result.global_str("ext1_data"), "ext1: True");
    assert_eq!(result.global_str("ext2_data"), "ext2: False");
    assert_eq!(result.global_str("ext3_data"), "ext3: True");
}

// Java: ModuleExtensionResolutionTest.printAndFailOnTag
#[test]
fn print_and_fail_on_tag() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "ext = use_extension('//:defs.bzl', 'ext')",
            "ext.foo()",
            "ext.foo()",
        ],
    );
    t.scratch.file(
        "defs.bzl",
        &[
            "repo = repository_rule(lambda ctx: True)",
            "def _ext_impl(ctx):",
            "  tag1 = ctx.modules[0].tags.foo[0]",
            "  tag2 = ctx.modules[0].tags.foo[1]",
            "  print('Conflict between', tag1, 'and', tag2)",
            "  fail('Fatal conflict between', tag1, 'and', tag2)",
            "foo = tag_class()",
            "ext = module_extension(implementation=_ext_impl,tag_classes={'foo':foo})",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.invalidate_packages();
    t.remove_fail_fast_handler();
    let result = evaluate_extension(&mut t, "//:defs.bzl", "ext");
    assert!(result.has_error());
    t.assert_contains_event_of_kinds(&t.ws("Fatal conflict between 'foo' tag at /workspace/MODULE.bazel:2:8 and 'foo' tag at /workspace/MODULE.bazel:3:8"), EventKindSet::of(&[EventKind::Error]));
    t.assert_contains_event_of_kinds(&t.ws("Conflict between 'foo' tag at /workspace/MODULE.bazel:2:8 and 'foo' tag at /workspace/MODULE.bazel:3:8"), EventKindSet::of(&[EventKind::Debug]));
}

// Java: ModuleExtensionResolutionTest.tagSortOrder
#[test]
fn tag_sort_order() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "module(name='root',version='1.0')",
            "bazel_dep(name='foo',version='1.0')",
            "bazel_dep(name='data_repo',version='1.0')",
            "ext = use_extension('//:defs.bzl', 'ext')",
            "ext.baz(id = 7)",
            "ext.foo(id = 2)",
            "ext.baz(id = 9)",
            "ext.bar(id = 42)",
            "ext.foo(id = -1)",
            "ext.foo(id = 5)",
            "use_repo(ext, 'ext_data')",
        ],
    );
    t.scratch.file("defs.bzl", &["load('@data_repo//:defs.bzl','data_repo')", "def _ext_impl(ctx):", "  tags = []", "  for module in ctx.modules:", "    tags += module.tags.foo", "    tags += module.tags.bar", "    tags += module.tags.baz", "  ids = [tag.id for tag in sorted(tags, key=lambda tag: ctx.tag_sort_key(tag))]", "  data_repo(name='ext_data',data=str(ids))", "foo = tag_class(attrs = {'id': attr.int()})", "bar = tag_class(attrs = {'id': attr.int()})", "baz = tag_class(attrs = {'id': attr.int()})", "ext = module_extension(implementation=_ext_impl,tag_classes={'foo':foo, 'bar':bar, 'baz':baz})"]);
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &[
            "load('@ext_data//:data.bzl', ext_data='data')",
            "data=ext_data",
        ],
    );
    t.registry().add_module(
        &create_module_key("foo", "1.0"),
        &[
            "module(name='foo',version='1.0')",
            "bazel_dep(name='root',version='1.0')",
            "ext = use_extension('@root//:defs.bzl','ext')",
            "ext.bar(id = 3)",
            "ext.foo(id = 4)",
            "ext.baz(id = 1)",
        ],
    );
    t.invalidate_packages();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    result.throw_if_error();
    assert_eq!(result.global_str("data"), "[7, 2, 9, 42, -1, 5, 3, 4, 1]");
    // sorted order
}

// Java: ModuleExtensionResolutionTest.innate
#[test]
fn innate() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "bazel_dep(name='foo',version='1.0')",
            "data_repo = use_repo_rule('@foo//:repo.bzl', 'data_repo')",
            "data_repo(name='data1', data='get up at 6am.')",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &[
            "load('@data1//:data.bzl', self_data='data')",
            "load('@data1//:names.bzl', self_names='names')",
            "load('@foo//:data.bzl', foo_data='data')",
            "load('@foo//:names.bzl', foo_names='names')",
            "data=self_data+' '+foo_data",
            "names=self_names['name']+' '+foo_names['name']",
            "original_names=self_names['original_name']+' '+foo_names['original_name']",
        ],
    );
    t.registry().add_module(
        &create_module_key("foo", "1.0"),
        &[
            "module(name='foo',version='1.0')",
            "data_repo = use_repo_rule('//:repo.bzl', 'data_repo')",
            "data_repo(name='data2', data='go to bed at 11pm.')",
        ],
    );
    t.scratch.file(&module_path(&t, "foo+1.0/REPO.bazel"), &[]);
    t.scratch.file(&module_path(&t, "foo+1.0/BUILD"), &[]);
    t.scratch.file(
        &module_path(&t, "foo+1.0/data.bzl"),
        &[
            "load('@data2//:data.bzl',repo_data='data')",
            "data=repo_data",
        ],
    );
    t.scratch.file(
        &module_path(&t, "foo+1.0/names.bzl"),
        &[
            "load('@data2//:names.bzl',repo_names='names')",
            "names=repo_names",
        ],
    );
    t.scratch.file(
        &module_path(&t, "foo+1.0/repo.bzl"),
        &[
            "def _data_repo_impl(ctx):",
            "  ctx.file('BUILD.bazel')",
            "  ctx.file('data.bzl', 'data='+json.encode(ctx.attr.data))",
            "  ctx.file(",
            "    'names.bzl',",
            "    'names='+json.encode({",
            "      'name': ctx.name,",
            "      'original_name': ctx.original_name,",
            "    })",
            "  )",
            "data_repo = repository_rule(",
            "  implementation=_data_repo_impl, attrs={'data':attr.string()})",
        ],
    );
    t.invalidate_packages();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    result.throw_if_error();
    assert_eq!(
        result.global_str("data"),
        "get up at 6am. go to bed at 11pm."
    );
    assert_eq!(
        result.global_str("names"),
        "+data_repo+data1 foo++data_repo+data2"
    );
    assert_eq!(result.global_str("original_names"), "data1 data2");
}

// Java: ModuleExtensionResolutionTest.innate_repoRuleDependencies
#[test]
fn innate_repo_rule_dependencies() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "bazel_dep(name='foo',version='1.0')",
            "gen_data_repo = use_repo_rule('@foo//:repo.bzl', 'gen_data_repo')",
            "gen_data_repo(name='gen_data_repo')",
            "data_repo = use_repo_rule('@gen_data_repo//:repo.bzl', 'data_repo')",
            "data_repo(name='data', data='get up at 6am.')",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &["load('@data//:data.bzl', _data='data')", "data=_data"],
    );
    t.registry().add_module(
        &create_module_key("foo", "1.0"),
        &[
            "module(name='foo',version='1.0')",
            "bazel_dep(name='data_repo', version='1.0')",
        ],
    );
    t.scratch.file(&module_path(&t, "foo+1.0/REPO.bazel"), &[]);
    t.scratch.file(&module_path(&t, "foo+1.0/BUILD"), &[]);
    t.scratch.file(
        &module_path(&t, "foo+1.0/repo.bzl"),
        &[
            "def _gen_data_repo_impl(ctx):",
            "  ctx.file('BUILD.bazel')",
            "  ctx.file('repo.bzl', '''",
            "load('{data_repo_defs}', _data_repo='data_repo')",
            "data_repo=_data_repo",
            "'''.format(data_repo_defs = Label('@data_repo//:defs.bzl')))",
            "gen_data_repo = repository_rule(implementation=_gen_data_repo_impl)",
        ],
    );
    t.invalidate_packages();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    result.throw_if_error();
    assert_eq!(result.global_str("data"), "get up at 6am.");
}

// Java: ModuleExtensionResolutionTest.innate_noSuchRepoRule
#[test]
fn innate_no_such_repo_rule() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "data_repo = use_repo_rule('//:repo.bzl', 'data_repo')",
            "data_repo(name='data', data='get up at 6am.')",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &[
            "load('@data//:data.bzl', self_data='data')",
            "data=self_data",
        ],
    );
    t.scratch.file(
        "repo.bzl",
        &["# not a repo rule", "def data_repo(name):", "    pass"],
    );
    t.invalidate_packages();
    t.remove_fail_fast_handler();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    assert!(result.has_error());
    assert_contains(&result.error_message(), &t.ws("//:repo.bzl exports a value called data_repo of type function, yet a repository_rule is requested at /workspace/MODULE.bazel"));
}

// Java: ModuleExtensionResolutionTest.innate_noSuchValue
#[test]
fn innate_no_such_value() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "data_repo = use_repo_rule('//:repo.bzl', 'data_repo')",
            "data_repo(name='data', data='get up at 6am.')",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &[
            "load('@data//:data.bzl', self_data='data')",
            "data=self_data",
        ],
    );
    t.scratch.file("repo.bzl", &[""]);
    t.invalidate_packages();
    t.remove_fail_fast_handler();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    assert!(result.has_error());
    assert_contains(&result.error_message(), &t.ws("//:repo.bzl does not export a repository_rule called data_repo, yet its use is requested at /workspace/MODULE.bazel"));
}

// Java: ModuleExtensionResolutionTest.innate_noSuchValueIfPrivate
#[test]
fn innate_no_such_value_if_private() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "data_repo = use_repo_rule('//:repo.bzl', '_data_repo')",
            "data_repo(name='data', data='get up at 6am.')",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &[
            "load('@data//:data.bzl', self_data='data')",
            "data=self_data",
        ],
    );
    t.scratch.file(
        "repo.bzl",
        &["_data_repo = repository_rule(lambda _: None)"],
    );
    t.invalidate_packages();
    t.remove_fail_fast_handler();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    assert!(result.has_error());
    assert_contains(&result.error_message(), &t.ws("//:repo.bzl does not export a repository_rule called _data_repo, yet its use is requested at /workspace/MODULE.bazel"));
}

// Java: ModuleExtensionResolutionTest.innate_invalidAttributeValue
#[test]
fn innate_invalid_attribute_value() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            "bazel_dep(name='foo',version='1.0')",
            "data_repo = use_repo_rule('@foo//:repo.bzl', 'data_repo')",
            "data_repo(name='data', data=5)",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &[
            "load('@data//:data.bzl', self_data='data')",
            "load('@foo//:data.bzl', foo_data='data')",
            "data=self_data+' '+foo_data",
        ],
    );
    t.registry().add_module(
        &create_module_key("foo", "1.0"),
        &[
            "module(name='foo',version='1.0')",
            "data_repo = use_repo_rule('//:repo.bzl', 'data_repo')",
            "data_repo(name='data', data='go to bed at 11pm.')",
        ],
    );
    t.scratch.file(&module_path(&t, "foo+1.0/REPO.bazel"), &[]);
    t.scratch.file(&module_path(&t, "foo+1.0/BUILD"), &[]);
    t.scratch.file(
        &module_path(&t, "foo+1.0/data.bzl"),
        &[
            "load('@data//:data.bzl',repo_data='data')",
            "data=repo_data",
        ],
    );
    t.scratch.file(
        &module_path(&t, "foo+1.0/repo.bzl"),
        &[
            "def _data_repo_impl(ctx):",
            "  ctx.file('BUILD.bazel')",
            "  ctx.file('data.bzl', 'data='+json.encode(ctx.attr.data))",
            "data_repo = repository_rule(",
            "  implementation=_data_repo_impl, attrs={'data':attr.string()})",
        ],
    );
    t.invalidate_packages();
    t.remove_fail_fast_handler();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    assert!(result.has_error());
    t.assert_contains_event(&t.ws(r#"ERROR /workspace/MODULE.bazel:3:10: Traceback (most recent call last):
	File "/workspace/MODULE.bazel", line 3, column 10, in <toplevel>
		data_repo(name='data', data=5)
Error: in call to 'data_repo' repo rule with name 'data', expected value of type 'string' for attribute 'data', but got 5 (int)"#));
}

// Java: ModuleExtensionResolutionTest.extensionRepoMapping
#[test]
fn extension_repo_mapping() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[r#"bazel_dep(name = "data_repo", version = "1.0")
ext = use_extension("//:defs.bzl","ext")
use_repo(ext, real_foo = "foo", real_bar = "bar")
other_ext = use_extension("//:defs.bzl", "other_ext")
use_repo(other_ext, foo = "other_foo", bar = "other_bar")
"#],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &[r#"load("@real_foo//:list.bzl", _foo_list = "list")
load("@real_bar//:list.bzl", _bar_list = "list")
foo_list = _foo_list
bar_list = _bar_list
"#],
    );
    t.scratch.file(
        "defs.bzl",
        &[r#"load("@data_repo//:defs.bzl", "data_repo")
def _list_repo_impl(ctx):
  ctx.file("BUILD")
  labels = [str(Label(l)) for l in ctx.attr.labels]
  names = [str(Label(n)) for n in ctx.attr.names]
  ctx.file("list.bzl", "list = " + repr(labels + names))
list_repo = repository_rule(
  implementation = _list_repo_impl,
  attrs = {
    "names": attr.string_list(),
    "labels": attr.label_list(),
  },
)
def _ext_impl(ctx):
  labels = [
    "@foo//:target1",
    "@bar//:target2",
    Label("@foo//:target3"),
    Label("@bar//:target4"),
  ]
  list_repo(
    name = "foo",
    labels = labels,
    names = [
      "@foo",
      "@bar",
    ],
  )

  # Modify the list passed to "foo" to verify that it is not retained by
  # reference.
  labels[0] = "@foo//:target5"
  labels[1] = "@bar//:target6"
  labels[2] = Label("@foo//:target7")
  labels[3] = Label("@bar//:target8")
  list_repo(
    name = "bar",
    labels = labels,
    names = [
      "@foo",
      "@bar",
    ],
  )
ext = module_extension(implementation = _ext_impl)
def _other_ext_impl(ctx):
  data_repo(name="other_foo",data="other_foo_data")
  data_repo(name="other_bar",data="other_bar_data")
other_ext=module_extension(implementation=_other_ext_impl)
"#],
    );
    t.invalidate_packages();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    result.throw_if_error();
    assert_eq!(
        result.global_str_list("foo_list"),
        vec![
            "@@+ext+foo//:target1",
            "@@+ext+bar//:target2",
            "@@+other_ext+other_foo//:target3",
            "@@+other_ext+other_bar//:target4",
            "@@+other_ext+other_foo//:foo",
            "@@+other_ext+other_bar//:bar"
        ]
    );
    assert_eq!(
        result.global_str_list("bar_list"),
        vec![
            "@@+ext+foo//:target5",
            "@@+ext+bar//:target6",
            "@@+other_ext+other_foo//:target7",
            "@@+other_ext+other_bar//:target8",
            "@@+other_ext+other_foo//:foo",
            "@@+other_ext+other_bar//:bar"
        ]
    );
}

// Java: ModuleExtensionResolutionTest.overrideRepo_override
#[test]
fn override_repo_override() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[r#"bazel_dep(name = "data_repo", version = "1.0")
ext = use_extension("//:defs.bzl","ext")
use_repo(ext, "bar", module_foo = "foo")
data_repo = use_repo_rule("@data_repo//:defs.bzl", "data_repo")
data_repo(name = "override", data = "overridden_data")
override_repo(ext, foo = "override")
"#],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &[r#"load("@bar//:list.bzl", _bar_list = "list")
load("@override//:data.bzl", _override_data = "data")
load("@module_foo//:data.bzl", _foo_data = "data")
bar_list = _bar_list
foo_data = _foo_data
override_data = _override_data
"#],
    );
    t.scratch.file(
        "defs.bzl",
        &[r#"load("@data_repo//:defs.bzl", "data_repo")
def _list_repo_impl(ctx):
  ctx.file("BUILD")
  labels = [str(Label(l)) for l in ctx.attr.labels]
  labels += [str(Label("@module_foo//:target3"))]
  ctx.file("list.bzl", "list = " + repr(labels) + " + [str(Label('@foo//:target4'))]")
list_repo = repository_rule(
  implementation = _list_repo_impl,
  attrs = {
    "labels": attr.label_list(),
  },
)
def _fail_repo_impl(ctx):
  fail("This rule should not be evaluated")
fail_repo = repository_rule(implementation = _fail_repo_impl)
def _ext_impl(ctx):
  fail_repo(name = "foo")
  list_repo(
    name = "bar",
    labels = [
      # lazy extension implementation function repository mapping
      "@foo//:target1",
      # module repo repository mapping
      "@module_foo//:target2",
    ],
  )
ext = module_extension(implementation = _ext_impl)
"#],
    );
    t.invalidate_packages();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    result.throw_if_error();
    assert_eq!(
        result.global_str_list("bar_list"),
        vec![
            "@@+data_repo+override//:target1",
            "@@+data_repo+override//:target2",
            "@@+data_repo+override//:target3",
            "@@+data_repo+override//:target4"
        ]
    );
    let override_data = result.global_value("override_data");
    assert!(override_data.value().unpack_str().is_some());
    assert_eq!(override_data.value().unpack_str(), Some("overridden_data"));
    let foo_data = result.global_value("foo_data");
    assert!(foo_data.value().ptr_eq(override_data.value()));
}

// Java: ModuleExtensionResolutionTest.overrideRepo_override_onNonExistentRepoFails
#[test]
fn override_repo_override_on_non_existent_repo_fails() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[r#"bazel_dep(name = "data_repo", version = "1.0")
ext = use_extension("//:defs.bzl","ext")
use_repo(ext, "bar", module_foo = "foo")
data_repo = use_repo_rule("@data_repo//:defs.bzl", "data_repo")
data_repo(name = "foo", data = "overridden_data")
override_repo(ext, "foo")
"#],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &[r#"load("@bar//:list.bzl", _bar_list = "list")
load("@foo//:data.bzl", _foo_data = "data")
bar_list = _bar_list
foo_data = _foo_data
"#],
    );
    t.scratch.file(
        "defs.bzl",
        &[r#"load("@data_repo//:defs.bzl", "data_repo")
def _list_repo_impl(ctx):
  ctx.file("BUILD")
  labels = [str(Label(l)) for l in ctx.attr.labels]
  labels += [str(Label("@foo//:target3"))]
  ctx.file("list.bzl", "list = " + repr(labels) + " + [str(Label('@foo//:target4'))]")
list_repo = repository_rule(
  implementation = _list_repo_impl,
  attrs = {
    "labels": attr.label_list(),
  },
)
def _ext_impl(ctx):
  list_repo(
    name = "bar",
    labels = [
      # lazy extension implementation function repository mapping
      "@foo//:target1",
      # module repo repository mapping
      Label("@foo//:target2"),
    ],
  )
ext = module_extension(implementation = _ext_impl)
"#],
    );
    t.invalidate_packages();
    t.remove_fail_fast_handler();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    assert!(result.has_error());
    assert_eq!(result.error_message(), t.ws(r#"module extension @@//:defs.bzl%ext does not generate repository "foo", yet it is overridden via override_repo() at /workspace/MODULE.bazel:6:14. Use inject_repo() instead to inject a new repository."#));
}

// Java: ModuleExtensionResolutionTest.overrideRepo_inject
#[test]
fn override_repo_inject() {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[r#"bazel_dep(name = "data_repo", version = "1.0")
ext = use_extension("//:defs.bzl","ext")
use_repo(ext, "bar")
data_repo = use_repo_rule("@data_repo//:defs.bzl", "data_repo")
data_repo(name = "foo", data = "overridden_data")
inject_repo(ext, "foo")
"#],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &[r#"load("@bar//:list.bzl", _bar_list = "list")
load("@foo//:data.bzl", _foo_data = "data")
bar_list = _bar_list
foo_data = _foo_data
"#],
    );
    t.scratch.file(
        "defs.bzl",
        &[r#"load("@data_repo//:defs.bzl", "data_repo")
def _list_repo_impl(ctx):
  ctx.file("BUILD")
  labels = [str(Label(l)) for l in ctx.attr.labels]
  labels += [str(Label("@foo//:target3"))]
  ctx.file("list.bzl", "list = " + repr(labels) + " + [str(Label('@foo//:target4'))]")
list_repo = repository_rule(
  implementation = _list_repo_impl,
  attrs = {
    "labels": attr.label_list(),
  },
)
def _ext_impl(ctx):
  list_repo(
    name = "bar",
    labels = [
      # lazy extension implementation function repository mapping
      "@foo//:target1",
      # module repo repository mapping
      Label("@foo//:target2"),
    ],
  )
ext = module_extension(implementation = _ext_impl)
"#],
    );
    t.invalidate_packages();
    let result = evaluate_bzl(&mut t, "//:data.bzl");
    result.throw_if_error();
    assert_eq!(
        result.global_str_list("bar_list"),
        vec![
            "@@+data_repo+foo//:target1",
            "@@+data_repo+foo//:target2",
            "@@+data_repo+foo//:target3",
            "@@+data_repo+foo//:target4"
        ]
    );
    let foo_data = result.global_value("foo_data");
    assert!(foo_data.value().unpack_str().is_some());
    assert_eq!(foo_data.value().unpack_str(), Some("overridden_data"));
}

// Java: ModuleExtensionResolutionTest.useRepo_placeholders
#[test]
fn use_repo_placeholders() {
    // Java: @TestParameter({"1.2.3-beta.4.5.6+build.7", "1.2.3-beta.4.5.6", ""}) rootVersion
    for root_version in ["1.2.3-beta.4.5.6+build.7", "1.2.3-beta.4.5.6", ""] {
        use_repo_placeholders_with(root_version);
    }
}

fn use_repo_placeholders_with(root_version: &str) {
    let mut t = new_test();
    t.scratch.overwrite_file(
        "MODULE.bazel",
        &[
            &format!("module(name='root',version='{root_version}')"),
            "bazel_dep(name='ext',version='2.0')",
            "bazel_dep(name='foo',version='3.0')",
            "ext = use_extension('@ext//:defs.bzl','ext')",
            "use_repo(ext,ext_repo='{name}_{version}_repo')",
        ],
    );
    t.scratch.overwrite_file("BUILD", &[]);
    t.scratch.file(
        "data.bzl",
        &[
            "load('@ext_repo//:data.bzl', ext_data='data')",
            "data=ext_data",
        ],
    );

    t.registry().add_module(
        &create_module_key("foo", "3.0"),
        &[
            "module(name='foo',version='3.0')",
            "bazel_dep(name='ext',version='2.0')",
            "ext = use_extension('@ext//:defs.bzl','ext')",
            "use_repo(ext,ext_repo='{name}_{version}_repo')",
        ],
    );
    t.scratch.file(&module_path(&t, "foo+3.0/REPO.bazel"), &[]);
    t.scratch.file(&module_path(&t, "foo+3.0/BUILD"), &[]);
    t.scratch.file(
        &module_path(&t, "foo+3.0/data.bzl"),
        &[
            "load('@ext_repo//:data.bzl', ext_data='data')",
            "data=ext_data",
        ],
    );

    t.registry().add_module(
        &create_module_key("ext", "2.0"),
        &[
            "module(name='ext',version='2.0')",
            "bazel_dep(name='data_repo',version='1.0')",
        ],
    );
    t.scratch.file(&module_path(&t, "ext+2.0/REPO.bazel"), &[]);
    t.scratch.file(&module_path(&t, "ext+2.0/BUILD"), &[]);
    t.scratch.file(
        &module_path(&t, "ext+2.0/defs.bzl"),
        &[
            "load('@data_repo//:defs.bzl','data_repo')",
            "def _ext_impl(ctx):",
            "  for mod in ctx.modules:",
            "    data_repo(",
            "      name = '{}_{}_repo'.format(mod.name, mod.version),",
            "      data = '{}@{}'.format(mod.name, mod.version),",
            "    )",
            "ext=module_extension(implementation=_ext_impl)",
        ],
    );
    t.invalidate_packages();

    let result = evaluate_bzl(&mut t, "@foo+//:data.bzl");
    result.throw_if_error();
    assert_eq!(result.global_str("data"), "foo@3.0");

    let result = evaluate_bzl(&mut t, "//:data.bzl");
    result.throw_if_error();
    let normalized = blaze_bzlmod_model::Version::parse(root_version)
        .unwrap_or_else(|e| panic!("{e:?}"))
        .get_normalized()
        .to_string();
    assert_eq!(result.global_str("data"), format!("root@{normalized}"));
}

/// The fixup part of `extensionMetadata_all`: the warning and the buildozer commands generated
/// from the extension's metadata (`root_module_direct_deps='all', root_module_direct_dev_deps=[]`)
/// and the root module's usage. PORT: the evaluation part (the INVALID_EXTENSION_IMPORT error and
/// the event count) needs the BuildViewTestCase Skyframe setup.
#[test]
fn extension_metadata_all_generate_fixup() {
    let proxy = |name: &str, dev: bool, line: i32, imports: &[&str]| ModuleExtensionUsageProxy {
        location: Location::new("/workspace/MODULE.bazel", line, 20),
        proxy_name: name.to_owned(),
        containing_module_file_path: PathFragment::create("MODULE.bazel"),
        dev_dependency: dev,
        imports: imports
            .iter()
            .map(|i| (i.to_string(), i.to_string()))
            .collect(),
    };
    let root_usage = ModuleExtensionUsage {
        extension_bzl_file: "@ext//:defs.bzl".to_owned(),
        extension_name: "ext".to_owned(),
        isolation_key: None,
        proxies: vec![
            proxy(
                "ext",
                false,
                3,
                &["direct_dep", "indirect_dep", "invalid_dep"],
            ),
            proxy(
                "ext_dev",
                true,
                5,
                &["direct_dev_dep", "indirect_dev_dep", "invalid_dev_dep"],
            ),
        ],
        tags: Vec::new(),
        repo_overrides: IndexMap::new(),
    };
    let metadata = LockfileModuleExtensionMetadata {
        explicit_root_module_direct_deps: None,
        explicit_root_module_direct_dev_deps: None,
        use_all_repos: UseAllRepos::Regular,
        reproducible: false,
    };
    let all_repos: IndexSet<String> = [
        "direct_dep",
        "direct_dev_dep",
        "missing_direct_dep",
        "missing_direct_dev_dep",
        "indirect_dep",
        "indirect_dev_dep",
    ]
    .iter()
    .map(|s| s.to_string())
    .collect();

    let fixup = generate_fixup(&metadata, &root_usage, &all_repos)
        .unwrap()
        .expect("a fixup");

    assert_eq!(
        fixup.warning.get_message(),
        "The module extension ext defined in @ext//:defs.bzl reported incorrect imports of \
         repositories via use_repo():\n\
         \n\
         Imported, but not created by the extension (will cause the build to fail):\n    \
         invalid_dep, invalid_dev_dep\n\
         \n\
         Not imported, but reported as direct dependencies by the extension (may cause the build \
         to fail):\n    \
         missing_direct_dep, missing_direct_dev_dep\n\
         \n\
         Imported as a dev dependency, but reported as a regular dependency by the extension \
         (may cause the build to fail when used by other modules):\n    \
         direct_dev_dep, indirect_dev_dep\n\
         \n\
         Fix the use_repo calls by running 'bazel mod tidy'."
    );
    assert_eq!(
        fixup.warning.get_location().map(|l| l.to_string()),
        Some("/workspace/MODULE.bazel:3:20".to_owned())
    );
    let commands: Vec<(String, String)> = fixup
        .module_file_path_to_buildozer_commands
        .iter()
        .flat_map(|(path, commands)| {
            commands
                .iter()
                .map(move |c| (path.get_path_string().to_owned(), c.clone()))
        })
        .collect();
    assert_eq!(
        commands,
        vec![
            (
                "MODULE.bazel".to_owned(),
                "use_repo_add ext direct_dev_dep indirect_dev_dep missing_direct_dep \
                 missing_direct_dev_dep"
                    .to_owned()
            ),
            (
                "MODULE.bazel".to_owned(),
                "use_repo_remove ext invalid_dep".to_owned()
            ),
            (
                "MODULE.bazel".to_owned(),
                "use_repo_remove ext_dev direct_dev_dep indirect_dev_dep invalid_dev_dep"
                    .to_owned()
            ),
        ]
    );
}
