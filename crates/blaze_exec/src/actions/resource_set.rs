//! Port of `lib/actions/ResourceSet.java` and `ResourceSetOrBuilder.java`.

use std::fmt;
use std::sync::Arc;

use super::exec_exception::ExecException;
use crate::worker::worker_key::WorkerKey;
use indexmap::IndexMap;

/// Resources needed to execute a spawn locally.
// Java: com/google/devtools/build/lib/actions/ResourceSet.java:39 ResourceSet
#[derive(Clone, Debug, PartialEq, Default)]
pub struct ResourceSet {
    resources: IndexMap<String, f64>,
    local_test_count: i32,
    /// The workerKey of used worker. Null if no worker is used.
    worker_key: Option<Arc<WorkerKey>>,
}

impl ResourceSet {
    /// `ResourceSet.CPU`.
    pub const CPU: &'static str = "cpu";
    /// `ResourceSet.MEMORY`.
    pub const MEMORY: &'static str = "memory";

    /// `ResourceSet.ZERO`.
    pub fn zero() -> ResourceSet {
        ResourceSet::default()
    }

    /// `createWithRamCpu(memoryMb, cpu)`.
    pub fn create_with_ram_cpu(memory_mb: f64, cpu: f64) -> ResourceSet {
        ResourceSet::create(memory_mb, cpu, 0)
    }

    /// `createWithLocalTestCount(localTestCount)`.
    pub fn create_with_local_test_count(local_test_count: i32) -> ResourceSet {
        ResourceSet::create_from_map(IndexMap::new(), local_test_count)
    }

    /// `create(memoryMb, cpu, localTestCount)`.
    pub fn create(memory_mb: f64, cpu: f64, local_test_count: i32) -> ResourceSet {
        let mut m = IndexMap::new();
        m.insert(ResourceSet::MEMORY.to_string(), memory_mb);
        m.insert(ResourceSet::CPU.to_string(), cpu);
        ResourceSet::create_from_map(m, local_test_count)
    }

    /// `create(ImmutableMap resources, localTestCount)`.
    pub fn create_from_map(resources: IndexMap<String, f64>, local_test_count: i32) -> ResourceSet {
        ResourceSet {
            resources,
            local_test_count,
            worker_key: None,
        }
    }

    /// `create(ImmutableMap resources, localTestCount, workerKey)`.
    // Java: com/google/devtools/build/lib/actions/ResourceSet.java:71 ResourceSet.create
    pub fn create_with_worker_key(
        resources: IndexMap<String, f64>,
        local_test_count: i32,
        worker_key: Option<Arc<WorkerKey>>,
    ) -> ResourceSet {
        ResourceSet {
            resources,
            local_test_count,
            worker_key,
        }
    }

    /// Returns the workerKey of worker. If there is no worker requested, then returns `None`.
    // Java: com/google/devtools/build/lib/actions/ResourceSet.java:119 ResourceSet.getWorkerKey
    pub fn get_worker_key(&self) -> Option<&Arc<WorkerKey>> {
        self.worker_key.as_ref()
    }

    /// `withResourceOverrides(overrides...)`.
    pub fn with_resource_overrides(&self, overrides: &[IndexMap<String, f64>]) -> ResourceSet {
        if overrides.iter().all(|o| o.is_empty()) {
            return self.clone();
        }
        let mut m = self.resources.clone();
        for o in overrides {
            for (k, v) in o {
                m.insert(k.clone(), *v);
            }
        }
        ResourceSet::create_with_worker_key(m, self.local_test_count, self.worker_key.clone())
    }

    /// `get(resource)`.
    pub fn get(&self, resource: &str) -> f64 {
        self.resources.get(resource).copied().unwrap_or(0.0)
    }
    /// `getMemoryMb()`.
    pub fn get_memory_mb(&self) -> f64 {
        self.get(ResourceSet::MEMORY)
    }
    /// `getCpuUsage()`.
    pub fn get_cpu_usage(&self) -> f64 {
        self.get(ResourceSet::CPU)
    }
    /// `getResources()`.
    pub fn get_resources(&self) -> &IndexMap<String, f64> {
        &self.resources
    }
    /// `getLocalTestCount()`.
    pub fn get_local_test_count(&self) -> i32 {
        self.local_test_count
    }
}

fn java_double(v: Option<&f64>) -> String {
    match v {
        None => "null".to_string(),
        Some(d) => {
            if d.fract() == 0.0 && d.abs() < 1e7 {
                format!("{d:.1}")
            } else {
                format!("{d}")
            }
        }
    }
}

impl fmt::Display for ResourceSet {
    // Java: com/google/devtools/build/lib/actions/ResourceSet.java:146 ResourceSet.toString
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Resources: \nMemory: {}M\nCPU: {}\n",
            java_double(self.resources.get(ResourceSet::MEMORY)),
            java_double(self.resources.get(ResourceSet::CPU))
        )?;
        for (k, v) in &self.resources {
            if k != ResourceSet::CPU && k != ResourceSet::MEMORY {
                writeln!(f, "{}: {}", k, java_double(Some(v)))?;
            }
        }
        writeln!(f, "Local tests: {}", self.local_test_count)
    }
}

/// Port of `ResourceSetOrBuilder`: a fixed `ResourceSet` or a function of the input count.
// Java: com/google/devtools/build/lib/actions/ResourceSetOrBuilder.java:22 ResourceSetOrBuilder
pub trait ResourceSetOrBuilder: Send + Sync + fmt::Debug {
    /// `buildResourceSet(OS os, int inputsSize)`.
    fn build_resource_set(&self, inputs_size: i32) -> ResourceSet;

    /// `buildResourceSet(OS os, int inputsSize) throws ExecException`: the checked form.
    ///
    /// PORT: Java's `buildResourceSet` throws `ExecException`; the infallible
    /// [`build_resource_set`](Self::build_resource_set) is kept for the implementations in other
    /// crates, and a builder that can fail (the Starlark `resource_set` callback) overrides this.
    // Java: com/google/devtools/build/lib/actions/ResourceSetOrBuilder.java:29 ResourceSetOrBuilder.buildResourceSet
    fn try_build_resource_set(&self, inputs_size: i32) -> Result<ResourceSet, ExecException> {
        Ok(self.build_resource_set(inputs_size))
    }

    /// `buildLocalResources(OS, inputsSize, executionInfo, execProperties)`: the resource set with
    /// `resources:<name>` execution-info and exec-property overrides applied.
    // Java: com/google/devtools/build/lib/actions/ResourceSetOrBuilder.java:37 ResourceSetOrBuilder.buildLocalResources
    fn build_local_resources(
        &self,
        inputs_size: i32,
        execution_info: &IndexMap<String, String>,
        exec_properties: &IndexMap<String, String>,
    ) -> Result<ResourceSet, ExecException> {
        let _ = (execution_info, exec_properties);
        // PORT: the "resources:" override parsing of ResourceSetOrBuilder.buildLocalResources is
        // not ported; no corpus spawn sets it. A spawn that does is reported loudly.
        if execution_info.keys().any(|k| k.starts_with("resources:"))
            || exec_properties.keys().any(|k| k.starts_with("resources:"))
        {
            blaze_util::unported!("ResourceSetOrBuilder.buildLocalResources(resources: overrides)");
        }
        self.try_build_resource_set(inputs_size)
    }
}

impl ResourceSetOrBuilder for ResourceSet {
    fn build_resource_set(&self, _inputs_size: i32) -> ResourceSet {
        self.clone()
    }
}
