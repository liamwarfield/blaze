//! Port of the loading-phase parts of `analysis/starlark/StarlarkSubrule.java` and the
//! `subrule()` builtin (`StarlarkRuleClassFunctions.subrule`). Calling a subrule happens during
//! analysis and is unported.

use std::fmt;
use std::hash::Hash;
use std::sync::Arc;
use std::sync::OnceLock;

use allocative::Allocative;
use blaze_cmdline::Label;
use indexmap::IndexSet;
use starlark::any::ProvidesStaticType;
use starlark::coerce::Coerce;
use starlark::collections::StarlarkHasher;
use starlark::eval::Arguments;
use starlark::eval::Evaluator;
use starlark::starlark_complex_value;
use starlark::values::Freeze;
use starlark::values::FreezeResult;
use starlark::values::Freezer;
use starlark::values::FrozenValue;
use starlark::values::NoSerialize;
use starlark::values::StarlarkValue;
use starlark::values::Trace;
use starlark::values::Value;
use starlark::values::ValueLifetimeless;
use starlark::values::ValueLike;
use starlark::values::starlark_value;

use super::attr_module::DescriptorRef;
use super::attr_module::as_descriptor;
use super::util::errorf;
use super::util::eval_exception;
use crate::attribute::AttributeValueSource;
use crate::bzl_context::BzlInitThreadContext;
use crate::rule_class::ToolchainTypeRequirement;

/// The non-Starlark part of a subrule.
#[derive(Debug)]
pub struct SubruleData {
    toolchains: IndexSet<ToolchainTypeRequirement>,
    fragments: IndexSet<String>,
    /// Java identity of an unexported subrule.
    symbol: u64,
    /// `(extensionLabel, exportedName)` once exported.
    exported: OnceLock<(Label, String)>,
}

/// A subrule (Java `StarlarkSubrule`).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkSubrule.java:74 StarlarkSubrule
#[derive(Debug, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[repr(C)]
pub struct StarlarkSubruleGen<V: ValueLifetimeless> {
    implementation: V,
    /// Java `SubruleAttribute`s: attribute name and its descriptor.
    attributes: Vec<(String, V)>,
    subrules: Vec<V>,
    #[trace(unsafe_ignore)]
    #[allocative(skip)]
    data: Arc<SubruleData>,
}

starlark_complex_value!(pub StarlarkSubrule);

// SAFETY: `repr(C)` and FrozenValue/Value have the same layout.
unsafe impl<'v> Coerce<StarlarkSubruleGen<Value<'v>>> for StarlarkSubruleGen<FrozenValue> {}

impl<'v> Freeze for StarlarkSubrule<'v> {
    type Frozen = FrozenStarlarkSubrule;
    fn freeze(self, freezer: &Freezer) -> FreezeResult<FrozenStarlarkSubrule> {
        Ok(StarlarkSubruleGen {
            implementation: self.implementation.freeze(freezer)?,
            attributes: self
                .attributes
                .into_iter()
                .map(|(k, v)| Ok((k, v.freeze(freezer)?)))
                .collect::<FreezeResult<Vec<_>>>()?,
            subrules: self.subrules.freeze(freezer)?,
            data: self.data,
        })
    }
}

impl<'v, V: ValueLike<'v>> StarlarkSubruleGen<V> {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkSubrule.java:101 StarlarkSubrule.getName
    pub fn get_name(&self) -> String {
        match self.data.exported.get() {
            Some((_, name)) => name.clone(),
            None => "unexported subrule".to_owned(),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkSubrule.java:252 StarlarkSubrule.isExported
    pub fn is_exported(&self) -> bool {
        self.data.exported.get().is_some()
    }

    pub fn data(&self) -> &Arc<SubruleData> {
        &self.data
    }
}

impl<'v, V: ValueLike<'v>> fmt::Display for StarlarkSubruleGen<V> {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkSubrule.java:110 StarlarkSubrule.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<subrule {}>", self.get_name())
    }
}

#[starlark_value(type = "Subrule")]
impl<'v, V: ValueLike<'v>> StarlarkValue<'v> for StarlarkSubruleGen<V>
where
    Self: ProvidesStaticType<'v>,
{
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkSubrule.java:115 StarlarkSubrule.equals
    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        match subrule_data(other) {
            Some(o) => Ok(same_subrule(&self.data, &o)),
            None => Ok(false),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkSubrule.java:126 StarlarkSubrule.hashCode
    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        match self.data.exported.get() {
            Some((l, n)) => {
                l.hash(hasher);
                n.hash(hasher);
            }
            None => self.data.symbol.hash(hasher),
        }
        Ok(())
    }

    fn invoke(
        &self,
        _me: Value<'v>,
        _args: &Arguments<'v, '_>,
        _eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        // W4-analysis: subrules are called during analysis; blaze_analysis installs the call.
        match SUBRULE_CALL_HOOK.get() {
            Some(hook) => hook(_me, _args, _eval),
            None => blaze_util::unported!("StarlarkSubrule.call"),
        }
    }

    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkSubrule.java:258 StarlarkSubrule.export
    fn export_as(
        &self,
        variable_name: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<()> {
        if self.is_exported() {
            return Ok(());
        }
        let Some(ctx) = BzlInitThreadContext::from_or_null(eval) else {
            return Ok(());
        };
        let label = ctx.bzl_file.clone();
        drop(ctx);
        // Java: SubruleAttribute.transformOnExport reports (and drops) attributes whose rule name
        // is invalid; the names are computed on demand here (`attributes_for_rule`).
        let _ = self.data.exported.set((label, variable_name.to_owned()));
        Ok(())
    }
}

fn same_subrule(a: &Arc<SubruleData>, b: &Arc<SubruleData>) -> bool {
    match (a.exported.get(), b.exported.get()) {
        (Some(x), Some(y)) => x == y,
        _ => Arc::ptr_eq(a, b),
    }
}

/// The shared data of a subrule value (frozen or not).
pub fn subrule_data(v: Value) -> Option<Arc<SubruleData>> {
    if let Some(s) = StarlarkSubrule::from_value(v) {
        return Some(s.data.clone());
    }
    v.downcast_ref::<FrozenStarlarkSubrule>()
        .map(|s| s.data.clone())
}

/// The analysis-time implementation of calling a subrule (Java `StarlarkSubrule.call`), installed
/// by blaze_analysis with [`set_subrule_call_hook`].
// W4-analysis: the call needs the rule context, which lives in blaze_analysis.
pub type SubruleCallHook = for<'v> fn(
    Value<'v>,
    &Arguments<'v, '_>,
    &mut Evaluator<'v, '_, '_>,
) -> starlark::Result<Value<'v>>;

static SUBRULE_CALL_HOOK: OnceLock<SubruleCallHook> = OnceLock::new();

/// Installs the analysis implementation of `StarlarkSubrule.call` (first call wins).
// W4-analysis: see SubruleCallHook.
pub fn set_subrule_call_hook(hook: SubruleCallHook) {
    let _ = SUBRULE_CALL_HOOK.set(hook);
}

/// Java `StarlarkSubrule`'s fields as analysis sees them.
// W4-analysis: read by blaze_analysis::starlark::starlark_subrule.
pub struct SubruleView<'v> {
    /// Java `getName()`.
    pub name: String,
    /// `(extensionLabel, exportedName)` once exported.
    pub exported: Option<(Label, String)>,
    pub implementation: Value<'v>,
    /// Java `SubruleAttribute`s: `(attrName, ruleAttrName)`; `ruleAttrName` is `None` while the
    /// subrule is unexported.
    pub attributes: Vec<(String, Option<String>)>,
    pub toolchains: Vec<ToolchainTypeRequirement>,
    pub fragments: IndexSet<String>,
    /// Java `getDeclaredSubrules()`.
    pub subrules: Vec<Value<'v>>,
}

/// The parts of `v` if it is a subrule.
// W4-analysis: read by blaze_analysis::starlark::starlark_subrule.
pub fn subrule_view<'v>(v: Value<'v>) -> Option<SubruleView<'v>> {
    let (data, attrs, subrules) = subrule_parts(v)?;
    let implementation = match StarlarkSubrule::from_value(v) {
        Some(s) => s.implementation,
        None => v
            .downcast_ref::<FrozenStarlarkSubrule>()
            .map(|s| s.implementation.to_value())?,
    };
    let exported = data.exported.get().cloned();
    let attributes = attrs
        .into_iter()
        .map(|(attr_name, descriptor)| {
            let rule_attr_name = match (&exported, as_descriptor(descriptor)) {
                (Some((label, name)), Some(d)) => {
                    get_rule_attr_name(label, name, &attr_name, d.get_value_source()).ok()
                }
                _ => None,
            };
            (attr_name, rule_attr_name)
        })
        .collect();
    let name = match &exported {
        Some((_, n)) => n.clone(),
        None => "unexported subrule".to_owned(),
    };
    Some(SubruleView {
        name,
        exported,
        implementation,
        attributes,
        toolchains: data.toolchains.iter().cloned().collect(),
        fragments: data.fragments.clone(),
        subrules,
    })
}

/// Java `x instanceof StarlarkSubrule`.
pub fn is_subrule(v: Value) -> bool {
    subrule_data(v).is_some()
}

/// Java `subrule.getDeclaredSubrules()`, `attributes` and `toolchains` of a subrule value.
fn subrule_parts<'v>(
    v: Value<'v>,
) -> Option<(Arc<SubruleData>, Vec<(String, Value<'v>)>, Vec<Value<'v>>)> {
    if let Some(s) = StarlarkSubrule::from_value(v) {
        return Some((s.data.clone(), s.attributes.clone(), s.subrules.clone()));
    }
    v.downcast_ref::<FrozenStarlarkSubrule>().map(|s| {
        (
            s.data.clone(),
            s.attributes
                .iter()
                .map(|(k, v)| (k.clone(), v.to_value()))
                .collect(),
            s.subrules.iter().map(|v| v.to_value()).collect(),
        )
    })
}

/// Java `StarlarkSubrule.getTransitiveSubrules`: unique subrules, each followed by its own
/// transitive subrules.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkSubrule.java:293 StarlarkSubrule.getTransitiveSubrules
fn get_transitive_subrules<'v>(subrules: &[Value<'v>]) -> Vec<Value<'v>> {
    fn go<'v>(subrules: &[Value<'v>], out: &mut Vec<(Arc<SubruleData>, Value<'v>)>) {
        for s in subrules {
            if let Some((data, _, declared)) = subrule_parts(*s) {
                if !out.iter().any(|(d, _)| same_subrule(d, &data)) {
                    out.push((data, *s));
                }
                go(&declared, out);
            }
        }
    }
    let mut out = Vec::new();
    go(subrules, &mut out);
    out.into_iter().map(|(_, v)| v).collect()
}

// _foo -> $//pkg:label%my_subrule%_foo
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkSubrule.java:555 StarlarkSubrule.getRuleAttrName
pub fn get_rule_attr_name(
    label: &Label,
    exported_name: &str,
    attr_name: &str,
    value_source: AttributeValueSource,
) -> starlark::Result<String> {
    value_source
        .convert_to_native_name(&format!(
            "_{}%{}%{}",
            label.get_canonical_form(),
            exported_name,
            attr_name
        ))
        .map_err(eval_exception)
}

/// Returns all attributes to be lifted from the given subrules to a rule/aspect; attributes are
/// discovered transitively, and those of common transitive dependencies are de-duped.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkSubrule.java:275 StarlarkSubrule.discoverAttributes
pub fn discover_attributes<'v>(
    subrules: &[Value<'v>],
) -> starlark::Result<Vec<(String, DescriptorRef<'v>)>> {
    let mut attributes = Vec::new();
    for subrule in get_transitive_subrules(subrules) {
        let (data, attrs, _) = subrule_parts(subrule).unwrap();
        // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkSubrule.java:236 StarlarkSubrule.attributesForRule
        let Some((label, name)) = data.exported.get() else {
            return Err(errorf("Invalid subrule hasn't been exported by a bzl file"));
        };
        for (attr_name, descriptor) in attrs {
            let descriptor = as_descriptor(descriptor).expect("subrule attribute descriptor");
            let rule_attr_name =
                get_rule_attr_name(label, name, &attr_name, descriptor.get_value_source())?;
            attributes.push((rule_attr_name, descriptor));
        }
    }
    Ok(attributes)
}

/// The user-defined attribute names of the given subrules' (transitive) attributes, by rule
/// attribute name (Java `StarlarkSubrule.getUserDefinedNameIfSubruleAttr`).
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkSubrule.java:304 StarlarkSubrule.getUserDefinedNameIfSubruleAttr
pub fn subrule_attribute_names(subrules: &[Value]) -> indexmap::IndexMap<String, String> {
    let mut names = indexmap::IndexMap::new();
    for subrule in get_transitive_subrules(subrules) {
        let (data, attrs, _) = subrule_parts(subrule).unwrap();
        let Some((label, name)) = data.exported.get() else {
            continue;
        };
        for (attr_name, descriptor) in attrs {
            let Some(descriptor) = as_descriptor(descriptor) else {
                continue;
            };
            if let Ok(rule_attr_name) =
                get_rule_attr_name(label, name, &attr_name, descriptor.get_value_source())
            {
                names.entry(rule_attr_name).or_insert(attr_name);
            }
        }
    }
    names
}

/// Java `StarlarkSubrule.discoverToolchains`.
// Java: com/google/devtools/build/lib/analysis/starlark/StarlarkSubrule.java:285 StarlarkSubrule.discoverToolchains
pub fn discover_toolchains(subrules: &[Value]) -> IndexSet<ToolchainTypeRequirement> {
    let mut toolchains = IndexSet::new();
    for subrule in get_transitive_subrules(subrules) {
        let (data, _, _) = subrule_parts(subrule).unwrap();
        toolchains.extend(data.toolchains.iter().cloned());
    }
    toolchains
}

/// Whether `a` and `b` are the same subrule (Java `StarlarkSubrule.equals`).
pub fn subrule_equals(a: Value, b: Value) -> bool {
    match (subrule_data(a), subrule_data(b)) {
        (Some(x), Some(y)) => same_subrule(&x, &y),
        _ => false,
    }
}

impl<'v> StarlarkSubrule<'v> {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkSubrule.java:87 StarlarkSubrule.<init>
    pub fn new(
        implementation: Value<'v>,
        attributes: Vec<(String, Value<'v>)>,
        toolchains: IndexSet<ToolchainTypeRequirement>,
        fragments: IndexSet<String>,
        subrules: Vec<Value<'v>>,
        symbol: u64,
    ) -> Self {
        StarlarkSubruleGen {
            implementation,
            attributes,
            subrules,
            data: Arc::new(SubruleData {
                toolchains,
                fragments,
                symbol,
                exported: OnceLock::new(),
            }),
        }
    }
}

impl SubruleData {
    /// Java `fragments`.
    pub fn fragments(&self) -> &IndexSet<String> {
        &self.fragments
    }
}
