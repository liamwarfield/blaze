//! Port of `analysis/AspectCollection.java`.

use std::collections::HashMap;
use std::fmt;
use std::sync::{Arc, LazyLock};

use blaze_packages::aspect::{Aspect, AspectDescriptor};
use indexmap::{IndexMap, IndexSet};

use crate::aspect_key_creator::{AspectKey, AspectKeyCreator};
use crate::configured_target_key::ConfiguredTargetKey;

/// The name of the native aspect that collects validation outputs.
// Java: com/google/devtools/build/lib/analysis/AspectCollection.java:153 AspectCollection.VALIDATION_ASPECT_NAME
pub const VALIDATION_ASPECT_NAME: &str = "ValidateTarget";

/// Represents aspects that should be applied to a configured target as part of `Dependency`.
///
/// One can consider the configured target graph as being a DAG in two dimensions: one is the DAG
/// analogous to the target graph and the other is a DAG between aspects applied to the same
/// configured target. This class represents the latter. The full "aspect dependency graph" is
/// computed when traversing the configured target graph. The analysis of the aspects attached to
/// the same configured target is done by simply unwrapping the graph of [`AspectDeps`]
/// instances.
///
/// Analysis arrives to a particular node in target graph with an ordered list of aspects that
/// need to be applied. Some of those aspects should visible to the node in question; some of them
/// are not directly visible, but are visible to other aspects, as specified by
/// `AspectDefinition#getRequiredProvidersForAspects()`.
///
/// [`AspectCollection::get_used_aspects`] returns all aspects that should be applied to the
/// target, in topological order. See the Java class comment for worked examples of aspect
/// visibility.
// Java: com/google/devtools/build/lib/analysis/AspectCollection.java:151 AspectCollection
#[derive(Clone, PartialEq, Eq)]
pub struct AspectCollection {
    /// aspects that should be visible to a dependency
    used_aspects: IndexSet<AspectDeps>,
}

/// Java `AspectCollection.EMPTY`.
// Java: com/google/devtools/build/lib/analysis/AspectCollection.java:158 AspectCollection.EMPTY
pub static EMPTY: LazyLock<AspectCollection> = LazyLock::new(|| AspectCollection {
    used_aspects: IndexSet::new(),
});

impl AspectCollection {
    /// Java `AspectCollection.EMPTY`.
    pub fn empty() -> AspectCollection {
        EMPTY.clone()
    }

    // Java: com/google/devtools/build/lib/analysis/AspectCollection.java:164 AspectCollection.getUsedAspects
    pub fn get_used_aspects(&self) -> &IndexSet<AspectDeps> {
        &self.used_aspects
    }

    // Java: com/google/devtools/build/lib/analysis/AspectCollection.java:168 AspectCollection.isEmpty
    pub fn is_empty(&self) -> bool {
        self.used_aspects.is_empty()
    }

    // Java: com/google/devtools/build/lib/analysis/AspectCollection.java:190 AspectCollection.createAspectKeys
    pub fn create_aspect_keys(&self, base_key: &ConfiguredTargetKey) -> Vec<AspectKey> {
        // PORT: java-hash-order, not replicable. Java collects the keys into a
        // `HashMap<AspectDescriptor, AspectKey>` and returns `values()` in HashMap iteration
        // order. `AspectDescriptor.hashCode` hashes the aspect class: a native class hashes by
        // identity, and `StarlarkAspectClass.hashCode` hashes its `BzlLoadValue.Key`, whose
        // `hashCode` mixes in `getClass().hashCode()` (BzlLoadValue.java:190), an identity
        // hash too. So Bazel's own order of several top-level aspect keys changes from one
        // server process to the next; this port returns them in insertion order (the order
        // with a single aspect).
        let mut descriptor_to_aspect_key: IndexMap<AspectDescriptor, AspectKey> = IndexMap::new();
        for aspect_deps in self.get_used_aspects() {
            build_aspect_key(aspect_deps, &mut descriptor_to_aspect_key, base_key);
        }
        descriptor_to_aspect_key.into_values().collect()
    }

    // Java: com/google/devtools/build/lib/analysis/AspectCollection.java:263 AspectCollection.createForTests
    pub fn create_for_tests(descriptors: &[AspectDescriptor]) -> AspectCollection {
        let mut deps_builder: IndexSet<AspectDeps> = IndexSet::new();
        for descriptor in descriptors {
            deps_builder.insert(AspectDeps::create(descriptor.clone(), Vec::new()));
        }
        AspectCollection {
            used_aspects: deps_builder,
        }
    }

    /// Creates an [`AspectCollection`] from an ordered list of aspects and a set of visible
    /// aspects.
    ///
    /// The order of aspects is reverse to the order in which they originated, with the earliest
    /// originating occurring last in the list.
    // Java: com/google/devtools/build/lib/analysis/AspectCollection.java:278 AspectCollection.create
    pub fn create(aspect_path: &[Aspect]) -> Result<AspectCollection, AspectCycleOnPathException> {
        let aspect_map: IndexMap<AspectDescriptor, Aspect> = deduplicate_aspects(aspect_path)?;
        let mut deps: IndexMap<AspectDescriptor, Vec<AspectDescriptor>> = IndexMap::new();

        // Calculate all needed aspects. Already discovered aspects are in key set of deps.
        // 1) Start from the end of the path. The aspect only sees other aspects that are
        //    before it
        // 2) Otherwise, check whether 'aspect' is visible to or required by any already seen
        //    aspects. If it is visible to 'depAspect' or explicitly required by it, add the
        //    'aspect' to a list of aspects visible to 'depAspect'.
        // At the end of this algorithm, key set of 'deps' contains the original aspect list in
        // reverse (since we iterate the original list in reverse).
        //
        // deps[aspect] contains all aspects that 'aspect' needs, in reverse order.
        let entries: Vec<(&AspectDescriptor, &Aspect)> = aspect_map.iter().collect();
        for (aspect_key, aspect_value) in entries.into_iter().rev() {
            for (dep_aspect_descriptor, dep_list) in deps.iter_mut() {
                let dep_aspect = &aspect_map[dep_aspect_descriptor];
                // As any aspect can add validation outputs, the special validation aspect that
                // collects their outputs has to depend on all aspects.
                if dep_aspect
                    .get_definition()
                    .get_required_providers_for_aspects()
                    .is_satisfied_by(aspect_value.get_definition().get_advertised_providers())
                    || dep_aspect.get_definition().requires(aspect_value)
                    || dep_aspect.get_aspect_class().get_name() == VALIDATION_ASPECT_NAME
                {
                    dep_list.push(aspect_key.clone());
                }
            }

            deps.insert(aspect_key.clone(), Vec::new());
        }

        // Calculate the path for every directly required aspect
        let mut aspect_paths: HashMap<AspectDescriptor, AspectDeps> = HashMap::new();
        let mut result: IndexSet<AspectDeps> = IndexSet::new();
        for aspect in aspect_map.keys() {
            result.insert(build_aspect_deps(aspect, &mut aspect_paths, &deps));
        }
        Ok(AspectCollection {
            used_aspects: result,
        })
    }
}

impl fmt::Debug for AspectCollection {
    // Java: com/google/devtools/build/lib/analysis/AspectCollection.java:173 AspectCollection.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts: Vec<String> = self.used_aspects.iter().map(|d| format!("{d:?}")).collect();
        write!(f, "AspectCollection{{[{}]}}", parts.join(", "))
    }
}

impl fmt::Display for AspectCollection {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Debug::fmt(self, f)
    }
}

impl std::hash::Hash for AspectCollection {
    // Java: com/google/devtools/build/lib/analysis/AspectCollection.java:178 AspectCollection.hashCode
    // PORT: ImmutableSet.hashCode is order-insensitive; so is this (sum of element hashes).
    fn hash<H: std::hash::Hasher>(&self, state: &mut H) {
        let mut sum: u64 = 0;
        for d in &self.used_aspects {
            let mut h = std::collections::hash_map::DefaultHasher::new();
            d.hash(&mut h);
            sum = sum.wrapping_add(std::hash::Hasher::finish(&h));
        }
        sum.hash(state);
    }
}

/// Represents an aspect with all the aspects it depends on (within an [`AspectCollection`]).
///
/// We preserve the order of aspects to correspond to the order originally specified in the call
/// to [`AspectCollection::create`], although that is not strictly needed semantically.
///
/// This data structure cannot be a simple list. Consider the case when four aspects [a1, a2, a3,
/// a4] are attached and a4 is interested in a3, a3 in a2 and a2 in a1.
///
/// In this case, when analyzing a3, only a2 will be in its direct dependencies (since we don't
/// want to merge in the dependencies of a1), but then a2 would have no way of knowing that a1 was
/// also propagated.
///
/// PORT: a Java record (structural equality); shared through an `Arc` so the instance built once
/// per descriptor is reused (Java shares the same object).
// Java: com/google/devtools/build/lib/analysis/AspectCollection.java:214 AspectCollection.AspectDeps
#[derive(Clone, PartialEq, Eq, Hash)]
pub struct AspectDeps(Arc<AspectDepsInner>);

#[derive(PartialEq, Eq, Hash)]
struct AspectDepsInner {
    aspect: AspectDescriptor,
    used_aspects: Vec<AspectDeps>,
}

impl AspectDeps {
    // Java: com/google/devtools/build/lib/analysis/AspectCollection.java:215 AspectCollection.AspectDeps.<init>
    pub fn new(aspect: AspectDescriptor, used_aspects: Vec<AspectDeps>) -> AspectDeps {
        AspectDeps(Arc::new(AspectDepsInner {
            aspect,
            used_aspects,
        }))
    }

    // Java: com/google/devtools/build/lib/analysis/AspectCollection.java:220 AspectCollection.AspectDeps.create
    fn create(aspect: AspectDescriptor, used_aspects: Vec<AspectDeps>) -> AspectDeps {
        AspectDeps::new(aspect, used_aspects)
    }

    /// Java record accessor `aspect()`.
    pub fn aspect(&self) -> &AspectDescriptor {
        &self.0.aspect
    }

    /// Java record accessor `usedAspects()`.
    pub fn used_aspects(&self) -> &[AspectDeps] {
        &self.0.used_aspects
    }

    /// Java `==` on two `AspectDeps` (the same instance).
    pub fn is_same_instance(&self, other: &AspectDeps) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl fmt::Debug for AspectDeps {
    // Java: record toString `AspectDeps[aspect=..., usedAspects=[...]]`.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let parts: Vec<String> = self
            .0
            .used_aspects
            .iter()
            .map(|d| format!("{d:?}"))
            .collect();
        write!(
            f,
            "AspectDeps[aspect={}, usedAspects=[{}]]",
            self.0.aspect,
            parts.join(", ")
        )
    }
}

/// Creates an [`AspectKey`] for the given root aspect, `aspect_deps`.
///
/// Converts the DAG of `AspectDescriptor`s rooted at `aspect_deps` into an isomorphic DAG of
/// [`AspectKey`] with corresponding `AspectKey::get_aspect_descriptor` values. All resulting
/// [`AspectKey`]s have `AspectKey::get_base_configured_target_key` equal to `base_key`.
///
/// As a side effect, `visited` is populated with all the DAG nodes with each map entry value's
/// descriptor matching the map entry key.
// Java: com/google/devtools/build/lib/analysis/AspectCollection.java:238 AspectCollection.buildAspectKey
pub fn build_aspect_key(
    aspect_deps: &AspectDeps,
    visited: &mut IndexMap<AspectDescriptor, AspectKey>,
    base_key: &ConfiguredTargetKey,
) -> AspectKey {
    let aspect = aspect_deps.aspect();
    if let Some(aspect_key) = visited.get(aspect) {
        return aspect_key.clone();
    }

    let used_aspects = aspect_deps.used_aspects();
    let mut used_aspect_keys: Vec<AspectKey> = Vec::with_capacity(used_aspects.len());
    for used_aspect in used_aspects {
        used_aspect_keys.push(build_aspect_key(used_aspect, visited, base_key));
    }

    let aspect_key =
        AspectKeyCreator::create_aspect_key(aspect.clone(), used_aspect_keys, base_key.clone());
    visited.insert(aspect.clone(), aspect_key.clone());
    aspect_key
}

/// Deduplicate aspects in path.
///
/// Fails with [`AspectCycleOnPathException`] if an aspect occurs twice on the path and the
/// second occurrence sees a different set of aspects.
// Java: com/google/devtools/build/lib/analysis/AspectCollection.java:328 AspectCollection.deduplicateAspects
fn deduplicate_aspects(
    aspect_path: &[Aspect],
) -> Result<IndexMap<AspectDescriptor, Aspect>, AspectCycleOnPathException> {
    let mut aspect_map: IndexMap<AspectDescriptor, Aspect> = IndexMap::new();
    let mut seen_aspects: Vec<Aspect> = Vec::new();
    for aspect in aspect_path {
        if !aspect_map.contains_key(aspect.get_descriptor()) {
            aspect_map.insert(aspect.get_descriptor().clone(), aspect.clone());
            seen_aspects.push(aspect.clone());
        } else {
            validate_duplicate_aspect(aspect, &seen_aspects)?;
        }
    }
    Ok(aspect_map)
}

/// Detect inconsistent duplicate occurrence of an aspect on the path. There is a previous
/// occurrence of `aspect` in `seen_aspects`.
///
/// If in between that previous occurrence and the newly discovered occurrence there is an aspect
/// that is visible to or required by `aspect`, then the second occurrence is inconsistent - the
/// set of aspects it sees is different from the first one.
// Java: com/google/devtools/build/lib/analysis/AspectCollection.java:352 AspectCollection.validateDuplicateAspect
fn validate_duplicate_aspect(
    aspect: &Aspect,
    seen_aspects: &[Aspect],
) -> Result<(), AspectCycleOnPathException> {
    for seen_aspect in seen_aspects.iter().rev() {
        if aspect.get_descriptor() == seen_aspect.get_descriptor() {
            // This is a previous occurrence of the same aspect.
            return Ok(());
        }

        if aspect
            .get_definition()
            .get_required_providers_for_aspects()
            .is_satisfied_by(seen_aspect.get_definition().get_advertised_providers())
            || aspect.get_definition().requires(seen_aspect)
        {
            return Err(AspectCycleOnPathException::new(
                aspect.get_descriptor().clone(),
                seen_aspect.get_descriptor().clone(),
            ));
        }
    }
    Ok(())
}

// Java: com/google/devtools/build/lib/analysis/AspectCollection.java:371 AspectCollection.buildAspectDeps
fn build_aspect_deps(
    descriptor: &AspectDescriptor,
    aspect_paths: &mut HashMap<AspectDescriptor, AspectDeps>,
    deps: &IndexMap<AspectDescriptor, Vec<AspectDescriptor>>,
) -> AspectDeps {
    if let Some(p) = aspect_paths.get(descriptor) {
        return p.clone();
    }

    let mut aspect_path_builder: Vec<AspectDeps> = Vec::new();
    let dep_list = &deps[descriptor];

    // deps[aspect] contains all aspects visible to 'aspect' in reverse order.
    for dep in dep_list.iter().rev() {
        aspect_path_builder.push(build_aspect_deps(dep, aspect_paths, deps));
    }
    let aspect_path = AspectDeps::create(descriptor.clone(), aspect_path_builder);
    aspect_paths.insert(descriptor.clone(), aspect_path.clone());
    aspect_path
}

/// Signals an inconsistency on aspect path: an aspect occurs twice on the path and the second
/// occurrence sees a different set of aspects.
///
/// [`AspectCycleOnPathException::get_aspect`] is the aspect occurring twice, and
/// [`AspectCycleOnPathException::get_previous_aspect`] is the aspect that the second occurrence
/// sees but the first does not.
// Java: com/google/devtools/build/lib/analysis/AspectCollection.java:397 AspectCollection.AspectCycleOnPathException
#[derive(Clone, Debug, thiserror::Error)]
#[error("{message}")]
pub struct AspectCycleOnPathException {
    message: String,
    aspect: AspectDescriptor,
    previous_aspect: AspectDescriptor,
}

impl AspectCycleOnPathException {
    // Java: com/google/devtools/build/lib/analysis/AspectCollection.java:402 AspectCollection.AspectCycleOnPathException.<init>
    pub fn new(
        aspect: AspectDescriptor,
        previous_aspect: AspectDescriptor,
    ) -> AspectCycleOnPathException {
        AspectCycleOnPathException {
            message: format!(
                "Aspect {} is applied twice, both before and after aspect {}",
                aspect.get_description(),
                previous_aspect.get_description()
            ),
            aspect,
            previous_aspect,
        }
    }

    pub fn get_message(&self) -> &str {
        &self.message
    }

    // Java: com/google/devtools/build/lib/analysis/AspectCollection.java:410 AspectCollection.AspectCycleOnPathException.getAspect
    pub fn get_aspect(&self) -> &AspectDescriptor {
        &self.aspect
    }

    // Java: com/google/devtools/build/lib/analysis/AspectCollection.java:414 AspectCollection.AspectCycleOnPathException.getPreviousAspect
    pub fn get_previous_aspect(&self) -> &AspectDescriptor {
        &self.previous_aspect
    }
}

/// Port of `src/test/java/.../analysis/AspectCollectionTest.java`. Expected values come from the
/// Java test.
#[cfg(test)]
mod tests {
    use std::any::Any;
    use std::collections::{HashMap, HashSet};
    use std::sync::Arc;

    use blaze_packages::aspect::{
        Aspect, AspectClass, AspectDefinition, AspectDescriptor, AspectParameters,
        NativeAspectClass,
    };
    use blaze_packages::rule_class::{ProviderKey, StarlarkProviderIdentifier};

    use super::{AspectCollection, AspectDeps};

    /// Java `new BuiltinProvider<>(name, StructImpl.class) {}.getKey()`: each anonymous subclass
    /// is its own provider class.
    fn provider_key(name: &str) -> ProviderKey {
        ProviderKey::Builtin {
            name: name.to_owned(),
            provider_class: format!("AspectCollectionTest${name}"),
        }
    }

    /// Java's anonymous `NativeAspectClass` of `createAspect`.
    ///
    /// PORT: Java keys its definition cache by the (identity-compared) anonymous class instance;
    /// `Aspect::for_native` caches by class name, so each test gives its classes a unique name
    /// prefix (`<test>:`) to keep the tests' definitions apart. Names are not otherwise observed.
    #[derive(Clone)]
    struct TestAspectClass {
        name: String,
        class_name: ProviderKey,
        required_aspects: Vec<ProviderKey>,
    }

    impl NativeAspectClass for TestAspectClass {
        fn get_name(&self) -> &str {
            &self.name
        }

        fn get_definition(&self, _aspect_parameters: &AspectParameters) -> Arc<AspectDefinition> {
            let required_providers: Vec<Vec<StarlarkProviderIdentifier>> = self
                .required_aspects
                .iter()
                .map(|k| vec![StarlarkProviderIdentifier::for_key(k.clone())])
                .collect();
            let mut builder =
                AspectDefinition::builder(AspectClass::Native(Arc::new(self.clone())));
            builder
                .require_aspects_with_providers(required_providers)
                .advertise_provider(&[StarlarkProviderIdentifier::for_key(
                    self.class_name.clone(),
                )]);
            builder.build()
        }

        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    /// Creates an aspect with a class named `class_name` advertizing a provider `class_name` that
    /// requires any of providers `required_aspects`.
    fn create_aspect(test: &str, class_name: &str, required_aspects: &[&str]) -> Aspect {
        let class: Arc<dyn NativeAspectClass> = Arc::new(TestAspectClass {
            name: format!("{test}:{class_name}"),
            class_name: provider_key(class_name),
            required_aspects: required_aspects.iter().map(|k| provider_key(k)).collect(),
        });
        Aspect::for_native_without_parameters(&class)
    }

    fn expect_deps(a: &Aspect, deps: &[&Aspect]) -> (Aspect, Vec<Aspect>) {
        (a.clone(), deps.iter().map(|d| (*d).clone()).collect())
    }

    fn validate_aspect_collection(
        collection: &AspectCollection,
        expected_used_aspects: &[&Aspect],
        expected_paths: &[(Aspect, Vec<Aspect>)],
    ) {
        let actual: Vec<&AspectDescriptor> = collection
            .get_used_aspects()
            .iter()
            .map(AspectDeps::aspect)
            .collect();
        let expected: Vec<&AspectDescriptor> = expected_used_aspects
            .iter()
            .map(|a| a.get_descriptor())
            .collect();
        assert_eq!(actual, expected);
        validate_aspect_paths(collection, expected_paths);
    }

    fn validate_aspect_paths(
        collection: &AspectCollection,
        expected_list: &[(Aspect, Vec<Aspect>)],
    ) {
        let mut all_paths: HashMap<AspectDescriptor, AspectDeps> = HashMap::new();
        for aspect_path in collection.get_used_aspects() {
            collect_and_validate_aspect_deps(aspect_path, &mut all_paths);
        }

        let mut expected_keys: HashSet<AspectDescriptor> = HashSet::new();

        for (first, second) in expected_list {
            let aspect_path = all_paths
                .get(first.get_descriptor())
                .unwrap_or_else(|| panic!("missing {}", first.get_descriptor()));
            let actual: Vec<&AspectDescriptor> = aspect_path
                .used_aspects()
                .iter()
                .map(AspectDeps::aspect)
                .collect();
            let expected: Vec<&AspectDescriptor> =
                second.iter().map(|a| a.get_descriptor()).collect();
            assert_eq!(actual, expected);
            expected_keys.insert(first.get_descriptor().clone());
        }
        let keys: HashSet<AspectDescriptor> = all_paths.keys().cloned().collect();
        assert_eq!(keys, expected_keys);
    }

    /// Collects all aspect paths transitively visible from `aspect_deps`. Validates that the
    /// `AspectDeps` instance corresponding to a given `AspectDescriptor` is unique.
    fn collect_and_validate_aspect_deps(
        aspect_deps: &AspectDeps,
        all_deps: &mut HashMap<AspectDescriptor, AspectDeps>,
    ) {
        if let Some(existing) = all_deps.get(aspect_deps.aspect()) {
            assert!(
                existing.is_same_instance(aspect_deps),
                "Two different deps for aspect {}",
                aspect_deps.aspect()
            );
            return;
        }
        all_deps.insert(aspect_deps.aspect().clone(), aspect_deps.clone());
        for path in aspect_deps.used_aspects() {
            collect_and_validate_aspect_deps(path, all_deps);
        }
    }

    /// a3 wants a1 and a2, a1 and a2 want no one, path is a1, a2, a3.
    #[test]
    fn linear_aspect_path1() {
        let t = "linear_aspect_path1";
        let a1 = create_aspect(t, "a1", &[]);
        let a2 = create_aspect(t, "a2", &[]);
        let a3 = create_aspect(t, "a3", &["a1", "a2"]);
        let collection = AspectCollection::create(&[a1.clone(), a2.clone(), a3.clone()]).unwrap();
        validate_aspect_collection(
            &collection,
            &[&a1, &a2, &a3],
            &[
                expect_deps(&a3, &[&a1, &a2]),
                expect_deps(&a1, &[]),
                expect_deps(&a2, &[]),
            ],
        );
    }

    /// a3 wants a2, a2 wants a1, a1 wants no one, path is a1, a2, a3.
    #[test]
    fn linear_aspect_path2() {
        let t = "linear_aspect_path2";
        let a1 = create_aspect(t, "a1", &[]);
        let a2 = create_aspect(t, "a2", &["a1"]);
        let a3 = create_aspect(t, "a3", &["a2"]);
        let collection = AspectCollection::create(&[a1.clone(), a2.clone(), a3.clone()]).unwrap();
        validate_aspect_collection(
            &collection,
            &[&a1, &a2, &a3],
            &[
                expect_deps(&a3, &[&a2]),
                expect_deps(&a2, &[&a1]),
                expect_deps(&a1, &[]),
            ],
        );
    }

    /// a3 wants a1, a1 wants a2, path is a1, a2, a3, so a2 comes after a1.
    #[test]
    fn validate_order() {
        let t = "validate_order";
        let a1 = create_aspect(t, "a1", &["a2"]);
        let a2 = create_aspect(t, "a2", &[]);
        let a3 = create_aspect(t, "a3", &["a1"]);
        let collection = AspectCollection::create(&[a1.clone(), a2.clone(), a3.clone()]).unwrap();
        validate_aspect_collection(
            &collection,
            &[&a1, &a2, &a3],
            &[
                expect_deps(&a1, &[]),
                expect_deps(&a2, &[]),
                expect_deps(&a3, &[&a1]),
            ],
        );
    }

    /// a3 wants a1, a1 wants a2, a2 wants a1, path is a1, a2, a3, so a2 comes after a1.
    #[test]
    fn validate_order2() {
        let t = "validate_order2";
        let a1 = create_aspect(t, "a1", &["a2"]);
        let a2 = create_aspect(t, "a2", &["a1"]);
        let a3 = create_aspect(t, "a3", &["a1"]);
        let collection = AspectCollection::create(&[a1.clone(), a2.clone(), a3.clone()]).unwrap();
        validate_aspect_collection(
            &collection,
            &[&a1, &a2, &a3],
            &[
                expect_deps(&a1, &[]),
                expect_deps(&a2, &[&a1]),
                expect_deps(&a3, &[&a1]),
            ],
        );
    }

    /// a3 wants itself.
    #[test]
    fn recursive() {
        let t = "recursive";
        let a1 = create_aspect(t, "a1", &[]);
        let a2 = create_aspect(t, "a2", &[]);
        let a3 = create_aspect(t, "a3", &["a3"]);
        let collection = AspectCollection::create(&[a1.clone(), a2.clone(), a3.clone()]).unwrap();
        validate_aspect_collection(
            &collection,
            &[&a1, &a2, &a3],
            &[
                expect_deps(&a1, &[]),
                expect_deps(&a2, &[]),
                expect_deps(&a3, &[]),
            ],
        );
    }

    /// a2 wants a1, a3 wants nothing.
    #[test]
    fn three_aspects() {
        let t = "three_aspects";
        let a1 = create_aspect(t, "a1", &[]);
        let a2 = create_aspect(t, "a2", &["a1"]);
        let a3 = create_aspect(t, "a3", &[]);
        let collection = AspectCollection::create(&[a1.clone(), a2.clone(), a3.clone()]).unwrap();
        validate_aspect_collection(
            &collection,
            &[&a1, &a2, &a3],
            &[
                expect_deps(&a3, &[]),
                expect_deps(&a2, &[&a1]),
                expect_deps(&a1, &[]),
            ],
        );
    }

    /// a2 wants a1, a3 wants a1 and a2, the path is [a2, a1, a2, a3], so a2 occurs twice.
    ///
    /// First occurrence of a2 would not see a1, but the second would: that is an error.
    #[test]
    fn duplicate_aspect() {
        let t = "duplicate_aspect";
        let a1 = create_aspect(t, "a1", &[]);
        let a2 = create_aspect(t, "a2", &["a1"]);
        let a3 = create_aspect(t, "a3", &["a2", "a1"]);
        let e = AspectCollection::create(&[a2.clone(), a1.clone(), a2.clone(), a3.clone()])
            .unwrap_err();
        assert_eq!(e.get_aspect(), a2.get_descriptor());
        assert_eq!(e.get_previous_aspect(), a1.get_descriptor());
    }

    /// a2 wants a1, a3 wants a2, the path is [a2, a1, a2, a3], so a2 occurs twice.
    ///
    /// First occurrence of a2 would not see a1, but the second would: that is an error.
    #[test]
    fn duplicate_aspect2() {
        let t = "duplicate_aspect2";
        let a1 = create_aspect(t, "a1", &[]);
        let a2 = create_aspect(t, "a2", &["a1"]);
        let a3 = create_aspect(t, "a3", &["a2"]);
        let e = AspectCollection::create(&[a2.clone(), a1.clone(), a2.clone(), a3.clone()])
            .unwrap_err();
        assert_eq!(e.get_aspect(), a2.get_descriptor());
        assert_eq!(e.get_previous_aspect(), a1.get_descriptor());
    }

    /// a3 wants a1 and a2, a2 does not want a1. The path is [a2, a1, a2, a3], so a2 occurs twice.
    /// Second occurrence of a2 is consistent with the first.
    #[test]
    fn duplicate_aspect2a() {
        let t = "duplicate_aspect2a";
        let a1 = create_aspect(t, "a1", &[]);
        let a2 = create_aspect(t, "a2", &[]);
        let a3 = create_aspect(t, "a3", &["a1", "a2"]);

        let collection =
            AspectCollection::create(&[a2.clone(), a1.clone(), a2.clone(), a3.clone()]).unwrap();

        validate_aspect_collection(
            &collection,
            &[&a2, &a1, &a3],
            &[
                expect_deps(&a2, &[]),
                expect_deps(&a1, &[]),
                expect_deps(&a3, &[&a2, &a1]),
            ],
        );
    }

    /// a2 wants a1, a3 wants a1 and a2, a1 wants a2. the path is [a2, a1, a2, a3], so a2 occurs
    /// twice. First occurrence of a2 does not see a1, but the second does => error.
    #[test]
    fn duplicate_aspect3() {
        let t = "duplicate_aspect3";
        let a1 = create_aspect(t, "a1", &["a2"]);
        let a2 = create_aspect(t, "a2", &["a1"]);
        let a3 = create_aspect(t, "a3", &["a1", "a2"]);
        let e = AspectCollection::create(&[a2.clone(), a1.clone(), a2.clone(), a3.clone()])
            .unwrap_err();
        assert_eq!(e.get_aspect(), a2.get_descriptor());
        assert_eq!(e.get_previous_aspect(), a1.get_descriptor());
    }

    /// a2 wants a1, a3 wants a2, a1 wants a2. the path is [a2, a1, a2, a3], so a2 occurs twice.
    /// First occurrence of a2 does not see a1, but the second does => error. a1 disappears.
    #[test]
    fn duplicate_aspect4() {
        let t = "duplicate_aspect4";
        let a1 = create_aspect(t, "a1", &["a2"]);
        let a2 = create_aspect(t, "a2", &["a1"]);
        let a3 = create_aspect(t, "a3", &["a2"]);
        let e = AspectCollection::create(&[a2.clone(), a1.clone(), a2.clone(), a3.clone()])
            .unwrap_err();
        assert_eq!(e.get_aspect(), a2.get_descriptor());
        assert_eq!(e.get_previous_aspect(), a1.get_descriptor());
    }

    /// a3 wants a2, a1 wants a2. The path is [a2, a1, a2, a3], so a2 occurs twice. First
    /// occurrence of a2 is consistent with the second. a1 disappears.
    #[test]
    fn duplicate_aspect5() {
        let t = "duplicate_aspect5";
        let a1 = create_aspect(t, "a1", &["a2"]);
        let a2 = create_aspect(t, "a2", &[]);
        let a3 = create_aspect(t, "a3", &["a2"]);
        let collection =
            AspectCollection::create(&[a2.clone(), a1.clone(), a2.clone(), a3.clone()]).unwrap();
        validate_aspect_collection(
            &collection,
            &[&a2, &a1, &a3],
            &[
                expect_deps(&a2, &[]),
                expect_deps(&a1, &[&a2]),
                expect_deps(&a3, &[&a2]),
            ],
        );
    }
}
