//! Port of `src/test/java/com/google/devtools/build/lib/bazel/bzlmod/modcommand/ModuleArgTest.java`.

mod modcommand_common;

use blaze_bzlmod::inspector::{AugmentedModule, ResolutionReason};
use blaze_bzlmod::modcommand::ModuleArg;
use blaze_bzlmod_model::ModuleKey;
use blaze_cmdline::{RepositoryMapping, RepositoryName};
use indexmap::{IndexMap, IndexSet};
use modcommand_common::*;

#[test]
fn converter() {
    assert_eq!(
        ModuleArg::convert("<root>").unwrap(),
        ModuleArg::SpecificVersionOfModule(ModuleKey::root())
    );
    assert_eq!(
        ModuleArg::convert("@@abc").unwrap(),
        ModuleArg::CanonicalRepoName(RepositoryName::create_unvalidated("abc"))
    );
    assert_eq!(
        ModuleArg::convert("@abc").unwrap(),
        ModuleArg::ApparentRepoName("abc".to_string())
    );
    assert_eq!(
        ModuleArg::convert("abc").unwrap(),
        ModuleArg::AllVersionsOfModule("abc".to_string())
    );
    assert_eq!(
        ModuleArg::convert("a@b").unwrap(),
        ModuleArg::SpecificVersionOfModule(create_module_key("a", "b"))
    );
    assert_eq!(
        ModuleArg::convert("a@3.1.0-pre").unwrap(),
        ModuleArg::SpecificVersionOfModule(create_module_key("a", "3.1.0-pre"))
    );

    assert!(ModuleArg::convert("abc@").is_err());
    assert!(ModuleArg::convert("@_abc").is_err());
}

// For the resolveToX test cases, we build a very, very simple dep graph, where root originally
// depends on foo@1.0, but it's magically upgraded to foo@2.0. The dependency has an
// affectionate repo name of "fred".
struct Fixture {
    foo1: ModuleKey,
    foo2: ModuleKey,
    modules_index: IndexMap<String, IndexSet<ModuleKey>>,
    dep_graph: IndexMap<ModuleKey, AugmentedModule>,
    module_key_to_canonical_names: IndexMap<ModuleKey, RepositoryName>,
    base_module_deps: IndexMap<String, ModuleKey>,
    base_module_unused_deps: IndexMap<String, ModuleKey>,
    root_mapping: RepositoryMapping,
}

impl Fixture {
    fn new() -> Fixture {
        let foo1 = create_module_key("foo", "1.0");
        let foo2 = create_module_key("foo", "2.0");
        let modules_index = IndexMap::from([
            ("".to_string(), IndexSet::from([ModuleKey::root()])),
            (
                "foo".to_string(),
                IndexSet::from([foo1.clone(), foo2.clone()]),
            ),
        ]);
        let dep_graph = map_of(vec![
            build_augmented_module("", "")
                .add_changed_dep_repo(
                    "fred",
                    "foo",
                    "2.0",
                    "1.0",
                    ResolutionReason::SINGLE_VERSION_OVERRIDE,
                )
                .build_entry(),
            build_augmented_module("foo", "1.0")
                .add_original_dependant_key(ModuleKey::root())
                .build_entry(),
            build_augmented_module("foo", "2.0")
                .add_still_dependant_key(ModuleKey::root())
                .build_entry(),
        ]);
        let module_key_to_canonical_names = dep_graph
            .keys()
            .map(|k| (k.clone(), k.get_canonical_repo_name_with_version()))
            .collect();
        let base_module_deps = IndexMap::from([("fred".to_string(), foo2.clone())]);
        let base_module_unused_deps = IndexMap::from([("fred".to_string(), foo1.clone())]);
        let root_mapping = create_repository_mapping(&ModuleKey::root(), &["fred", "foo+2.0"]);
        Fixture {
            foo1,
            foo2,
            modules_index,
            dep_graph,
            module_key_to_canonical_names,
            base_module_deps,
            base_module_unused_deps,
            root_mapping,
        }
    }

    fn resolve_to_module_keys(
        &self,
        arg: &ModuleArg,
        include_unused: bool,
        warn_unused: bool,
    ) -> Result<IndexSet<ModuleKey>, blaze_bzlmod::modcommand::InvalidArgumentException> {
        arg.resolve_to_module_keys(
            &self.modules_index,
            &self.dep_graph,
            &self.module_key_to_canonical_names,
            &self.base_module_deps,
            &self.base_module_unused_deps,
            include_unused,
            warn_unused,
        )
    }

    fn resolve_to_repo_names(
        &self,
        arg: &ModuleArg,
    ) -> Result<IndexMap<String, RepositoryName>, blaze_bzlmod::modcommand::InvalidArgumentException>
    {
        arg.resolve_to_repo_names(
            &self.modules_index,
            &self.dep_graph,
            &self.module_key_to_canonical_names,
            &self.root_mapping,
        )
    }
}

fn set(keys: &[&ModuleKey]) -> IndexSet<ModuleKey> {
    keys.iter().map(|k| (*k).clone()).collect()
}

fn repos(name: &str, repo: &str) -> IndexMap<String, RepositoryName> {
    IndexMap::from([(name.to_string(), RepositoryName::create(repo).unwrap())])
}

#[test]
fn resolve_specific_version_good() {
    let f = Fixture::new();
    let arg = ModuleArg::SpecificVersionOfModule(f.foo2.clone());
    assert_eq!(
        f.resolve_to_module_keys(
            &arg, /* include_unused= */ false, /* warn_unused= */ false
        )
        .unwrap(),
        set(&[&f.foo2])
    );

    assert_eq!(
        f.resolve_to_repo_names(&arg).unwrap(),
        repos("foo@2.0", "foo+2.0")
    );
}

#[test]
fn resolve_specific_version_not_found() {
    let f = Fixture::new();
    let arg = ModuleArg::SpecificVersionOfModule(create_module_key("foo", "3.0"));
    assert!(
        f.resolve_to_module_keys(
            &arg, /* include_unused= */ true, /* warn_unused= */ true
        )
        .is_err()
    );
    assert!(f.resolve_to_repo_names(&arg).is_err());
}

#[test]
fn resolve_specific_version_unused() {
    let f = Fixture::new();
    let arg = ModuleArg::SpecificVersionOfModule(f.foo1.clone());
    // Without --include_unused, this doesn't resolve, as foo@1.0 has been replaced by foo@2.0.
    let e = f
        .resolve_to_module_keys(
            &arg, /* include_unused= */ false, /* warn_unused= */ true,
        )
        .unwrap_err();
    assert!(e.get_message().contains("--include_unused"));
    // With --include_unused, this resolves to foo@1.0.
    assert_eq!(
        f.resolve_to_module_keys(
            &arg, /* include_unused= */ true, /* warn_unused= */ true
        )
        .unwrap(),
        set(&[&f.foo1])
    );

    // resolving to repo names doesn't care about unused deps.
    assert!(f.resolve_to_repo_names(&arg).is_err());
}

#[test]
fn resolve_all_versions_good() {
    let f = Fixture::new();
    let arg = ModuleArg::AllVersionsOfModule("foo".to_string());

    assert_eq!(
        f.resolve_to_module_keys(
            &arg, /* include_unused= */ false, /* warn_unused= */ false
        )
        .unwrap(),
        set(&[&f.foo2])
    );
    // foo1 is unused, so --include_unused would return that too
    assert_eq!(
        f.resolve_to_module_keys(
            &arg, /* include_unused= */ true, /* warn_unused= */ false
        )
        .unwrap(),
        set(&[&f.foo2, &f.foo1])
    );

    // resolving to repo names doesn't care about unused deps.
    assert_eq!(
        f.resolve_to_repo_names(&arg).unwrap(),
        repos("foo@2.0", "foo+2.0")
    );
}

#[test]
fn resolve_all_versions_not_found() {
    let f = Fixture::new();
    let arg = ModuleArg::AllVersionsOfModule("bar".to_string());

    assert!(
        f.resolve_to_module_keys(
            &arg, /* include_unused= */ true, /* warn_unused= */ true
        )
        .is_err()
    );
    assert!(f.resolve_to_repo_names(&arg).is_err());
}

#[test]
fn resolve_apparent_repo_name_good() {
    let f = Fixture::new();
    let arg = ModuleArg::ApparentRepoName("fred".to_string());

    assert_eq!(
        f.resolve_to_module_keys(
            &arg, /* include_unused= */ false, /* warn_unused= */ false
        )
        .unwrap(),
        set(&[&f.foo2])
    );
    // foo1 is unused, so --include_unused would return that too
    assert_eq!(
        f.resolve_to_module_keys(
            &arg, /* include_unused= */ true, /* warn_unused= */ false
        )
        .unwrap(),
        set(&[&f.foo2, &f.foo1])
    );

    assert_eq!(
        f.resolve_to_repo_names(&arg).unwrap(),
        repos("@fred", "foo+2.0")
    );
}

#[test]
fn resolve_apparent_repo_name_not_found() {
    let f = Fixture::new();
    let arg = ModuleArg::ApparentRepoName("brad".to_string());

    assert!(
        f.resolve_to_module_keys(
            &arg, /* include_unused= */ true, /* warn_unused= */ true
        )
        .is_err()
    );
    assert!(f.resolve_to_repo_names(&arg).is_err());
}

#[test]
fn resolve_canonical_repo_name_good() {
    let f = Fixture::new();
    let arg = ModuleArg::CanonicalRepoName(f.foo2.get_canonical_repo_name_with_version());

    assert_eq!(
        f.resolve_to_module_keys(
            &arg, /* include_unused= */ false, /* warn_unused= */ false
        )
        .unwrap(),
        set(&[&f.foo2])
    );

    assert_eq!(
        f.resolve_to_repo_names(&arg).unwrap(),
        repos("@@foo+2.0", "foo+2.0")
    );
}

#[test]
fn resolve_canonical_repo_name_not_found() {
    let f = Fixture::new();
    let arg = ModuleArg::CanonicalRepoName(RepositoryName::create("bar+1.0").unwrap());

    assert!(
        f.resolve_to_module_keys(
            &arg, /* include_unused= */ true, /* warn_unused= */ true
        )
        .is_err()
    );
    // The repo need not exist in the "repo -> repo" case.
    assert_eq!(
        f.resolve_to_repo_names(&arg).unwrap(),
        repos("@@bar+1.0", "bar+1.0")
    );
}

#[test]
fn resolve_canonical_repo_name_unused() {
    let f = Fixture::new();
    let arg = ModuleArg::CanonicalRepoName(f.foo1.get_canonical_repo_name_with_version());

    // Without --include_unused, this doesn't resolve, as foo@1.0 has been replaced by foo@2.0.
    let e = f
        .resolve_to_module_keys(
            &arg, /* include_unused= */ false, /* warn_unused= */ true,
        )
        .unwrap_err();
    assert!(e.get_message().contains("--include_unused"));
    // With --include_unused, this resolves to foo@1.0.
    assert_eq!(
        f.resolve_to_module_keys(
            &arg, /* include_unused= */ true, /* warn_unused= */ true
        )
        .unwrap(),
        set(&[&f.foo1])
    );

    // resolving to repo names doesn't care about unused deps.
    assert_eq!(
        f.resolve_to_repo_names(&arg).unwrap(),
        repos("@@foo+1.0", "foo+1.0")
    );
}
