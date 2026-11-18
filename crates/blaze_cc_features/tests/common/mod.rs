//! Test support: evaluates the Java tests' Starlark toolchain fixtures with starlark-rust and
//! rules_cc 0.2.17's `cc_toolchain_config_lib.bzl` (tests/data/rules_cc), plus the Java tests'
//! helper types.

#![allow(dead_code)]

use std::collections::HashMap;
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use blaze_cc_features::actions::{Artifact, ArtifactRef, InputMetadataProvider, PathMapper};
use blaze_cc_features::cc_toolchain_variables::{
    VariableObject, VariableValue, VariableValueAdapter,
};
use blaze_cc_features::errors::ExpansionException;
use blaze_cc_features::starlark_info::{StarlarkInfo, StarlarkInfoValue};
use blaze_cc_features::{
    CcToolchainConfigInfo, CcToolchainFeatures, CcToolchainVariables, CcToolchainVariablesRef,
};
use blaze_vfs::PathFragment;
use indexmap::IndexMap;
use starlark::environment::{FrozenModule, Globals, LibraryExtension, Module};
use starlark::eval::{Evaluator, ReturnFileLoader};
use starlark::syntax::{AstModule, Dialect};
use starlark::values::{Heap, Value};

const DATA: &str = concat!(env!("CARGO_MANIFEST_DIR"), "/tests/data/rules_cc");

fn read_data(name: &str) -> String {
    std::fs::read_to_string(format!("{DATA}/{name}")).unwrap()
}

const PROVIDER_PRELUDE: &str = "def provider(doc = None, fields = None, init = None):\n    def _instantiate(**kwargs):\n        return struct(**kwargs)\n    return _instantiate\n";

fn globals() -> Globals {
    Globals::extended_by(&[
        LibraryExtension::StructType,
        LibraryExtension::SetType,
        LibraryExtension::Print,
    ])
}

fn eval_frozen(
    name: &str,
    src: String,
    deps: &HashMap<&str, &FrozenModule>,
) -> Result<FrozenModule, String> {
    let ast = AstModule::parse(name, src, &Dialect::Extended).map_err(|e| format!("{e}"))?;
    let globals = globals();
    let loader = ReturnFileLoader { modules: deps };
    Module::with_temp_heap(|module| {
        {
            let mut eval = Evaluator::new(&module);
            eval.set_loader(&loader);
            eval.eval_module(ast, &globals)
                .map_err(|e| format!("{e}"))?;
        }
        module.freeze().map_err(|e| format!("{e:?}"))
    })
}

fn convert<'v>(v: Value<'v>, heap: Heap<'v>) -> StarlarkInfoValue {
    if v.is_none() {
        return StarlarkInfoValue::None;
    }
    if let Some(b) = v.unpack_bool() {
        return StarlarkInfoValue::Bool(b);
    }
    if let Some(i) = v.unpack_i32() {
        return StarlarkInfoValue::Int(i as i64);
    }
    if let Some(s) = v.unpack_str() {
        return StarlarkInfoValue::String(s.to_string());
    }
    match v.get_type() {
        "list" | "tuple" => {
            StarlarkInfoValue::List(v.iterate(heap).unwrap().map(|e| convert(e, heap)).collect())
        }
        "struct" => {
            let mut info = StarlarkInfo::new("struct", "<starlark-rust>");
            for name in v.dir_attr() {
                let field = v.get_attr(&name, heap).unwrap().unwrap();
                info.fields.insert(name, convert(field, heap));
            }
            StarlarkInfoValue::Info(Arc::new(info))
        }
        other => StarlarkInfoValue::Other(other.to_string()),
    }
}

/// Evaluates `create_cc_toolchain_config_info(ctx = ctx, <content>, ...)` like the Java tests'
/// `crosstool.bzl` and returns the resulting CcToolchainConfigInfo struct.
pub fn build_config_info(content: &[&str]) -> Result<Arc<StarlarkInfo>, String> {
    // starlark-rust has no `provider`; provider instances become structs (same fields).
    let lib = eval_frozen(
        "cc_toolchain_config_lib.bzl",
        format!(
            "{PROVIDER_PRELUDE}{}",
            read_data("cc_toolchain_config_lib.bzl")
        ),
        &HashMap::new(),
    )?;
    let action_names = eval_frozen(
        "action_names.bzl",
        read_data("action_names.bzl"),
        &HashMap::new(),
    )?;
    let mut deps: HashMap<&str, &FrozenModule> = HashMap::new();
    deps.insert("//cc:cc_toolchain_config_lib.bzl", &lib);
    deps.insert("//cc:action_names.bzl", &action_names);
    let legacy = eval_frozen(
        "legacy_features.bzl",
        read_data("legacy_features.bzl"),
        &deps,
    )?;
    let mut deps2: HashMap<&str, &FrozenModule> = HashMap::new();
    deps2.insert("//cc/private/toolchain_config:legacy_features.bzl", &legacy);
    let info = eval_frozen(
        "cc_toolchain_config_info.bzl",
        read_data("cc_toolchain_config_info.bzl"),
        &deps2,
    )?;
    let mut fixture_deps: HashMap<&str, &FrozenModule> = HashMap::new();
    fixture_deps.insert("//tools/cpp:cc_toolchain_config_lib.bzl", &lib);
    fixture_deps.insert("//cc:cc_toolchain_config_info.bzl", &info);

    let src = [
        "load(",
        "    '//tools/cpp:cc_toolchain_config_lib.bzl',",
        "    'action_config',",
        "    'artifact_name_pattern',",
        "    'env_entry',",
        "    'env_set',",
        "    'feature',",
        "    'feature_set',",
        "    'flag_group',",
        "    'flag_set',",
        "    'tool',",
        "    'variable_with_value',",
        "    'with_feature_set',",
        ")",
        "load('//cc:cc_toolchain_config_info.bzl', 'create_cc_toolchain_config_info')",
        "",
        "ctx = struct(label = struct(workspace_root = '', package = 'crosstool'), attr = struct())",
        "config = create_cc_toolchain_config_info(",
        "        ctx = ctx,",
        &if content.is_empty() {
            String::new()
        } else {
            content.join("\n") + ","
        },
        "        toolchain_identifier = 'toolchain',",
        "        host_system_name = 'host',",
        "        target_system_name = 'target',",
        "        target_cpu = 'cpu',",
        "        target_libc = 'libc',",
        "        compiler = 'compiler',",
        "    )",
    ]
    .join("\n");
    let ast =
        AstModule::parse("crosstool.bzl", src, &Dialect::Extended).map_err(|e| format!("{e}"))?;
    let globals = globals();
    let loader = ReturnFileLoader {
        modules: &fixture_deps,
    };
    Module::with_temp_heap(|module| {
        let mut eval = Evaluator::new(&module);
        eval.set_loader(&loader);
        eval.eval_module(ast, &globals)
            .map_err(|e| format!("{e}"))?;
        let config = module.get("config").unwrap();
        match convert(config, module.heap()) {
            StarlarkInfoValue::Info(i) => Ok(i),
            other => Err(format!("not a struct: {other:?}")),
        }
    })
}

/// Java: `buildFeatures(content...)` with `tools_directory = "crosstool"`. Errors (Starlark
/// `fail` or `EvalException`) come back as their message, like the Java tests' AssertionError.
pub fn build_features(content: &[&str]) -> Result<CcToolchainFeatures, String> {
    build_features_with_tools_directory(content, "crosstool")
}

pub fn build_features_with_tools_directory(
    content: &[&str],
    tools_directory: &str,
) -> Result<CcToolchainFeatures, String> {
    let info = build_config_info(content)?;
    CcToolchainFeatures::new(
        &CcToolchainConfigInfo::new(info),
        PathFragment::create(tools_directory),
    )
    .map_err(|e| e.message)
}

/// Java: `createVariables(entries...)`: repeated keys become string sequences.
pub fn create_variables(entries: &[&str]) -> CcToolchainVariablesRef {
    assert!(
        entries.len().is_multiple_of(2),
        "createVariables takes an even number of arguments (key/value pairs)"
    );
    let mut entry_map: IndexMap<&str, Vec<&str>> = IndexMap::new();
    for pair in entries.chunks(2) {
        entry_map.entry(pair[0]).or_default().push(pair[1]);
    }
    let mut variables = CcToolchainVariables::builder();
    for (name, value) in entry_map {
        if value.len() == 1 {
            variables.add_variable(name, VariableObject::string(value[0]));
        } else {
            variables.add_string_sequence_variable(name, value);
        }
    }
    variables.build()
}

/// The Java test's `StructureValue` record.
#[derive(Debug)]
pub struct StructureValue {
    pub value: IndexMap<String, VariableValue>,
}

impl VariableValueAdapter for StructureValue {
    fn get_variable_type_name(&self) -> String {
        "structure".to_string()
    }

    fn is_truthy(&self) -> bool {
        !self.value.is_empty()
    }

    fn get_field_value(
        &self,
        _variable_name: &str,
        field: &str,
        _input_metadata_provider: Option<&dyn InputMetadataProvider>,
        _path_mapper: &dyn PathMapper,
        _throw_on_missing_variable: bool,
    ) -> Result<Option<VariableValue>, ExpansionException> {
        Ok(self.value.get(field).cloned())
    }
}

/// The Java test's `StructureBuilder`.
#[derive(Default)]
pub struct StructureBuilder {
    fields: IndexMap<String, VariableValue>,
}

impl StructureBuilder {
    pub fn new() -> StructureBuilder {
        StructureBuilder::default()
    }

    pub fn add_field_value(mut self, name: &str, value: VariableValue) -> Self {
        self.fields.insert(name.to_string(), value);
        self
    }

    pub fn add_field_builder(self, name: &str, builder: StructureBuilder) -> Self {
        let v = builder.build();
        self.add_field_value(name, v)
    }

    pub fn add_field(self, name: &str, value: &str) -> Self {
        self.add_field_value(
            name,
            VariableValue::String(blaze_cc_features::cc_toolchain_variables::StringValue::new(
                value,
            )),
        )
    }

    pub fn add_field_list(self, name: &str, values: &[&str]) -> Self {
        self.add_field_value(
            name,
            VariableValue::Sequence(blaze_cc_features::cc_toolchain_variables::Sequence::new(
                values.iter().map(|s| VariableObject::string(s)).collect(),
            )),
        )
    }

    pub fn build(self) -> VariableValue {
        VariableValue::Custom(Arc::new(StructureValue { value: self.fields }))
    }
}

pub fn create_structure_variables(name: &str, value: StructureBuilder) -> CcToolchainVariablesRef {
    let mut b = CcToolchainVariables::builder();
    b.add_variable(name, VariableObject::Value(value.build()));
    b.build()
}

pub fn create_structure_sequence_variables(
    name: &str,
    values: Vec<VariableValue>,
) -> CcToolchainVariablesRef {
    let mut b = CcToolchainVariables::builder();
    b.add_variable(
        name,
        VariableObject::list(values.into_iter().map(VariableObject::Value).collect()),
    );
    b.build()
}

/// A Starlark `struct(...)` as seen by `StarlarkStructureAdapter` (Java tests use
/// `StructProvider.STRUCT.create`).
#[derive(Debug, Default)]
pub struct TestStruct {
    pub fields: IndexMap<String, VariableObject>,
}

impl TestStruct {
    pub fn object(fields: Vec<(&str, VariableObject)>) -> VariableObject {
        VariableObject::Structure(Arc::new(TestStruct {
            fields: fields
                .into_iter()
                .map(|(k, v)| (k.to_string(), v))
                .collect(),
        }))
    }
}

impl blaze_cc_features::cc_toolchain_variables::Structure for TestStruct {
    fn get_value(
        &self,
        name: &str,
    ) -> Result<Option<VariableObject>, blaze_cc_features::EvalException> {
        Ok(self.fields.get(name).cloned())
    }

    fn starlark_type(&self) -> String {
        "struct".to_string()
    }

    fn truth(&self) -> bool {
        true
    }
}

/// Java's `FakeActionInputFileCache` with tree artifacts.
#[derive(Default)]
pub struct FakeInputMetadataProvider {
    pub trees: Vec<(ArtifactRef, Vec<ArtifactRef>)>,
}

impl InputMetadataProvider for FakeInputMetadataProvider {
    fn get_tree_metadata(&self, artifact: &ArtifactRef) -> Option<Vec<ArtifactRef>> {
        self.trees
            .iter()
            .find(|(t, _)| t == artifact)
            .map(|(_, c)| c.clone())
    }
}

/// A minimal artifact: source or derived, optionally a tree artifact.
#[derive(Debug, Clone)]
pub struct TestArtifact {
    pub exec_path: PathFragment,
    pub source: bool,
    pub tree: bool,
}

impl TestArtifact {
    pub fn source(exec_path: &str) -> ArtifactRef {
        ArtifactRef::new(TestArtifact {
            exec_path: PathFragment::create(exec_path),
            source: true,
            tree: false,
        })
    }

    pub fn derived(exec_path: &str) -> ArtifactRef {
        ArtifactRef::new(TestArtifact {
            exec_path: PathFragment::create(exec_path),
            source: false,
            tree: false,
        })
    }

    pub fn tree(exec_path: &str) -> ArtifactRef {
        ArtifactRef::new(TestArtifact {
            exec_path: PathFragment::create(exec_path),
            source: false,
            tree: true,
        })
    }
}

impl Artifact for TestArtifact {
    fn get_exec_path(&self) -> PathFragment {
        self.exec_path.clone()
    }

    fn is_tree_artifact(&self) -> bool {
        self.tree
    }

    fn is_source_artifact(&self) -> bool {
        self.source
    }

    fn pretty_print(&self) -> String {
        self.exec_path.get_path_string().to_string()
    }

    fn artifact_equals(&self, other: &dyn Artifact) -> bool {
        self.exec_path == other.get_exec_path()
            && self.source == other.is_source_artifact()
            && self.tree == other.is_tree_artifact()
    }

    fn artifact_hash(&self) -> u64 {
        let mut h = std::collections::hash_map::DefaultHasher::new();
        self.exec_path.hash(&mut h);
        h.finish()
    }
}
