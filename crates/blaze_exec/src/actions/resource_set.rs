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
    // Java: com/google/devtools/build/lib/actions/ResourceSet.java:51 ResourceSet.createWithRamCpu
    pub fn create_with_ram_cpu(memory_mb: f64, cpu: f64) -> ResourceSet {
        ResourceSet::create(memory_mb, cpu, 0)
    }

    /// `createWithLocalTestCount(localTestCount)`.
    // Java: com/google/devtools/build/lib/actions/ResourceSet.java:55 ResourceSet.createWithLocalTestCount
    pub fn create_with_local_test_count(local_test_count: i32) -> ResourceSet {
        ResourceSet::create_from_map(IndexMap::new(), local_test_count)
    }

    /// `create(memoryMb, cpu, localTestCount)`.
    // Java: com/google/devtools/build/lib/actions/ResourceSet.java:63 ResourceSet.create
    pub fn create(memory_mb: f64, cpu: f64, local_test_count: i32) -> ResourceSet {
        let mut m = IndexMap::new();
        m.insert(ResourceSet::MEMORY.to_string(), memory_mb);
        m.insert(ResourceSet::CPU.to_string(), cpu);
        ResourceSet::create_from_map(m, local_test_count)
    }

    /// `create(ImmutableMap resources, localTestCount)`.
    // Java: com/google/devtools/build/lib/actions/ResourceSet.java:67 ResourceSet.create
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
    // Java: com/google/devtools/build/lib/actions/ResourceSet.java:82 ResourceSet.withResourceOverrides
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
    // Java: com/google/devtools/build/lib/actions/ResourceSet.java:102 ResourceSet.get
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
    // Java: com/google/devtools/build/lib/actions/ResourceSet.java:123 ResourceSet.getResources
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

/// Converter for [`ResourceSet`].
// Java: com/google/devtools/build/lib/actions/ResourceSet.java:153 ResourceSet.ResourceSetConverter
#[derive(Clone, Copy, Debug, Default)]
pub struct ResourceSetConverter;

impl ResourceSetConverter {
    // Java: com/google/devtools/build/lib/actions/ResourceSet.java:156 ResourceSet.ResourceSetConverter.convert
    pub fn convert(
        &self,
        input: &str,
    ) -> Result<ResourceSet, blaze_options::OptionsParsingException> {
        let expected = || {
            blaze_options::OptionsParsingException::new(
                "Expected exactly 3 comma-separated float values",
            )
        };
        // Splitter.on(','): every piece, empty ones included.
        let mut values = input.split(',');
        let mut next_double = || -> Result<f64, blaze_options::OptionsParsingException> {
            values
                .next()
                .and_then(blaze_options::converters::java_parse_double)
                .ok_or_else(expected)
        };
        let memory_mb = next_double()?;
        let cpu_usage = next_double()?;
        // There used to be a third field here called ioUsage. In order to not break existing
        // users, we keep expecting a third field, which must be a double. In the future, we may
        // accept the two-param variant, and then even phase out the three-param variant.
        next_double()?;
        if values.next().is_some() {
            return Err(expected());
        }
        if memory_mb <= 0.0 || cpu_usage <= 0.0 {
            return Err(blaze_options::OptionsParsingException::new(
                "All resource values must be positive",
            ));
        }
        Ok(ResourceSet::create(memory_mb, cpu_usage, i32::MAX))
    }

    // Java: com/google/devtools/build/lib/actions/ResourceSet.java:178 ResourceSet.ResourceSetConverter.getTypeDescription
    pub fn get_type_description(&self) -> String {
        "comma-separated available amount of RAM (in MB), CPU (in cores) and available I/O (1.0 being average workstation)".to_string()
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
        Ok(self
            .try_build_resource_set(inputs_size)?
            .with_resource_overrides(&[
                crate::actions::execution_requirements::parse_resources(execution_info)?,
                crate::actions::execution_requirements::parse_resources(exec_properties)?,
            ]))
    }
}

impl ResourceSetOrBuilder for ResourceSet {
    // Java: com/google/devtools/build/lib/actions/ResourceSet.java:185 ResourceSet.buildResourceSet
    fn build_resource_set(&self, _inputs_size: i32) -> ResourceSet {
        self.clone()
    }
}
