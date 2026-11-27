//! Port of the local-resources tests of
//! src/test/java/com/google/devtools/build/lib/actions/SpawnTest.java.
//!
//! PORT: Java builds the spawns with `SpawnBuilder` and `BaseSpawn`; Blaze has `SimpleSpawn`,
//! whose `getLocalResources` is the same `ResourceSetOrBuilder.buildLocalResources` call.
//! `ResourceSetOrBuilder.ignoringOverrides` is a test-local builder here.

use std::sync::Arc;

use blaze_collect::{NestedSetBuilder, Order};
use blaze_exec::actions::{
    ExecException, FakeOwner, PathMapper, ResourceSet, ResourceSetOrBuilder, SimpleSpawn, Spawn,
};
use indexmap::IndexMap;

// Java: src/test/java/com/google/devtools/build/lib/actions/SpawnTest.java:30 SpawnTest.DECLARED_RESOURCES
fn declared_resources() -> ResourceSet {
    ResourceSet::create_with_ram_cpu(/* memory_mb= */ 250.0, /* cpu= */ 1.0)
}

/// `ResourceSetOrBuilder.ignoringOverrides(resources)`.
// Java: com/google/devtools/build/lib/actions/ResourceSetOrBuilder.java:52 ResourceSetOrBuilder.ignoringOverrides
#[derive(Debug)]
struct IgnoringOverrides(ResourceSet);

impl ResourceSetOrBuilder for IgnoringOverrides {
    fn build_resource_set(&self, _inputs_size: i32) -> ResourceSet {
        self.0.clone()
    }
    fn build_local_resources(
        &self,
        _inputs_size: i32,
        _execution_info: &IndexMap<String, String>,
        _exec_properties: &IndexMap<String, String>,
    ) -> Result<ResourceSet, ExecException> {
        Ok(self.0.clone())
    }
}

fn spawn(
    local_resources: Arc<dyn ResourceSetOrBuilder>,
    execution_info: &[(&str, &str)],
    exec_properties: &[(&str, &str)],
) -> SimpleSpawn {
    let mut owner = FakeOwner::new("Mnemonic", None, None);
    owner.owner.exec_properties = exec_properties
        .iter()
        .map(|(k, v)| (k.to_string(), v.to_string()))
        .collect();
    SimpleSpawn::new(
        Arc::new(owner),
        vec!["/bin/true".to_string()],
        IndexMap::new(),
        execution_info
            .iter()
            .map(|(k, v)| (k.to_string(), v.to_string()))
            .collect(),
        NestedSetBuilder::empty_set(Order::STABLE_ORDER),
        NestedSetBuilder::empty_set(Order::STABLE_ORDER),
        Vec::new(),
        None,
        local_resources,
        PathMapper::NOOP,
    )
}

// Java: src/test/java/com/google/devtools/build/lib/actions/SpawnTest.java:34 SpawnTest.getLocalResources_appliesResourceOverrides
#[test]
fn get_local_resources_applies_resource_overrides() {
    let spawn = spawn(
        Arc::new(declared_resources()),
        &[("resources:cpu:4", "")],
        &[("resources:memory", "8000")],
    );
    let resources = spawn.get_local_resources().unwrap();

    // Execution info and exec properties both override the declared amounts.
    assert_eq!(resources.get_cpu_usage(), 4.0);
    assert_eq!(resources.get_memory_mb(), 8000.0);
}

// Java: src/test/java/com/google/devtools/build/lib/actions/SpawnTest.java:49 SpawnTest.getLocalResources_fixedDeclaration_ignoresResourceOverrides
#[test]
fn get_local_resources_fixed_declaration_ignores_resource_overrides() {
    let spawn = spawn(
        Arc::new(IgnoringOverrides(declared_resources())),
        &[("resources:cpu:4", "")],
        &[("resources:memory", "8000")],
    );

    assert_eq!(spawn.get_local_resources().unwrap(), declared_resources());
    // The declarations still reach the spawn, so strategy selection is unaffected.
    assert!(spawn.get_execution_info().contains_key("resources:cpu:4"));
    assert_eq!(
        spawn
            .get_combined_exec_properties()
            .get("resources:memory")
            .map(String::as_str),
        Some("8000")
    );
}

// Java: src/test/java/com/google/devtools/build/lib/actions/SpawnTest.java:75 SpawnTest.baseSpawn_getLocalResources_appliesResourceOverrides
#[test]
fn base_spawn_get_local_resources_applies_resource_overrides() {
    let resources = spawn(
        Arc::new(declared_resources()),
        &[("resources:cpu:4", "")],
        &[],
    )
    .get_local_resources()
    .unwrap();

    assert_eq!(resources.get_cpu_usage(), 4.0);
}

// Java: src/test/java/com/google/devtools/build/lib/actions/SpawnTest.java:82 SpawnTest.baseSpawn_getLocalResources_fixedDeclaration_keepsDeclaredResources
#[test]
fn base_spawn_get_local_resources_fixed_declaration_keeps_declared_resources() {
    let spawn = spawn(
        Arc::new(IgnoringOverrides(declared_resources())),
        &[("resources:cpu:4", "")],
        &[],
    );

    assert_eq!(spawn.get_local_resources().unwrap(), declared_resources());
    assert!(spawn.get_execution_info().contains_key("resources:cpu:4"));
}

/// `Spawns.getTimeout` keeps a negative `timeout` execution requirement negative
/// (`Duration.ofSeconds(Integer.parseInt(timeoutStr))`).
#[test]
fn get_timeout_negative_stays_negative() {
    let spawn = spawn(Arc::new(declared_resources()), &[("timeout", "-1")], &[]);
    assert_eq!(
        blaze_exec::actions::spawns::get_timeout(&spawn).unwrap(),
        blaze_util::java_duration::JavaDuration::of_seconds(-1)
    );
}

/// `ProcessWrapper.CommandLineBuilder.build`: `--timeout=` and `--kill_delay=` carry
/// `Duration.toSeconds()`, negative values included.
#[test]
fn process_wrapper_negative_timeout_and_kill_delay() {
    use blaze_exec::actions::{ActionInputRef, BasicActionInput};
    use blaze_exec::runtime::process_wrapper::ProcessWrapper;
    use blaze_util::java_duration::JavaDuration;
    let process_wrapper = ProcessWrapper::new(
        blaze_vfs::PathFragment::create("/pw"),
        ActionInputRef::new(BasicActionInput::from_path("/pw")),
        Some(JavaDuration::of_seconds(-1)),
        false,
    );
    let mut builder = process_wrapper.command_line_builder(vec!["/bin/true".to_string()]);
    builder.set_timeout(JavaDuration::of_seconds(-1));
    let args = builder.build();
    assert!(args.contains(&"--timeout=-1".to_string()), "{args:?}");
    assert!(args.contains(&"--kill_delay=-1".to_string()), "{args:?}");
}
