//! `repository_rule()`, `module_extension()` and `tag_class()`, and the values they return.
//!
//! Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryModule.java
//! (with `StarlarkRepoRule`), starlarkbuildapi/repository/RepositoryModuleApi.java (parameters),
//! bazel/bzlmod/ModuleExtension.java and TagClass.java (the values).
//!
//! The values are Starlark objects created during .bzl evaluation; the immutable Rust data
//! ([`RepoRule`], [`ModuleExtension`], [`TagClass`]) is read from their frozen, exported form by
//! [`repo_rule_from_global`], [`module_extension_from_global`] and [`tag_class_of`].

use std::fmt;
use std::sync::Arc;
use std::sync::OnceLock;

use allocative::Allocative;
use anyhow::anyhow;
use blaze_bzlmod_model::RepoRuleId;
use blaze_cmdline::Label;
use blaze_cmdline::RepositoryName;
use blaze_packages::bzl_context::BazelModuleContext;
use blaze_packages::bzl_context::BzlInitThreadContext;
use blaze_packages::starlark_defs::attr_module::as_descriptor;
use blaze_packages::starlark_defs::util::trim_doc_string;
use blaze_skyframe::Environment;
use blaze_skyframe::Interrupted;
use blaze_starlark::Location;
use blaze_starlark::env::caller_location;
use indexmap::IndexMap;
use indexmap::IndexSet;
use starlark::any::ProvidesStaticType;
use starlark::coerce::Coerce;
use starlark::environment::GlobalsBuilder;
use starlark::eval::Arguments;
use starlark::eval::Evaluator;
use starlark::starlark_complex_value;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::Freeze;
use starlark::values::FreezeResult;
use starlark::values::Freezer;
use starlark::values::FrozenValue;
use starlark::values::NoSerialize;
use starlark::values::OwnedFrozenValue;
use starlark::values::StarlarkValue;
use starlark::values::Trace;
use starlark::values::Value;
use starlark::values::ValueLifetimeless;
use starlark::values::ValueLike;
use starlark::values::dict::DictRef;
use starlark::values::list::ListRef;
use starlark::values::none::NoneOr;
use starlark::values::starlark_value;
use starlark::values::tuple::TupleRef;

use crate::bzlmod::module_extension::ModuleExtension;
use crate::bzlmod::tag_class::TagClass;
use crate::repository::repo_rule::RepoRule;
use crate::repository::repo_rule::RepoRuleBuilder;
use crate::starlark::context::ExternalContext;

fn eval_error(msg: impl fmt::Display) -> starlark::Error {
    starlark::Error::new_other(anyhow!("{msg}"))
}

/// `Sequence.cast(x, String.class, what)`.
// Java: net/starlark/java/eval/Sequence.java:113 Sequence.cast
fn cast_string_sequence(x: Value, what: &str) -> starlark::Result<Vec<String>> {
    let items: Vec<Value> = if let Some(l) = ListRef::from_value(x) {
        l.iter().collect()
    } else if let Some(t) = TupleRef::from_value(x) {
        t.iter().collect()
    } else {
        return Err(eval_error(format!(
            "got value of type '{}', want 'sequence of strings'",
            x.get_type()
        )));
    };
    let mut out = Vec::with_capacity(items.len());
    for (i, item) in items.into_iter().enumerate() {
        match item.unpack_str() {
            Some(s) => out.push(s.to_owned()),
            None => {
                return Err(eval_error(format!(
                    "at index {i} of {what}, got element of type {}, want string",
                    item.get_type()
                )));
            }
        }
    }
    Ok(out)
}

/// Builds the attributes of a `repository_rule` or `tag_class` from its `attrs` dict
/// (`Dict.cast(attrs, String.class, Descriptor.class, "attrs")` then
/// `source.convertToNativeName` and `descriptor.build(name)`), calling `add` for each.
fn for_each_attr_descriptor(
    attrs: Value,
    mut add: impl FnMut(String, blaze_packages::attribute::Attribute) -> starlark::Result<()>,
) -> starlark::Result<()> {
    let Some(dict) = DictRef::from_value(attrs) else {
        return Err(eval_error(format!(
            "got value of type '{}', want 'dict'",
            attrs.get_type()
        )));
    };
    for (k, v) in dict.iter() {
        let Some(key) = k.unpack_str() else {
            return Err(eval_error(format!(
                "got dict<{}, {}> for 'attrs', want dict<string, Attribute>",
                k.get_type(),
                v.get_type()
            )));
        };
        let Some(descriptor) = as_descriptor(v) else {
            return Err(eval_error(format!(
                "got dict<{}, {}> for 'attrs', want dict<string, Attribute>",
                k.get_type(),
                v.get_type()
            )));
        };
        let source = descriptor.get_value_source();
        let attr_name = source
            .convert_to_native_name(key)
            .map_err(|e| eval_error(e.get_message()))?;
        let attribute = descriptor.build(&attr_name);
        add(attr_name, attribute)?;
    }
    Ok(())
}

// ---------------------------------------------------------------------------------------------
// repository_rule

/// The data of a `repository_rule` value fixed at creation (Java `RepoRule.Builder`).
struct RepoRuleData {
    bzl_file_label: Label,
    transitive_bzl_digest: Vec<u8>,
    doc: Option<String>,
    builder: RepoRuleBuilder,
    local: bool,
    configure: bool,
    remotable: bool,
    environ: IndexSet<String>,
    module_context: Option<Arc<BazelModuleContext>>,
    /// Java `builder.recordedRepoMappingEntries(repoMappingRecorder.recordedEntries())`, taken
    /// when `repository_rule()` is called; `None` when the thread had no recorder (the builder's
    /// default, an empty table).
    recorded_repo_mapping_entries: Option<Vec<(RepositoryName, String, RepositoryName)>>,
    /// Set on export (Java `builder.idBuilder().ruleName(exportedName)`).
    rule_name: OnceLock<String>,
    /// Populated on first use after export (Java `StarlarkRepoRule.repoRule`).
    repo_rule: OnceLock<Arc<RepoRule>>,
}

/// The value returned by calling the `repository_rule` function in Starlark. It itself is a
/// callable value; calling it defines a repo.
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryModule.java:122 StarlarkRepositoryModule.StarlarkRepoRule
#[derive(Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[repr(C)]
pub struct StarlarkRepoRuleGen<V: ValueLifetimeless> {
    implementation: V,
    #[trace(unsafe_ignore)]
    #[allocative(skip)]
    data: Arc<RepoRuleData>,
}

starlark_complex_value!(pub StarlarkRepoRule);

// SAFETY: `repr(C)`, and `FrozenValue`/`Value` have the same layout.
unsafe impl<'v> Coerce<StarlarkRepoRuleGen<Value<'v>>> for StarlarkRepoRuleGen<FrozenValue> {}

impl<'v> Freeze for StarlarkRepoRule<'v> {
    type Frozen = FrozenStarlarkRepoRule;
    fn freeze(self, freezer: &Freezer) -> FreezeResult<FrozenStarlarkRepoRule> {
        Ok(StarlarkRepoRuleGen {
            implementation: self.implementation.freeze(freezer)?,
            data: self.data,
        })
    }
}

impl<V: ValueLifetimeless> fmt::Debug for StarlarkRepoRuleGen<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

impl<V: ValueLifetimeless> StarlarkRepoRuleGen<V> {
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryModule.java:153 StarlarkRepoRule.isExported
    fn is_exported(&self) -> bool {
        self.data.rule_name.get().is_some()
    }

    fn id(&self) -> Option<RepoRuleId> {
        self.data
            .rule_name
            .get()
            .map(|name| RepoRuleId::new(self.data.bzl_file_label.clone(), name.clone()))
    }
}

impl<V: ValueLifetimeless> fmt::Display for StarlarkRepoRuleGen<V> {
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryModule.java:158 StarlarkRepoRule.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.id() {
            None => write!(f, "<anonymous starlark repository rule>"),
            Some(id) => write!(f, "<starlark repository rule {id}>"),
        }
    }
}

/// Marks the frozen form of a complex value (exporting never happens to frozen values).
pub trait FrozenMarker {
    const FROZEN: bool;
}

impl FrozenMarker for Value<'_> {
    const FROZEN: bool = false;
}

impl FrozenMarker for FrozenValue {
    const FROZEN: bool = true;
}

// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryModule.java:128 StarlarkRepositoryModule.StarlarkRepoRule.getName
// PORT: skipped StarlarkRepositoryModule.StarlarkRepoRule.isImmutable: a frozen value is immutable.
#[starlark_value(type = "repository_rule")]
impl<'v, V: ValueLike<'v> + FrozenMarker> StarlarkValue<'v> for StarlarkRepoRuleGen<V>
where
    Self: ProvidesStaticType<'v>,
{
    /// The description `starlark_doc_extract` reads (Java `ModuleInfoExtractor` takes
    /// `starlarkRepoRule.getRepoRule()`).
    fn provide(&'v self, demand: &mut starlark::values::Demand<'_, 'v>) {
        let d = &self.data;
        demand.provide_value(
            blaze_packages::starlark_defs::documentable::DocumentableRepoRule {
                exported: self.is_exported(),
                doc: d.doc.clone(),
                rule_name: d.rule_name.get().cloned(),
                bzl_file_label: d.bzl_file_label.clone(),
                attributes: d.builder.attributes.clone(),
                environ: d.environ.iter().cloned().collect(),
            },
        );
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryModule.java:167 StarlarkRepoRule.call
    fn invoke(
        &self,
        _me: Value<'v>,
        args: &Arguments<'v, '_>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        if args.len()? > 0 && !args.positions(eval.heap())?.collect::<Vec<_>>().is_empty() {
            return Err(eval_error("unexpected positional arguments"));
        }
        let kwargs: Vec<(String, Value<'v>)> = args
            .names_map()?
            .iter()
            .map(|(k, v)| (k.as_str().to_owned(), *v))
            .collect();
        let Some(mut ctx) = ExternalContext::from_eval_or_none(eval) else {
            return Err(eval_error(
                "repo rules can only be called from within module extension impl functions",
            ));
        };
        if ctx.extension_thread_context().is_none() {
            return Err(eval_error(
                "repo rules can only be called from within module extension impl functions",
            ));
        }
        if !self.is_exported() {
            return Err(eval_error(
                "attempting to instantiate a non-exported repository rule",
            ));
        }
        let repo_rule = {
            let env = ctx.base.env.get();
            let implementation = self.implementation.to_value();
            let Some(frozen) = implementation.unpack_frozen() else {
                // PORT: a repo rule is only callable after its .bzl was frozen (extensions run
                // after loading); Java has no such state.
                return Err(eval_error(
                    "internal error: repository rule called before its .bzl file was frozen",
                ));
            };
            match get_repo_rule(&self.data, frozen, env) {
                Ok(Some(r)) => r,
                Ok(None) => return Err(ctx.needs_restart()),
                Err(i) => return Err(ctx.interrupted(i)),
            }
        };
        let thread_context = ctx.extension_thread_context().expect("checked above");
        thread_context
            .lazily_create_repo(eval, kwargs, repo_rule)
            .map_err(eval_error)?;
        Ok(Value::new_none())
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryModule.java:144 StarlarkRepoRule.export
    fn export_as(
        &self,
        variable_name: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<()> {
        if V::FROZEN || self.is_exported() {
            return Ok(());
        }
        if BzlInitThreadContext::from_or_null(eval).is_none() {
            return Ok(());
        }
        let _ = self.data.rule_name.set(variable_name.to_owned());
        Ok(())
    }
}

/// Builds the [`RepoRule`] of a frozen `repository_rule` value (Java `getRepoRule`), once.
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryModule.java:180 StarlarkRepositoryModule.StarlarkRepoRule.getRepoRule
fn get_repo_rule(
    data: &Arc<RepoRuleData>,
    implementation: FrozenValue,
    env: &mut Environment<'_>,
) -> Result<Option<Arc<RepoRule>>, Interrupted> {
    if let Some(r) = data.repo_rule.get() {
        return Ok(Some(r.clone()));
    }
    let rule_name = data
        .rule_name
        .get()
        .expect("only exported repository rules are built")
        .clone();
    let bootstrap = blaze_bzlmod_model::module::BOOTSTRAP_REPO_RULES
        .iter()
        .any(|id| id.bzl_file_label() == Some(&data.bzl_file_label) && id.rule_name() == rule_name);
    let Some(defining_bzl) =
        crate::bzl_load::load_bzl_for_bzlmod(env, &data.bzl_file_label, bootstrap)?
    else {
        return Ok(None);
    };
    let defining_bzl = match defining_bzl {
        Ok(v) => v,
        // The .bzl defining the rule was loaded already (the rule value exists).
        Err(e) => panic!("internal error: reloading {}: {e}", data.bzl_file_label),
    };
    // The defining module was loaded above, so its heap is in the build arena's keep-alive.
    let _ = defining_bzl;
    // PORT: keep-alive. The handle does not own the defining module's heap: the `RepoRule` is
    // cached in `data`, which the exported `repository_rule` value on that same heap holds, so an
    // owning handle would make the heap own itself and it would never be freed (with the
    // persistent server's release of the build arena, every module defining a repository rule
    // and everything it references stayed alive after every command). The heap is kept by
    // BZL_LOAD in the build arena (`blaze_util::build_arena`, decision D7), which outlives every
    // holder of the `RepoRule`: the repository functions' values in the command's evaluators
    // (each holds a lease) and the exported value itself.
    // SAFETY: `implementation` is reachable from the defining module's frozen heap (it is a
    // field of the exported `repository_rule` value, frozen with that module), which the build
    // arena keeps alive for as long as the `RepoRule` can be reached (see above).
    let impl_ = unsafe {
        OwnedFrozenValue::new(starlark::values::FrozenHeapRef::default(), implementation)
    };
    let repo_rule = Arc::new(RepoRule {
        id: RepoRuleId::new(data.bzl_file_label.clone(), rule_name),
        transitive_bzl_digest: data.transitive_bzl_digest.clone(),
        recorded_repo_mapping_entries: data
            .recorded_repo_mapping_entries
            .clone()
            .unwrap_or_default(),
        impl_,
        impl_stamp: blaze_util::build_arena::ArenaStamp::current(),
        doc: data.doc.clone(),
        attributes: data.builder.attributes.clone(),
        attribute_indices: data.builder.attribute_indices.clone(),
        local: data.local,
        configure: data.configure,
        remotable: data.remotable,
        environ: data.environ.clone(),
        module_context: data.module_context.clone(),
    });
    let _ = data.repo_rule.set(repo_rule);
    Ok(Some(data.repo_rule.get().expect("just set").clone()))
}

/// Java `object instanceof RepoRule.Supplier s ? s.getRepoRule() : ...`: the [`RepoRule`] of a
/// module global, `Ok(Some(None))` if the value is not a repository rule, `Ok(None)` if a
/// Skyframe value is missing.
pub fn repo_rule_from_global(
    value: &OwnedFrozenValue,
    env: &mut Environment<'_>,
) -> Result<Option<Option<Arc<RepoRule>>>, Interrupted> {
    let Some(rule) = value.value().downcast_ref::<FrozenStarlarkRepoRule>() else {
        return Ok(Some(None));
    };
    if !rule.is_exported() {
        return Ok(Some(None));
    }
    Ok(get_repo_rule(&rule.data, rule.implementation, env)?.map(Some))
}

// ---------------------------------------------------------------------------------------------
// module_extension

/// The value returned by `module_extension()`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/ModuleExtension.java:30 ModuleExtension
#[derive(Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[repr(C)]
pub struct ModuleExtensionValueGen<V: ValueLifetimeless> {
    implementation: V,
    #[trace(unsafe_ignore)]
    #[allocative(skip)]
    data: Arc<ModuleExtensionData>,
}

struct ModuleExtensionData {
    tag_classes: IndexMap<String, Arc<TagClass>>,
    doc: Option<String>,
    defining_bzl_file_label: Label,
    location: Location,
    env_variables: Vec<String>,
    os_dependent: bool,
    arch_dependent: bool,
    facts_version: i32,
    module_context: Option<Arc<BazelModuleContext>>,
}

starlark_complex_value!(pub ModuleExtensionValue);

// SAFETY: `repr(C)`, and `FrozenValue`/`Value` have the same layout.
unsafe impl<'v> Coerce<ModuleExtensionValueGen<Value<'v>>>
    for ModuleExtensionValueGen<FrozenValue>
{
}

impl<'v> Freeze for ModuleExtensionValue<'v> {
    type Frozen = FrozenModuleExtensionValue;
    fn freeze(self, freezer: &Freezer) -> FreezeResult<FrozenModuleExtensionValue> {
        Ok(ModuleExtensionValueGen {
            implementation: self.implementation.freeze(freezer)?,
            data: self.data,
        })
    }
}

impl<V: ValueLifetimeless> fmt::Debug for ModuleExtensionValueGen<V> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

impl<V: ValueLifetimeless> fmt::Display for ModuleExtensionValueGen<V> {
    // PORT: Java ModuleExtension is an AutoBuilder record with the default StarlarkValue repr
    // (`<module_extension object>`).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<module_extension object>")
    }
}

#[starlark_value(type = "module_extension")]
impl<'v, V: ValueLike<'v>> StarlarkValue<'v> for ModuleExtensionValueGen<V>
where
    Self: ProvidesStaticType<'v>,
{
    /// The description `starlark_doc_extract` reads (Java `ModuleInfoExtractor` takes the
    /// `ModuleExtension` itself).
    fn provide(&'v self, demand: &mut starlark::values::Demand<'_, 'v>) {
        let d = &self.data;
        demand.provide_value(
            blaze_packages::starlark_defs::documentable::DocumentableModuleExtension {
                doc: d.doc.clone(),
                defining_bzl_file_label: d.defining_bzl_file_label.clone(),
                tag_classes: d
                    .tag_classes
                    .iter()
                    .map(|(name, tag_class)| {
                        blaze_packages::starlark_defs::documentable::DocumentableTagClass {
                            name: name.clone(),
                            doc: tag_class.doc.clone(),
                            attributes: tag_class.attributes.clone(),
                        }
                    })
                    .collect(),
            },
        );
    }
}

/// The [`ModuleExtension`] of a module global, or `None` if it is not a module extension
/// (Java `getGlobal(name) instanceof ModuleExtension`).
pub fn module_extension_from_global(value: &OwnedFrozenValue) -> Option<Arc<ModuleExtension>> {
    let ext = value.value().downcast_ref::<FrozenModuleExtensionValue>()?;
    let implementation = value.map(|_| ext.implementation);
    let d = &ext.data;
    Some(Arc::new(ModuleExtension {
        implementation,
        tag_classes: d.tag_classes.clone(),
        doc: d.doc.clone(),
        defining_bzl_file_label: d.defining_bzl_file_label.clone(),
        location: d.location.clone(),
        env_variables: d.env_variables.clone(),
        os_dependent: d.os_dependent,
        arch_dependent: d.arch_dependent,
        facts_version: d.facts_version,
        module_context: d.module_context.clone(),
    }))
}

// ---------------------------------------------------------------------------------------------
// tag_class

/// The value returned by `tag_class()`.
// Java: com/google/devtools/build/lib/bazel/bzlmod/TagClass.java:28 TagClass
#[derive(ProvidesStaticType, NoSerialize, Allocative)]
pub struct TagClassValue(#[allocative(skip)] pub Arc<TagClass>);

starlark_simple_value!(TagClassValue);

impl fmt::Debug for TagClassValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{self}")
    }
}

impl fmt::Display for TagClassValue {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<tag_class object>")
    }
}

#[starlark_value(type = "tag_class")]
impl<'v> StarlarkValue<'v> for TagClassValue {}

/// The [`TagClass`] of a `tag_class` value.
pub fn tag_class_of(value: Value) -> Option<Arc<TagClass>> {
    value.downcast_ref::<TagClassValue>().map(|t| t.0.clone())
}

// ---------------------------------------------------------------------------------------------
// The globals

/// Java `StarlarkRepositoryModule`'s top-level functions (`RepositoryModuleApi`).
// Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryModule.java:47 StarlarkRepositoryModule
#[starlark_module]
pub fn register_repository_module(builder: &mut GlobalsBuilder) {
    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryModule.java:49 StarlarkRepositoryModule.repositoryRule
    fn repository_rule<'v>(
        implementation: Value<'v>,
        #[starlark(default = NoneOr::None)] attrs: NoneOr<Value<'v>>,
        #[starlark(require = named, default = false)] local: bool,
        #[starlark(require = named)] environ: Option<Value<'v>>,
        #[starlark(require = named, default = false)] configure: bool,
        #[starlark(require = named, default = false)] remotable: bool,
        #[starlark(require = named, default = NoneOr::None)] doc: NoneOr<&str>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        let environ = match environ {
            None => Vec::new(),
            Some(v) => cast_string_sequence(v, "repository_rule")?,
        };
        let mut builder = RepoRuleBuilder::default();
        let semantics = blaze_starlark::ThreadContext::get(eval)?.semantics.clone();
        if semantics.experimental_repo_remote_exec {
            // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryModule.java:77 StarlarkRepositoryModule.repositoryRule
            builder.add_attribute(
                blaze_packages::attribute::attr(
                    "exec_properties",
                    blaze_packages::types::Type::StringDict,
                )
                .value(Some(blaze_packages::types::AttrValue::StringDict(
                    IndexMap::new(),
                )))
                .build(),
            );
        }
        if let NoneOr::Other(attrs) = attrs {
            let mut error = None;
            for_each_attr_descriptor(attrs, |attr_name, attribute| {
                if builder.has_attribute(&attr_name) {
                    error = Some(eval_error(format!(
                        "There is already a built-in attribute '{attr_name}' which cannot be \
                         overridden"
                    )));
                    return Err(error.take().unwrap());
                }
                builder.add_attribute(attribute);
                Ok(())
            })?;
        }
        let (bzl_file_label, transitive_bzl_digest) = {
            let ctx = BzlInitThreadContext::from_or_fail(eval, "repository_rule")
                .map_err(|e| eval_error(e.get_message()))?;
            (ctx.bzl_file.clone(), ctx.transitive_digest.clone())
        };
        let module_context = BazelModuleContext::of_filename_label(&bzl_file_label, eval);
        let recorded_repo_mapping_entries =
            BzlInitThreadContext::recorded_repo_mapping_entries(eval);
        let data = RepoRuleData {
            bzl_file_label,
            transitive_bzl_digest,
            doc: doc.into_option().map(trim_doc_string),
            builder,
            local,
            configure,
            remotable,
            environ: environ.into_iter().collect(),
            module_context,
            recorded_repo_mapping_entries,
            rule_name: OnceLock::new(),
            repo_rule: OnceLock::new(),
        };
        Ok(eval.heap().alloc(StarlarkRepoRule {
            implementation,
            data: Arc::new(data),
        }))
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryModule.java:216 StarlarkRepositoryModule.moduleExtension
    fn module_extension<'v>(
        implementation: Value<'v>,
        #[starlark(require = named)] tag_classes: Option<Value<'v>>,
        #[starlark(require = named, default = NoneOr::None)] doc: NoneOr<&str>,
        #[starlark(require = named)] environ: Option<Value<'v>>,
        #[starlark(require = named, default = false)] os_dependent: bool,
        #[starlark(require = named, default = false)] arch_dependent: bool,
        #[starlark(require = named, default = 0)] facts_version: i32,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        let facts_version_int = facts_version;
        if facts_version_int < 0 {
            return Err(eval_error(format!(
                "facts_version must be non-negative, got {facts_version_int}"
            )));
        }
        let mut tag_class_map = IndexMap::new();
        if let Some(tag_classes) = tag_classes {
            let Some(dict) = DictRef::from_value(tag_classes) else {
                return Err(eval_error(format!(
                    "got value of type '{}', want 'dict'",
                    tag_classes.get_type()
                )));
            };
            for (k, v) in dict.iter() {
                match (k.unpack_str(), tag_class_of(v)) {
                    (Some(k), Some(tc)) => {
                        tag_class_map.insert(k.to_owned(), tc);
                    }
                    _ => {
                        return Err(eval_error(format!(
                            "got dict<{}, {}> for 'tag_classes', want dict<string, tag_class>",
                            k.get_type(),
                            v.get_type()
                        )));
                    }
                }
            }
        }
        let env_variables = match environ {
            None => Vec::new(),
            Some(v) => cast_string_sequence(v, "environ")?,
        };
        let defining_bzl_file_label =
            BzlInitThreadContext::from_or_fail(eval, "module_extension()")
                .map_err(|e| eval_error(e.get_message()))?
                .bzl_file
                .clone();
        let module_context = BazelModuleContext::of_filename_label(&defining_bzl_file_label, eval);
        let location = caller_location(eval).unwrap_or_else(Location::builtin);
        Ok(eval.heap().alloc(ModuleExtensionValue {
            implementation,
            data: Arc::new(ModuleExtensionData {
                tag_classes: tag_class_map,
                doc: doc.into_option().map(trim_doc_string),
                defining_bzl_file_label,
                location,
                env_variables,
                os_dependent,
                arch_dependent,
                facts_version: facts_version_int,
                module_context,
            }),
        }))
    }

    // Java: com/google/devtools/build/lib/bazel/repository/starlark/StarlarkRepositoryModule.java:240 StarlarkRepositoryModule.tagClass
    fn tag_class<'v>(
        #[starlark(default = NoneOr::None)] attrs: NoneOr<Value<'v>>,
        #[starlark(require = named, default = NoneOr::None)] doc: NoneOr<&str>,
    ) -> starlark::Result<TagClassValue> {
        let mut attributes = Vec::new();
        if let NoneOr::Other(attrs) = attrs {
            for_each_attr_descriptor(attrs, |_, attribute| {
                attributes.push(attribute);
                // TODO(wyv): validate attributes. No selects, no latebound defaults, or any crazy
                //   stuff like that.
                Ok(())
            })?;
        }
        Ok(TagClassValue(Arc::new(TagClass::create(
            attributes,
            doc.into_option().map(trim_doc_string),
        ))))
    }
}

/// The `BazelModuleContext` of the .bzl being initialized.
trait OfFilenameLabel {
    fn of_filename_label(label: &Label, eval: &Evaluator) -> Option<Arc<BazelModuleContext>>;
}

impl OfFilenameLabel for BazelModuleContext {
    fn of_filename_label(label: &Label, eval: &Evaluator) -> Option<Arc<BazelModuleContext>> {
        // The innermost .bzl on the stack is the one being initialized when the builtin is
        // called from its top level.
        BazelModuleContext::of_innermost_bzl(eval).filter(|c| c.label() == label)
    }
}

#[allow(dead_code)]
fn _unused(_: RepositoryName) {}
