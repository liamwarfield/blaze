//! Spike: Bazel-like values and per-phase context on starlark-rust 0.14.2.
//!
//! Not a port. These are the smallest shapes that prove the integration points Blaze needs:
//! - `Label`: a simple (immutable, `Send + Sync`) value with attributes and methods.
//! - `Depset`: a complex value holding other values, frozen with its module.
//! - `Provider` / `ProviderInstance`: a callable value named by `export_as` on assignment,
//!   whose identity is (defining .bzl, exported name), as Bazel's `StarlarkProvider.Key`.
//! - `Ctx`: an analysis-time value that is never frozen (`alloc_complex_no_freeze`).
//! - `EvalContext`: per-evaluation Rust state in `Evaluator::extra` (Bazel's StarlarkThread
//!   thread-locals: BazelStarlarkContext / Package.Builder / RuleContext).
//! - `MapLoader`: a `FileLoader` over already-frozen modules (Bazel resolves every load()
//!   before executing a file, so a lookup table is enough).

use std::cell::RefCell;
use std::collections::HashMap;
use std::fmt;

use allocative::Allocative;
use starlark::any::ProvidesStaticType;
use starlark::environment::FrozenModule;
use starlark::environment::Globals;
use starlark::environment::GlobalsBuilder;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::environment::Module;
use starlark::eval::Arguments;
use starlark::eval::Evaluator;
use starlark::eval::FileLoader;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::syntax::AstModule;
use starlark::values::AllocValue;
use starlark::values::Freeze;
use starlark::values::FreezeResult;
use starlark::values::Freezer;
use starlark::values::FrozenValue;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::StarlarkValue;
use starlark::values::Trace;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::list::AllocList;
use starlark::values::list::ListRef;
use starlark::values::list_or_tuple::UnpackListOrTuple;
use starlark::values::none::NoneOr;
use starlark::values::none::NoneType;
use starlark::values::starlark_value;
use starlark::values::tuple::UnpackTuple;
use starlark_map::small_set::SmallSet;

use crate::dialect;
use crate::traceback;

// ---------------------------------------------------------------------------------------------
// Per-evaluation context (Evaluator::extra)

/// Which Bazel phase an evaluation belongs to. Natives check it the way Bazel checks for the
/// presence of a thread-local (e.g. `Package.Builder` only exists while loading a BUILD file).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// .bzl top-level evaluation (BzlLoadFunction).
    BzlLoad,
    /// BUILD file evaluation (PackageFunction).
    Loading,
    /// Rule implementation function (RuleConfiguredTargetBuilder).
    Analysis,
}

/// Rust-side state of one evaluation, reached by natives through `Evaluator::extra`.
#[derive(ProvidesStaticType)]
pub struct EvalContext {
    pub phase: Phase,
    /// Label of the file being evaluated (provider keys use it).
    pub file_label: String,
    /// `print()` output, already in Bazel's "DEBUG: file:line:col: msg" format.
    pub prints: RefCell<Vec<String>>,
    /// Targets declared by `rule_target()` while loading a package.
    pub targets: RefCell<Vec<(String, Vec<String>)>>,
    /// Actions registered by `ctx.run()` during analysis.
    pub actions: RefCell<Vec<String>>,
}

impl EvalContext {
    pub fn new(phase: Phase, file_label: &str) -> EvalContext {
        EvalContext {
            phase,
            file_label: file_label.to_owned(),
            prints: RefCell::new(Vec::new()),
            targets: RefCell::new(Vec::new()),
            actions: RefCell::new(Vec::new()),
        }
    }

    /// The context of the running evaluation, or an error naming the native that needs it.
    pub fn get<'a>(eval: &Evaluator<'_, 'a, '_>) -> anyhow::Result<&'a EvalContext> {
        eval.extra
            .and_then(|e| e.downcast_ref::<EvalContext>())
            .ok_or_else(|| anyhow::anyhow!("no Blaze evaluation context"))
    }
}

// ---------------------------------------------------------------------------------------------
// Label: simple value

/// A parsed label `@repo//package:name` (spike-level parsing only).
#[derive(Debug, Clone, PartialEq, Eq, Hash, ProvidesStaticType, NoSerialize, Allocative)]
pub struct Label {
    pub repo: String,
    pub package: String,
    pub name: String,
}

starlark_simple_value!(Label);

impl Label {
    pub fn parse(s: &str) -> anyhow::Result<Label> {
        let (repo, rest) = match s.strip_prefix('@') {
            Some(r) => {
                let r = r.trim_start_matches('@');
                let i = r
                    .find("//")
                    .ok_or_else(|| anyhow::anyhow!("invalid label '{s}'"))?;
                (r[..i].to_owned(), &r[i..])
            }
            None => (String::new(), s),
        };
        let rest = rest
            .strip_prefix("//")
            .ok_or_else(|| anyhow::anyhow!("invalid label '{s}': must start with //"))?;
        let (package, name) = match rest.split_once(':') {
            Some((p, n)) => (p.to_owned(), n.to_owned()),
            None => (
                rest.to_owned(),
                rest.rsplit('/').next().unwrap_or("").to_owned(),
            ),
        };
        Ok(Label {
            repo,
            package,
            name,
        })
    }
}

impl fmt::Display for Label {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "@@{}//{}:{}", self.repo, self.package, self.name)
    }
}

#[starlark_value(type = "Label")]
impl<'v> StarlarkValue<'v> for Label {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("Label", label_methods);
        Some(RES.methods())
    }

    fn collect_repr(&self, collector: &mut String) {
        collector.push_str(&format!("Label(\"{self}\")"));
    }

    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(Label::from_value(other).is_some_and(|o| o == self))
    }

    fn write_hash(&self, hasher: &mut starlark_map::StarlarkHasher) -> starlark::Result<()> {
        use std::hash::Hash;
        self.hash(hasher);
        Ok(())
    }
}

#[starlark_module]
fn label_methods(builder: &mut MethodsBuilder) {
    #[starlark(attribute)]
    fn name(this: &Label) -> anyhow::Result<String> {
        Ok(this.name.clone())
    }

    #[starlark(attribute)]
    fn package(this: &Label) -> anyhow::Result<String> {
        Ok(this.package.clone())
    }

    #[starlark(attribute)]
    fn repo_name(this: &Label) -> anyhow::Result<String> {
        Ok(this.repo.clone())
    }

    fn same_package_label(this: &Label, target_name: &str) -> anyhow::Result<Label> {
        Ok(Label {
            repo: this.repo.clone(),
            package: this.package.clone(),
            name: target_name.to_owned(),
        })
    }
}

// ---------------------------------------------------------------------------------------------
// Depset: complex value, frozen with its module

#[derive(Trace, Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct Depset<'v> {
    order: String,
    direct: Vec<Value<'v>>,
    transitive: Vec<Value<'v>>,
}

#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FrozenDepset {
    order: String,
    direct: Vec<FrozenValue>,
    transitive: Vec<FrozenValue>,
}

impl<'v> AllocValue<'v> for Depset<'v> {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

impl<'v> Freeze for Depset<'v> {
    type Frozen = FrozenDepset;
    fn freeze(self, freezer: &Freezer) -> FreezeResult<FrozenDepset> {
        Ok(FrozenDepset {
            order: self.order,
            direct: self.direct.freeze(freezer)?,
            transitive: self.transitive.freeze(freezer)?,
        })
    }
}

/// (order, direct, transitive) of a frozen or unfrozen depset.
fn depset_parts<'v>(v: Value<'v>) -> Option<(&'v str, Vec<Value<'v>>, Vec<Value<'v>>)> {
    if let Some(d) = v.downcast_ref::<Depset<'v>>() {
        return Some((&d.order, d.direct.clone(), d.transitive.clone()));
    }
    v.downcast_ref::<FrozenDepset>().map(|d| {
        (
            d.order.as_str(),
            d.direct.iter().map(|x| x.to_value()).collect(),
            d.transitive.iter().map(|x| x.to_value()).collect(),
        )
    })
}

/// Postorder flattening with dedup (Bazel "default"/"postorder" shape, simplified).
fn depset_to_list<'v>(v: Value<'v>, out: &mut SmallSet<Value<'v>>) -> starlark::Result<()> {
    let (_, direct, transitive) = depset_parts(v).expect("depset");
    for t in transitive {
        depset_to_list(t, out)?;
    }
    for d in direct {
        out.insert_hashed(d.get_hashed()?);
    }
    Ok(())
}

fn fmt_depset(f: &mut fmt::Formatter<'_>, n_direct: usize, n_trans: usize) -> fmt::Result {
    write!(f, "depset(<{n_direct} direct, {n_trans} transitive>)")
}

impl fmt::Display for Depset<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_depset(f, self.direct.len(), self.transitive.len())
    }
}

impl fmt::Display for FrozenDepset {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_depset(f, self.direct.len(), self.transitive.len())
    }
}

#[starlark_value(type = "depset")]
impl<'v> StarlarkValue<'v> for Depset<'v> {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("depset", depset_methods);
        Some(RES.methods())
    }
}

#[starlark_value(type = "depset")]
impl<'v> StarlarkValue<'v> for FrozenDepset {
    type Canonical = Depset<'v>;
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("depset", depset_methods);
        Some(RES.methods())
    }
}

#[starlark_module]
fn depset_methods(builder: &mut MethodsBuilder) {
    fn to_list<'v>(this: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let mut out = SmallSet::new();
        depset_to_list(this, &mut out)?;
        Ok(heap.alloc(AllocList(out.into_iter().collect::<Vec<_>>())))
    }
}

// ---------------------------------------------------------------------------------------------
// Provider: callable, named by export_as

#[derive(Trace, Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct Provider {
    /// (defining file label, exported name), set by `export_as`.
    #[allocative(skip)]
    key: RefCell<Option<(String, String)>>,
    fields: Option<Vec<String>>,
}

#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FrozenProvider {
    key: Option<(String, String)>,
    fields: Option<Vec<String>>,
}

impl<'v> AllocValue<'v> for Provider {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

impl Freeze for Provider {
    type Frozen = FrozenProvider;
    fn freeze(self, _freezer: &Freezer) -> FreezeResult<FrozenProvider> {
        Ok(FrozenProvider {
            key: self.key.into_inner(),
            fields: self.fields,
        })
    }
}

fn provider_name(key: &Option<(String, String)>) -> String {
    match key {
        Some((_, n)) => n.clone(),
        None => "<unexported provider>".to_owned(),
    }
}

impl fmt::Display for Provider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<provider {}>", provider_name(&self.key.borrow()))
    }
}

impl fmt::Display for FrozenProvider {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<provider {}>", provider_name(&self.key))
    }
}

/// Key of a provider value (frozen or not).
pub fn provider_key(v: Value) -> Option<(String, String)> {
    if let Some(p) = v.downcast_ref::<Provider>() {
        return p.key.borrow().clone();
    }
    v.downcast_ref::<FrozenProvider>()
        .and_then(|p| p.key.clone())
}

fn provider_call<'v>(
    me: Value<'v>,
    fields: &Option<Vec<String>>,
    args: &Arguments<'v, '_>,
    eval: &mut Evaluator<'v, '_, '_>,
) -> starlark::Result<Value<'v>> {
    args.no_positional_args(eval.heap())?;
    let mut out = Vec::new();
    for (k, v) in args.names_map()? {
        if let Some(allowed) = fields
            && !allowed.iter().any(|a| a == k.as_str())
        {
            return Err(starlark::Error::new_other(anyhow::anyhow!(
                "got unexpected field '{}' in call to instantiate provider {}",
                k.as_str(),
                provider_key(me).map(|k| k.1).unwrap_or_default()
            )));
        }
        out.push((k.as_str().to_owned(), v));
    }
    Ok(eval.heap().alloc(ProviderInstance {
        provider: me,
        fields: out,
    }))
}

#[starlark_value(type = "Provider")]
impl<'v> StarlarkValue<'v> for Provider {
    fn invoke(
        &self,
        me: Value<'v>,
        args: &Arguments<'v, '_>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        provider_call(me, &self.fields, args, eval)
    }

    // Bazel: StarlarkExportable.export, run by the post-assign hook at top-level assignment.
    fn export_as(
        &self,
        variable_name: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<()> {
        let mut key = self.key.borrow_mut();
        if key.is_none() {
            let file = EvalContext::get(eval)?.file_label.clone();
            *key = Some((file, variable_name.to_owned()));
        }
        Ok(())
    }
}

#[starlark_value(type = "Provider")]
impl<'v> StarlarkValue<'v> for FrozenProvider {
    type Canonical = Provider;
    fn invoke(
        &self,
        me: Value<'v>,
        args: &Arguments<'v, '_>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        provider_call(me, &self.fields, args, eval)
    }
}

#[derive(Trace, Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct ProviderInstance<'v> {
    provider: Value<'v>,
    fields: Vec<(String, Value<'v>)>,
}

#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct FrozenProviderInstance {
    provider: FrozenValue,
    fields: Vec<(String, FrozenValue)>,
}

impl<'v> AllocValue<'v> for ProviderInstance<'v> {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex(self)
    }
}

impl<'v> Freeze for ProviderInstance<'v> {
    type Frozen = FrozenProviderInstance;
    fn freeze(self, freezer: &Freezer) -> FreezeResult<FrozenProviderInstance> {
        Ok(FrozenProviderInstance {
            provider: self.provider.freeze(freezer)?,
            fields: self.fields.freeze(freezer)?,
        })
    }
}

fn fmt_instance(
    f: &mut fmt::Formatter<'_>,
    provider: Value,
    fields: impl Iterator<Item = (String, String)>,
) -> fmt::Result {
    let name = provider_key(provider).map(|k| k.1).unwrap_or_default();
    write!(f, "struct(")?;
    for (i, (k, v)) in fields.enumerate() {
        if i > 0 {
            write!(f, ", ")?;
        }
        write!(f, "{k} = {v}")?;
    }
    write!(f, ")")?;
    let _ = name;
    Ok(())
}

impl fmt::Display for ProviderInstance<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_instance(
            f,
            self.provider,
            self.fields.iter().map(|(k, v)| (k.clone(), v.to_repr())),
        )
    }
}

impl fmt::Display for FrozenProviderInstance {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt_instance(
            f,
            self.provider.to_value(),
            self.fields
                .iter()
                .map(|(k, v)| (k.clone(), v.to_value().to_repr())),
        )
    }
}

#[starlark_value(type = "struct")]
impl<'v> StarlarkValue<'v> for ProviderInstance<'v> {
    fn get_attr(&self, attribute: &str, _heap: Heap<'v>) -> Option<Value<'v>> {
        self.fields
            .iter()
            .find(|(k, _)| k == attribute)
            .map(|(_, v)| *v)
    }
    fn dir_attr(&self) -> Vec<String> {
        self.fields.iter().map(|(k, _)| k.clone()).collect()
    }
}

#[starlark_value(type = "struct")]
impl<'v> StarlarkValue<'v> for FrozenProviderInstance {
    type Canonical = ProviderInstance<'v>;
    fn get_attr(&self, attribute: &str, _heap: Heap<'v>) -> Option<Value<'v>> {
        self.fields
            .iter()
            .find(|(k, _)| k == attribute)
            .map(|(_, v)| v.to_value())
    }
    fn dir_attr(&self) -> Vec<String> {
        self.fields.iter().map(|(k, _)| k.clone()).collect()
    }
}

/// The provider of an instance, if `v` is a provider instance.
pub fn instance_provider(v: Value) -> Option<Value> {
    if let Some(i) = v.downcast_ref::<ProviderInstance>() {
        return Some(i.provider);
    }
    v.downcast_ref::<FrozenProviderInstance>()
        .map(|i| i.provider.to_value())
}

// ---------------------------------------------------------------------------------------------
// Ctx: analysis-only, never frozen

#[derive(Trace, Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct Ctx<'v> {
    pub label: Value<'v>,
    /// `ctx.attr.deps`-like list of provider instances.
    pub deps: Value<'v>,
}

impl<'v> AllocValue<'v> for Ctx<'v> {
    fn alloc_value(self, heap: Heap<'v>) -> Value<'v> {
        heap.alloc_complex_no_freeze(self)
    }
}

impl fmt::Display for Ctx<'_> {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<rule context for {}>", self.label)
    }
}

#[starlark_value(type = "ctx", UnpackValue, StarlarkTypeRepr)]
impl<'v> StarlarkValue<'v> for Ctx<'v> {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("ctx", ctx_methods);
        Some(RES.methods())
    }
}

#[starlark_module]
fn ctx_methods(builder: &mut MethodsBuilder) {
    #[starlark(attribute)]
    fn label<'v>(this: &Ctx<'v>) -> starlark::Result<Value<'v>> {
        Ok(this.label)
    }

    #[starlark(attribute)]
    fn deps<'v>(this: &Ctx<'v>) -> starlark::Result<Value<'v>> {
        Ok(this.deps)
    }

    /// Registers an action in the Rust-side analysis context.
    fn run<'v>(
        #[starlark(this)] _this: &Ctx<'v>,
        #[starlark(require = named)] mnemonic: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let cx = EvalContext::get(eval)?;
        if cx.phase != Phase::Analysis {
            anyhow::bail!("ctx.run() is only available during analysis");
        }
        cx.actions.borrow_mut().push(mnemonic.to_owned());
        Ok(NoneType)
    }
}

// ---------------------------------------------------------------------------------------------
// Globals

#[starlark_module]
pub fn spike_natives(builder: &mut GlobalsBuilder) {
    #[starlark(as_type = Label)]
    fn Label(#[starlark(require = pos)] s: &str) -> anyhow::Result<Label> {
        Label::parse(s)
    }

    fn depset<'v>(
        #[starlark(default = NoneOr::None)] direct: NoneOr<Value<'v>>,
        #[starlark(require = named, default = "default")] order: &str,
        #[starlark(require = named, default = NoneOr::None)] transitive: NoneOr<Value<'v>>,
    ) -> anyhow::Result<Depset<'v>> {
        let list = |v: NoneOr<Value<'v>>| -> anyhow::Result<Vec<Value<'v>>> {
            match v {
                NoneOr::None => Ok(Vec::new()),
                NoneOr::Other(v) => Ok(ListRef::from_value(v)
                    .ok_or_else(|| anyhow::anyhow!("expected a list"))?
                    .content()
                    .to_vec()),
            }
        };
        let transitive = list(transitive)?;
        for t in &transitive {
            if depset_parts(*t).is_none() {
                anyhow::bail!(
                    "for 'transitive' element: got {}, want depset",
                    t.get_type()
                );
            }
        }
        Ok(Depset {
            order: order.to_owned(),
            direct: list(direct)?,
            transitive,
        })
    }

    fn provider(
        #[starlark(require = named, default = NoneOr::None)] fields: NoneOr<
            UnpackListOrTuple<String>,
        >,
        #[starlark(require = named, default = "")] doc: &str,
    ) -> anyhow::Result<Provider> {
        let _ = doc;
        Ok(Provider {
            key: RefCell::new(None),
            fields: fields.into_option().map(|f| f.items),
        })
    }

    /// Bazel's print: `DEBUG: <file>:<line>:<col>: <args joined by sep>`.
    /// Java: net/starlark/java/eval/MethodLibrary.java:918 MethodLibrary.print
    fn print<'v>(
        #[starlark(args)] args: UnpackTuple<Value<'v>>,
        #[starlark(require = named, default = " ")] sep: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let msg = args
            .items
            .iter()
            .map(|v| v.to_str())
            .collect::<Vec<_>>()
            .join(sep);
        let loc = eval
            .call_stack_top_location()
            .map(|l| {
                let (line, col) = traceback::bazel_position(&l, true);
                format!("{}:{line}:{col}", l.filename())
            })
            .unwrap_or_default();
        EvalContext::get(eval)?
            .prints
            .borrow_mut()
            .push(format!("DEBUG: {loc}: {msg}"));
        Ok(NoneType)
    }

    /// A loading-phase native (stands in for a rule class call such as `cc_library`).
    fn rule_target<'v>(
        #[starlark(require = named)] name: &str,
        #[starlark(require = named, default = NoneOr::None)] deps: NoneOr<
            UnpackListOrTuple<String>,
        >,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> anyhow::Result<NoneType> {
        let cx = EvalContext::get(eval)?;
        if cx.phase != Phase::Loading {
            // Bazel's text for a rule called outside a BUILD-file thread.
            anyhow::bail!("'rule_target' can only be called during the loading phase");
        }
        cx.targets.borrow_mut().push((
            name.to_owned(),
            deps.into_option().map(|d| d.items).unwrap_or_default(),
        ));
        Ok(NoneType)
    }
}

/// Bazel universe + spike natives (the `print` here replaces starlark-rust's).
pub fn spike_globals() -> Globals {
    let (builder, _, _) = dialect::bazel_universe_builder();
    builder.with(spike_natives).build()
}

// ---------------------------------------------------------------------------------------------
// Loading and evaluating

/// `FileLoader` over modules that were frozen before this file runs.
#[derive(Default)]
pub struct MapLoader {
    pub modules: HashMap<String, FrozenModule>,
}

impl FileLoader for MapLoader {
    fn load(&self, path: &str) -> starlark::Result<FrozenModule> {
        self.modules.get(path).cloned().ok_or_else(|| {
            starlark::Error::new_other(anyhow::anyhow!("cannot load '{path}': not resolved"))
        })
    }
}

/// Evaluates a .bzl/BUILD file into a frozen module. The context is returned so callers can
/// read what natives recorded.
pub fn eval_file(
    name: &str,
    src: &str,
    globals: &Globals,
    loader: &MapLoader,
    cx: &EvalContext,
) -> starlark::Result<FrozenModule> {
    let ast = AstModule::parse(name, src.to_owned(), &dialect::bzl_dialect())?;
    Module::with_temp_heap(|module| {
        {
            let mut eval = Evaluator::new(&module);
            eval.set_loader(loader);
            eval.extra = Some(cx);
            eval.eval_module(ast, globals)?;
        }
        module
            .freeze()
            .map_err(|e| starlark::Error::new_other(anyhow::anyhow!("{e:?}")))
    })
}

/// Brings a value frozen elsewhere (e.g. a function from a loaded .bzl, or a provider instance
/// from a dependency's analysis) into `module`'s heap brand.
///
/// starlark-rust 0.14's `OwnedFrozenValue::owned_value(&FrozenHeap)` cannot be used with
/// `Module::with_temp_heap`: the returned `Value<'x>` is tied to the borrow of the frozen heap,
/// not to the module's invariant heap brand `'v`. This is the one `unsafe` Blaze needs.
pub fn frozen_into<'v>(module: &Module<'v>, v: &starlark::values::OwnedFrozenValue) -> Value<'v> {
    // SAFETY: `owned_frozen_value` adds `v`'s heap as a reference of `module`'s frozen heap, so
    // the frozen value lives at least as long as the module (and anything frozen from it).
    Value::new_frozen(unsafe { v.owned_frozen_value(module.frozen_heap()) })
}
