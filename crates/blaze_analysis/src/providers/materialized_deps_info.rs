//! Port of `analysis/MaterializedDepsInfo.java`.

use std::fmt;

use allocative::Allocative;
use blaze_cmdline::Label;
use blaze_packages::starlark_defs::builtin_providers::{BuiltinProviderId, MATERIALIZED_DEPS_INFO};
use blaze_starlark::structs::DeeplyImmutable;
use starlark::any::ProvidesStaticType;
use starlark::eval::{Arguments, Evaluator};
use starlark::starlark_simple_value;
use starlark::values::{
    Demand, Heap, NoSerialize, StarlarkValue, Value, ValueLike, starlark_value,
};

use crate::configuredtargets::target_value::StarlarkTarget;
use crate::dormant_dependency::DormantDependency;
use crate::info::InfoProviderKey;
use crate::providers::default_info::named_args;

fn errorf(msg: String) -> starlark::Error {
    starlark::Error::new_other(anyhow::anyhow!("{msg}"))
}

/// One dependency to be materialized: Java `Either<ConfiguredTarget, DormantDependency>`.
///
/// PORT: the configured target is kept by its label (`ConfiguredTarget.getLabel()`), the only
/// thing its reader (`DependencyProducer.evaluateMaterializerTargets`) uses.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum MaterializedDep {
    ConfiguredTarget(Label),
    Dormant(DormantDependency),
}

impl MaterializedDep {
    /// `dependency.map(ConfiguredTarget::getLabel, DormantDependency::getLabel)`.
    pub fn label(&self) -> &Label {
        match self {
            MaterializedDep::ConfiguredTarget(l) => l,
            MaterializedDep::Dormant(d) => d.get_label(),
        }
    }
}

/// The provider returned from materializer rules to materialize dependencies.
// Java: com/google/devtools/build/lib/analysis/MaterializedDepsInfo.java:29 MaterializedDepsInfo
#[derive(Clone, Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct MaterializedDepsInfo {
    /// The dependencies to be materialized.
    #[allocative(skip)]
    deps: Vec<MaterializedDep>,
}

starlark_simple_value!(MaterializedDepsInfo);

impl MaterializedDepsInfo {
    /// The dependencies to be materialized. These may be ConfiguredTarget or DormantDependency
    /// objects.
    // Java: com/google/devtools/build/lib/analysis/MaterializedDepsInfo.java:48 MaterializedDepsInfo.getDeps
    pub fn get_deps(&self) -> &[MaterializedDep] {
        &self.deps
    }

    /// The provider of a Starlark value, if it is one.
    pub fn of(v: Value<'_>) -> Option<&MaterializedDepsInfo> {
        v.downcast_ref::<MaterializedDepsInfo>()
    }
}

impl fmt::Display for MaterializedDepsInfo {
    // Java: com/google/devtools/build/lib/packages/NativeInfo.java NativeInfo.repr: the `deps`
    // field's value is a list of Java `Either` objects.
    fn fmt(&self, _f: &mut fmt::Formatter<'_>) -> fmt::Result {
        blaze_util::unported!("MaterializedDepsInfo.repr (a list of Either objects)")
    }
}

// Java: com/google/devtools/build/lib/starlarkbuildapi/MaterializedDepsInfoApi.java:34 MaterializedDepsInfoApi
#[starlark_value(type = "MaterializedDepsInfo")]
impl<'v> StarlarkValue<'v> for MaterializedDepsInfo {
    // Java: com/google/devtools/build/lib/starlarkbuildapi/MaterializedDepsInfoApi.java:43 MaterializedDepsInfoApi.getDeps
    fn get_attr(&self, attribute: &str, _heap: Heap<'v>) -> Option<Value<'v>> {
        match attribute {
            // Starlark.fromJava of an ImmutableList of Java `Either` objects.
            "deps" => blaze_util::unported!("MaterializedDepsInfo.getDeps from Starlark"),
            _ => None,
        }
    }

    fn has_attr(&self, attribute: &str, _heap: Heap<'v>) -> bool {
        attribute == "deps"
    }

    fn dir_attr(&self) -> Vec<String> {
        vec!["deps".to_owned()]
    }

    // Java: com/google/devtools/build/lib/packages/NativeInfo.java:97 NativeInfo.equals
    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        crate::info::native_info_equals(self, &MATERIALIZED_DEPS_INFO.key(), other)
    }

    // Java: com/google/devtools/build/lib/packages/NativeInfo.java:115 NativeInfo.hashCode
    fn write_hash(&self, hasher: &mut starlark_map::StarlarkHasher) -> starlark::Result<()> {
        crate::info::native_info_write_hash(self, &MATERIALIZED_DEPS_INFO.key(), hasher)
    }

    // Java: com/google/devtools/build/lib/packages/NativeInfo.java:125 NativeInfo.repr
    fn collect_repr(&self, collector: &mut String) {
        crate::info::native_info_collect_repr(self, collector)
    }

    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(InfoProviderKey(MATERIALIZED_DEPS_INFO.key()));
        demand.provide_value(DeeplyImmutable);
    }
}

/// The `MaterializedDepsInfo(deps)` constructor.
// Java: com/google/devtools/build/lib/analysis/MaterializedDepsInfo.java:66 MaterializedDepsInfo.Provider.materializedDepsInfo
pub fn materialized_deps_info_constructor<'v>(
    _id: &BuiltinProviderId,
    _me: Value<'v>,
    args: &Arguments<'v, '_>,
    eval: &mut Evaluator<'v, '_, '_>,
) -> starlark::Result<Value<'v>> {
    let [dependencies] = named_args("MaterializedDepsInfo", ["deps"], 1, args, eval)?;
    let Some(dependencies) = dependencies else {
        return Err(errorf(
            "MaterializedDepsInfo() missing 1 required positional argument: deps".to_owned(),
        ));
    };
    // @ParamType(type = Sequence.class)
    let items: Vec<Value<'v>> =
        if let Some(l) = starlark::values::list::ListRef::from_value(dependencies) {
            l.iter().collect()
        } else if let Some(t) = starlark::values::tuple::TupleRef::from_value(dependencies) {
            t.iter().collect()
        } else {
            return Err(errorf(format!(
                "in call to MaterializedDepsInfo(), parameter 'deps' got value of type '{}', want \
             'sequence'",
                dependencies.get_type()
            )));
        };
    let mut deps = Vec::with_capacity(items.len());
    for (index, dependency) in items.into_iter().enumerate() {
        // Note that materializer rules can depend only on dependency_resolution_rule or dormant
        // dependencies, so the ConfiguredTargets here should be dependency_resolution_rules.
        if let Some(configured_target) = StarlarkTarget::configured_target_of(dependency) {
            deps.push(MaterializedDep::ConfiguredTarget(
                configured_target.get_label(),
            ));
        } else if let Some(dormant) = DormantDependency::of(dependency) {
            deps.push(MaterializedDep::Dormant(dormant.clone()));
        } else {
            return Err(errorf(format!(
                "MaterializedDepsInfo dependencies must be Target objects (retrieved from \
                 ctx.attr) or DormantDependency objects (from attr.dormant_label() or \
                 attr.dormant_label_list() attributes), but got {} at index {index}",
                dependency.get_type()
            )));
        }
    }
    Ok(eval.heap().alloc(MaterializedDepsInfo { deps }))
}
