//! Providers defined in Starlark and the `provider()` builtin.
//!
//! Java: packages/StarlarkProvider.java (with its `RawConstructor`, argument processors and the
//! `StarlarkInfoFactory`s of packages/StarlarkInfoNoSchema.java and StarlarkInfoWithSchema.java)
//! and analysis/starlark/StarlarkRuleClassFunctions.java:381 `provider`.
//!
//! Instances are `blaze_starlark::structs::StarlarkInfo` values whose provider is the
//! [`StarlarkProvider`] value. A provider value describes itself to blaze_starlark with
//! `ProviderInfo` (`StarlarkValue::provide`).
//!
//! PORT: Java's `StarlarkInfoWithSchema` stores fields in a table indexed by the schema and
//! optimises depset fields to their `NestedSet` (`optimizeField`, a memory optimisation); Blaze
//! stores the set fields of schemaful instances like schemaless ones (sorted by name), which is
//! what `getFieldNames`/`getValue`/`repr` observe. The creation location of instances is not
//! kept (blaze_starlark's `StarlarkInfo` has none).

use std::fmt;
use std::hash::Hash;
use std::sync::OnceLock;

use allocative::Allocative;
use blaze_cmdline::Label;
use blaze_starlark::Location;
use blaze_starlark::ThreadContext;
use blaze_starlark::structs::ProviderInfo;
use blaze_starlark::structs::StarlarkInfo;
use starlark::any::ProvidesStaticType;
use starlark::coerce::Coerce;
use starlark::environment::GlobalsBuilder;
use starlark::eval::Arguments;
use starlark::eval::Evaluator;
use starlark::starlark_complex_value;
use starlark::starlark_module;
use starlark::values::Demand;
use starlark::values::Freeze;
use starlark::values::FreezeResult;
use starlark::values::Freezer;
use starlark::values::FrozenValue;
use starlark::values::NoSerialize;
use starlark::values::StarlarkValue;
use starlark::values::Trace;
use starlark::values::UnpackValue;
use starlark::values::Value;
use starlark::values::ValueLifetimeless;
use starlark::values::ValueLike;
use starlark::values::starlark_value;
use starlark::values::typing::StarlarkCallable;
use starlark_map::StarlarkHasher;

use crate::bzl_context::BzlInitThreadContext;
use crate::rule_class::ProviderKey;
use crate::starlark_defs::builtin_providers::builtin_provider_key;
use crate::starlark_defs::util::dict_cast;
use crate::starlark_defs::util::dict_cast_string_string;
use crate::starlark_defs::util::dict_entries;
use crate::starlark_defs::util::errorf;
use crate::starlark_defs::util::sequence_cast_strings;
use crate::starlark_defs::util::sequence_elements;
use crate::starlark_defs::util::starlark_type;
use crate::starlark_defs::util::trim_doc_string;

// -------------------------------------------------------------------------------------------
// ProviderView: what agent B's rule()/aspect() code needs to know about any provider value.

/// Java `x instanceof Provider` with `isExported()` and `getKey()` (`None` when unexported).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProviderView {
    pub exported: bool,
    pub key: Option<ProviderKey>,
}

/// The provider view of `v`, or `None` if `v` is not a provider: Starlark-defined providers and
/// every builtin provider (including `struct`).
pub fn provider_view(v: Value) -> Option<ProviderView> {
    if let Some(p) = StarlarkProvider::from_value(v) {
        return Some(ProviderView {
            exported: p.is_exported(),
            key: p.key.get().cloned(),
        });
    }
    // Java: BuiltinProvider.isExported is always true.
    builtin_provider_key(v).map(|key| ProviderView {
        exported: true,
        key: Some(key),
    })
}

// -------------------------------------------------------------------------------------------
// StarlarkProvider

/// Java `SymbolGenerator.Symbol`: the identity of an unexported provider (the evaluation's owner
/// and a per-evaluation counter, `StarlarkThread.getNextIdentityToken`).
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct IdentityToken {
    pub owner: String,
    pub index: u64,
}

impl fmt::Display for IdentityToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}#{}", self.owner, self.index)
    }
}

impl IdentityToken {
    /// Java `thread.getNextIdentityToken()`.
    // Java: net/starlark/java/eval/StarlarkThread.java StarlarkThread.getNextIdentityToken
    pub fn next(eval: &Evaluator) -> starlark::Result<IdentityToken> {
        let cx = ThreadContext::get(eval)?;
        Ok(IdentityToken {
            // PORT: Java's owner is the thread's owner (a BzlLoadValue.Key during .bzl
            // evaluation); Blaze uses the label of the evaluated file.
            owner: cx
                .file_label
                .as_ref()
                .map(|l| l.get_canonical_form())
                .unwrap_or_default(),
            index: cx.next_symbol(),
        })
    }
}

/// Java `keyOrIdentityToken`, as seen at one moment.
#[derive(Debug, PartialEq, Eq)]
enum KeyOrIdentityToken<'a> {
    Key(&'a ProviderKey),
    Token(&'a IdentityToken),
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

/// A provider defined in Starlark (Java `StarlarkProvider`), schemaless or schemaful, with an
/// optional custom initializer.
// Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:72 StarlarkProvider
#[derive(Debug, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[repr(C)]
pub struct StarlarkProviderGen<V: ValueLifetimeless> {
    #[trace(unsafe_ignore)]
    #[allocative(skip)]
    location: Location,
    #[trace(unsafe_ignore)]
    documentation: Option<String>,
    /// For schemaful providers, the sorted list of allowed field names.
    #[trace(unsafe_ignore)]
    fields: Option<Vec<String>>,
    /// For schemaful providers, field name -> documentation, in `provider()` order.
    #[trace(unsafe_ignore)]
    schema: Option<Vec<(String, Option<String>)>>,
    /// Optional custom initializer callback.
    init: Option<V>,
    /// The identity token while unexported (`None` for providers built exported).
    #[trace(unsafe_ignore)]
    #[allocative(skip)]
    identity_token: Option<IdentityToken>,
    /// Set by `export` (Java replaces `keyOrIdentityToken` by the `Key`).
    #[trace(unsafe_ignore)]
    #[allocative(skip)]
    key: OnceLock<ProviderKey>,
}

starlark_complex_value!(pub StarlarkProvider);

// SAFETY: `repr(C)`, and `FrozenValue`/`Value` have the same layout.
unsafe impl<'v> Coerce<StarlarkProviderGen<Value<'v>>> for StarlarkProviderGen<FrozenValue> {}

impl<'v> Freeze for StarlarkProvider<'v> {
    type Frozen = FrozenStarlarkProvider;
    fn freeze(self, freezer: &Freezer) -> FreezeResult<FrozenStarlarkProvider> {
        Ok(StarlarkProviderGen {
            location: self.location,
            documentation: self.documentation,
            fields: self.fields,
            schema: self.schema,
            init: self.init.freeze(freezer)?,
            identity_token: self.identity_token,
            key: self.key,
        })
    }
}

/// Java `StarlarkProvider.Builder`.
// Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:145 StarlarkProvider.Builder
pub struct StarlarkProviderBuilder<V> {
    location: Location,
    documentation: Option<String>,
    schema: Option<Vec<(String, Option<String>)>>,
    init: Option<V>,
}

impl<V: ValueLifetimeless> StarlarkProviderBuilder<V> {
    /// Java `StarlarkProvider.builder(location)`.
    // Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:140 StarlarkProvider.builder
    pub fn new(location: Location) -> Self {
        StarlarkProviderBuilder {
            location,
            documentation: None,
            schema: None,
            init: None,
        }
    }

    /// Java `Builder.setSchema(Collection<String>)`: fields without documentation.
    // Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:163 StarlarkProvider.Builder.setSchema
    pub fn set_schema(mut self, fields: Vec<String>) -> Self {
        let mut schema: Vec<(String, Option<String>)> = Vec::new();
        for field in fields {
            // Java: ImmutableMap.Builder.buildOrThrow rejects duplicate keys.
            if schema.iter().any(|(k, _)| *k == field) {
                panic!(
                    "IllegalArgumentException: Multiple entries with same key: \
                     {field}=Optional.empty and {field}=Optional.empty"
                );
            }
            schema.push((field, None));
        }
        self.schema = Some(schema);
        self
    }

    /// Java `Builder.setSchema(Map<String, String>)`: fields with documentation.
    // Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:177 StarlarkProvider.Builder.setSchema
    pub fn set_documented_schema(
        mut self,
        schema_with_documentation: Vec<(String, String)>,
    ) -> Self {
        self.schema = Some(
            schema_with_documentation
                .into_iter()
                .map(|(k, v)| (k, Some(v)))
                .collect(),
        );
        self
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:188 StarlarkProvider.Builder.setDocumentation
    pub fn set_documentation(mut self, documentation: String) -> Self {
        self.documentation = Some(documentation);
        self
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:207 StarlarkProvider.Builder.setInit
    pub fn set_init(mut self, init: V) -> Self {
        self.init = Some(init);
        self
    }

    /// Builds an exported provider.
    // Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:213 StarlarkProvider.Builder.buildExported
    pub fn build_exported(self, key: ProviderKey) -> StarlarkProviderGen<V> {
        let cell = OnceLock::new();
        let _ = cell.set(key);
        self.build(None, cell)
    }

    /// Builds an unexported provider.
    // Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:218 StarlarkProvider.Builder.buildWithIdentityToken
    pub fn build_with_identity_token(
        self,
        identity_token: IdentityToken,
    ) -> StarlarkProviderGen<V> {
        self.build(Some(identity_token), OnceLock::new())
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:230 StarlarkProvider.<init>
    fn build(
        self,
        identity_token: Option<IdentityToken>,
        key: OnceLock<ProviderKey>,
    ) -> StarlarkProviderGen<V> {
        let fields = self.schema.as_ref().map(|s| {
            let mut f: Vec<String> = s.iter().map(|(k, _)| k.clone()).collect();
            f.sort();
            f
        });
        StarlarkProviderGen {
            location: self.location,
            documentation: self.documentation,
            fields,
            schema: self.schema,
            init: self.init,
            identity_token,
            key,
        }
    }
}

impl<V: ValueLifetimeless> StarlarkProviderGen<V> {
    // Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:398 StarlarkProvider.getInit
    pub fn get_init(&self) -> Option<V> {
        self.init
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:403 StarlarkProvider.getLocation
    pub fn get_location(&self) -> &Location {
        &self.location
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:411 StarlarkProvider.getDocumentation
    pub fn get_documentation(&self) -> Option<&str> {
        self.documentation.as_deref()
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:416 StarlarkProvider.isExported
    pub fn is_exported(&self) -> bool {
        self.key.get().is_some()
    }

    /// Java `getKey`; panics on an unexported provider.
    // Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:421 StarlarkProvider.getKey
    pub fn get_key(&self) -> &ProviderKey {
        match self.key.get() {
            Some(k) => k,
            None => panic!(
                "IllegalStateException: Calling getKey() is disallowed on an unexported provider. \
                 location: {}, identity token: {}",
                self.location,
                self.identity_token
                    .as_ref()
                    .map(|t| t.to_string())
                    .unwrap_or_default()
            ),
        }
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:432 StarlarkProvider.getName
    pub fn get_name(&self) -> String {
        match self.key.get() {
            Some(ProviderKey::Starlark { exported_name, .. }) => exported_name.clone(),
            // PORT: a Starlark provider's key is always a StarlarkProvider.Key.
            Some(k) => k.to_string(),
            None => "<no name>".to_owned(),
        }
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:440 StarlarkProvider.getPrintableName
    pub fn get_printable_name(&self) -> String {
        self.get_name()
    }

    /// Sorted allowed field names, or `None` if schemaless.
    // Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:449 StarlarkProvider.getFields
    pub fn get_fields(&self) -> Option<&[String]> {
        self.fields.as_deref()
    }

    /// Field -> documentation in `provider()` order, or `None` if schemaless.
    // Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:461 StarlarkProvider.getSchema
    pub fn get_schema(&self) -> Option<&[(String, Option<String>)]> {
        self.schema.as_deref()
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:466 StarlarkProvider.getErrorMessageForUnknownField
    pub fn get_error_message_for_unknown_field(&self, name: &str) -> String {
        format!(
            "'{}' value has no field or method '{}'",
            if self.is_exported() {
                self.get_name()
            } else {
                "struct".to_owned()
            },
            name
        )
    }

    /// Java `export`: the provider becomes identified by `(extension_label, exported_name)`.
    // TODO(bazel-team): use exportedLocation as the callable symbol's location.
    // PORT: Java derives the key's BzlLoadValue.Key from the identity token's owner; Blaze keys
    // Starlark providers by the .bzl label (rule_class/provider.rs).
    // Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:473 StarlarkProvider.export
    pub fn export(&self, extension_label: Label, exported_name: &str) {
        blaze_util::check_state!(!self.is_exported());
        let _ = self.key.set(ProviderKey::Starlark {
            extension_label,
            exported_name: exported_name.to_owned(),
        });
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:522 StarlarkProvider.isImmutable
    pub fn is_immutable(&self) -> bool {
        // Hash code for non exported constructors may be changed
        self.is_exported()
    }

    fn key_or_identity_token(&self) -> KeyOrIdentityToken<'_> {
        match self.key.get() {
            Some(k) => KeyOrIdentityToken::Key(k),
            None => KeyOrIdentityToken::Token(
                self.identity_token
                    .as_ref()
                    .expect("unexported provider has an identity token"),
            ),
        }
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:260 StarlarkProvider.newStarlarkInfoFactory
    fn new_starlark_info_factory<'v>(&self, provider: Value<'v>) -> StarlarkInfoFactory<'v> {
        let printable_name = self.get_printable_name();
        match &self.fields {
            Some(fields) => StarlarkInfoFactory::WithSchema {
                provider,
                printable_name,
                fields: fields.clone(),
                value_table: vec![None; fields.len()],
                unexpected: None,
            },
            None => StarlarkInfoFactory::NoSchema {
                provider,
                printable_name,
                named_arg_map: Vec::new(),
            },
        }
    }
}

impl<V: ValueLifetimeless> fmt::Display for StarlarkProviderGen<V> {
    // Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:528 StarlarkProvider.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "<provider>")
    }
}

#[starlark_value(type = "Provider")]
impl<'v, V: ValueLike<'v>> StarlarkValue<'v> for StarlarkProviderGen<V>
where
    Self: ProvidesStaticType<'v>,
    V: FrozenMarker,
{
    // Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:248 StarlarkProvider.requestArgumentProcessor
    fn invoke(
        &self,
        me: Value<'v>,
        args: &Arguments<'v, '_>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        match self.init {
            Some(init) => {
                // Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:298 StarlarkProvider.ArgumentProcessorWithInit.call
                let init_result = init.invoke(args, eval)?;
                let kwargs = dict_cast(
                    init_result,
                    "string",
                    |k| k.unpack_str().is_some(),
                    "unknown",
                    |_| true,
                    "return value of provider init()",
                )?;
                let map = kwargs
                    .into_iter()
                    .map(|(k, v)| (k.unpack_str().unwrap().to_owned(), v))
                    .collect();
                self.new_starlark_info_factory(me)
                    .create_from_map(map, eval)
            }
            None => raw_call(me, self, &self.get_name(), args, eval),
        }
    }

    fn name_for_call_stack(&self, _me: Value<'v>) -> String {
        self.get_name()
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:511 StarlarkProvider.equals
    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(match StarlarkProvider::from_value(other) {
            Some(that) => self.key_or_identity_token() == that.key_or_identity_token(),
            None => false,
        })
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:506 StarlarkProvider.hashCode
    // and StarlarkValue.checkHashable (unexported providers are not immutable).
    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        match self.key.get() {
            Some(k) => {
                k.hash(hasher);
                Ok(())
            }
            None => Err(errorf("unhashable type: 'Provider'")),
        }
    }

    /// Java: the post-assignment hook of BzlLoadFunction, which exports every unexported
    /// `StarlarkExportable` bound at the top level of a .bzl file.
    // Java: com/google/devtools/build/lib/skyframe/BzlLoadFunction.java BzlLoadFunction.executeBzlFile
    fn export_as(
        &self,
        variable_name: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<()> {
        // PORT: a frozen provider (loaded from another .bzl) is never exported here; Java would
        // fail its owner check (`export extensionLabel=..., but owner=...`).
        if V::FROZEN || self.is_exported() {
            return Ok(());
        }
        let extension_label = match BzlInitThreadContext::from_or_null(eval) {
            Some(ctx) => ctx.bzl_file.clone(),
            // Not a .bzl initialization thread: no post-assignment hook.
            None => return Ok(()),
        };
        self.export(extension_label, variable_name);
        Ok(())
    }

    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(ProviderInfo {
            printable_name: self.get_printable_name(),
            exported: self.is_exported(),
        });
    }
}

/// Java `RawArgumentProcessor` (calling a provider without init, or its raw constructor).
// Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:311 StarlarkProvider.RawArgumentProcessor
fn raw_call<'v, V: ValueLifetimeless>(
    provider_value: Value<'v>,
    provider: &StarlarkProviderGen<V>,
    owner_name: &str,
    args: &Arguments<'v, '_>,
    eval: &mut Evaluator<'v, '_, '_>,
) -> starlark::Result<Value<'v>> {
    // Java: RawArgumentProcessor.addPositionalArg
    if args.positions(eval.heap())?.next().is_some() {
        return Err(errorf(format!(
            "{owner_name}: unexpected positional arguments"
        )));
    }
    let mut factory = provider.new_starlark_info_factory(provider_value);
    // Java: RawArgumentProcessor.addNamedArg
    for (name, value) in args.names_map()? {
        factory.add_named_arg(name.as_str(), value)?;
    }
    // Java: RawArgumentProcessor.call
    factory.create_from_args(eval)
}

/// Java `StarlarkInfoNoSchema.StarlarkInfoFactory` / `StarlarkInfoWithSchema.StarlarkInfoFactory`.
enum StarlarkInfoFactory<'v> {
    // Java: com/google/devtools/build/lib/packages/StarlarkInfoNoSchema.java:118 StarlarkInfoNoSchema.StarlarkInfoFactory
    NoSchema {
        provider: Value<'v>,
        printable_name: String,
        named_arg_map: Vec<(String, Value<'v>)>,
    },
    // Java: com/google/devtools/build/lib/packages/StarlarkInfoWithSchema.java:77 StarlarkInfoWithSchema.StarlarkInfoFactory
    WithSchema {
        provider: Value<'v>,
        printable_name: String,
        fields: Vec<String>,
        value_table: Vec<Option<Value<'v>>>,
        unexpected: Option<Vec<String>>,
    },
}

impl<'v> StarlarkInfoFactory<'v> {
    fn add_named_arg(&mut self, name: &str, value: Value<'v>) -> starlark::Result<()> {
        match self {
            // Java: com/google/devtools/build/lib/packages/StarlarkInfoNoSchema.java:127 StarlarkInfoNoSchema.StarlarkInfoFactory.addNamedArg
            StarlarkInfoFactory::NoSchema {
                printable_name,
                named_arg_map,
                ..
            } => {
                if let Some(old) = named_arg_map.iter_mut().find(|(k, _)| k == name) {
                    old.1 = value;
                    return Err(errorf(format!(
                        "got multiple values for parameter {name} in call to instantiate provider {printable_name}"
                    )));
                }
                named_arg_map.push((name.to_owned(), value));
                Ok(())
            }
            // Java: com/google/devtools/build/lib/packages/StarlarkInfoWithSchema.java:90 StarlarkInfoWithSchema.StarlarkInfoFactory.addNamedArg
            StarlarkInfoFactory::WithSchema {
                printable_name,
                fields,
                value_table,
                unexpected,
                ..
            } => {
                match fields.binary_search_by(|f| f.as_str().cmp(name)) {
                    Ok(pos) => {
                        if value_table[pos].is_some() {
                            return Err(errorf(format!(
                                "got multiple values for parameter {name} in call to instantiate provider {printable_name}"
                            )));
                        }
                        // PORT: no depset optimisation (`provider.optimizeField`).
                        value_table[pos] = Some(value);
                    }
                    Err(_) => {
                        unexpected
                            .get_or_insert_with(Vec::new)
                            .push(name.to_owned());
                    }
                }
                Ok(())
            }
        }
    }

    fn create_from_args(self, eval: &mut Evaluator<'v, '_, '_>) -> starlark::Result<Value<'v>> {
        match self {
            // Java: com/google/devtools/build/lib/packages/StarlarkInfoNoSchema.java:138 StarlarkInfoNoSchema.StarlarkInfoFactory.createFromArgs
            StarlarkInfoFactory::NoSchema {
                provider,
                named_arg_map,
                ..
            } => Ok(eval
                .heap()
                .alloc(StarlarkInfo::new(provider, named_arg_map))),
            // Java: com/google/devtools/build/lib/packages/StarlarkInfoWithSchema.java:108 StarlarkInfoWithSchema.StarlarkInfoFactory.createFromArgs
            StarlarkInfoFactory::WithSchema {
                provider,
                printable_name,
                fields,
                value_table,
                unexpected,
            } => {
                if let Some(unexpected) = unexpected {
                    return Err(errorf(format!(
                        "got unexpected field{} '{}' in call to instantiate provider {}",
                        if unexpected.len() > 1 { "s" } else { "" },
                        unexpected.join("', '"),
                        printable_name
                    )));
                }
                let set: Vec<(String, Value<'v>)> = fields
                    .into_iter()
                    .zip(value_table)
                    .filter_map(|(k, v)| v.map(|v| (k, v)))
                    .collect();
                Ok(eval.heap().alloc(StarlarkInfo::new(provider, set)))
            }
        }
    }

    fn create_from_map(
        mut self,
        map: Vec<(String, Value<'v>)>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        match self {
            // Java: com/google/devtools/build/lib/packages/StarlarkInfoNoSchema.java:143 StarlarkInfoNoSchema.StarlarkInfoFactory.createFromMap
            StarlarkInfoFactory::NoSchema { provider, .. } => {
                Ok(eval.heap().alloc(StarlarkInfo::new(provider, map)))
            }
            // Java: com/google/devtools/build/lib/packages/StarlarkInfoWithSchema.java:119 StarlarkInfoWithSchema.StarlarkInfoFactory.createFromMap
            StarlarkInfoFactory::WithSchema { .. } => {
                for (k, v) in map {
                    self.add_named_arg(&k, v)?;
                }
                self.create_from_args(eval)
            }
        }
    }
}

// -------------------------------------------------------------------------------------------
// RawConstructor

/// The raw constructor of a provider with `init` (Java `StarlarkProvider.RawConstructor`).
// Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:362 StarlarkProvider.RawConstructor
#[derive(Debug, Trace, ProvidesStaticType, NoSerialize, Allocative)]
#[repr(C)]
pub struct RawConstructorGen<V: ValueLifetimeless> {
    provider: V,
}

starlark_complex_value!(pub RawConstructor);

// SAFETY: `repr(C)`, and `FrozenValue`/`Value` have the same layout.
unsafe impl<'v> Coerce<RawConstructorGen<Value<'v>>> for RawConstructorGen<FrozenValue> {}

impl<'v> Freeze for RawConstructor<'v> {
    type Frozen = FrozenRawConstructor;
    fn freeze(self, freezer: &Freezer) -> FreezeResult<FrozenRawConstructor> {
        Ok(RawConstructorGen {
            provider: self.provider.freeze(freezer)?,
        })
    }
}

impl<'v, V: ValueLike<'v>> RawConstructorGen<V> {
    fn provider(&self) -> &'v StarlarkProvider<'v> {
        StarlarkProvider::from_value(self.provider.to_value())
            .expect("raw constructor of a StarlarkProvider")
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:375 StarlarkProvider.RawConstructor.getName
    pub fn get_name(&self) -> String {
        let provider = self.provider();
        let mut name = String::from("<raw constructor");
        if provider.is_exported() {
            name.push_str(" for ");
            name.push_str(&provider.get_name());
        }
        name.push('>');
        name
    }
}

impl<V: ValueLifetimeless> fmt::Display for RawConstructorGen<V> {
    // Java: StarlarkValue.repr default (RawConstructor has no repr).
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "<unknown object com.google.devtools.build.lib.packages.StarlarkProvider$RawConstructor>"
        )
    }
}

// Java: Starlark.type of a class without @StarlarkBuiltin is its simple name.
#[starlark_value(type = "RawConstructor")]
impl<'v, V: ValueLike<'v>> StarlarkValue<'v> for RawConstructorGen<V>
where
    Self: ProvidesStaticType<'v>,
{
    // Java: com/google/devtools/build/lib/packages/StarlarkProvider.java:370 StarlarkProvider.RawConstructor.requestArgumentProcessor
    fn invoke(
        &self,
        _me: Value<'v>,
        args: &Arguments<'v, '_>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        let name = self.get_name();
        raw_call(self.provider.to_value(), self.provider(), &name, args, eval)
    }

    fn name_for_call_stack(&self, _me: Value<'v>) -> String {
        self.get_name()
    }

    // Java: StarlarkValue.checkHashable (isImmutable is false).
    fn write_hash(&self, _hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        Err(errorf("unhashable type: 'RawConstructor'"))
    }
}

// -------------------------------------------------------------------------------------------
// provider()

/// Java `Starlark.type` check of a `@Param` with `allowedTypes` (BuiltinFunction).
// Java: net/starlark/java/eval/BuiltinFunction.java:516 BuiltinFunction.checkParamValue
fn param_type_error(method: &str, param: &str, value: Value, want: &str) -> starlark::Error {
    errorf(format!(
        "in call to {method}(), parameter '{param}' got value of type '{}', want '{want}'",
        starlark_type(value)
    ))
}

/// Registers `provider` (Java `StarlarkRuleClassFunctions.provider`).
#[starlark_module]
pub fn register_provider(builder: &mut GlobalsBuilder) {
    // Java: com/google/devtools/build/lib/analysis/starlark/StarlarkRuleClassFunctions.java:381 StarlarkRuleClassFunctions.provider
    fn provider<'v>(
        doc: Option<Value<'v>>,
        #[starlark(require = named)] fields: Option<Value<'v>>,
        #[starlark(require = named)] init: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        let none = Value::new_none();
        let doc = doc.unwrap_or(none);
        let fields = fields.unwrap_or(none);
        let init = init.unwrap_or(none);
        // Java: the @Param allowedTypes checks of StarlarkRuleFunctionsApi.provider.
        if !doc.is_none() && doc.unpack_str().is_none() {
            return Err(param_type_error(
                "provider",
                "doc",
                doc,
                "string or NoneType",
            ));
        }
        let fields_is_sequence = sequence_elements(fields).is_some();
        let fields_is_dict = dict_entries(fields).is_some();
        if !fields.is_none() && !fields_is_sequence && !fields_is_dict {
            return Err(param_type_error(
                "provider",
                "fields",
                fields,
                "sequence, dict, or NoneType",
            ));
        }
        let init_callable =
            <StarlarkCallable<'v> as UnpackValue<'v>>::unpack_value_opt(init).is_some();
        if !init.is_none() && !init_callable {
            return Err(param_type_error(
                "provider",
                "init",
                init,
                "callable or NoneType",
            ));
        }

        let mut builder = StarlarkProviderBuilder::<Value<'v>>::new(
            crate::starlark_defs::util::caller_location(eval),
        );
        if let Some(doc) = doc.unpack_str() {
            builder = builder.set_documentation(trim_doc_string(doc));
        }
        if fields_is_sequence {
            builder = builder.set_schema(sequence_cast_strings(fields, "fields")?);
        } else if fields_is_dict {
            builder = builder.set_documented_schema(
                dict_cast_string_string(fields, "fields")?
                    .into_iter()
                    .map(|(k, v)| (k, trim_doc_string(&v)))
                    .collect(),
            );
        }
        if init.is_none() {
            let token = IdentityToken::next(eval)?;
            return Ok(eval.heap().alloc(builder.build_with_identity_token(token)));
        }
        builder = builder.set_init(init);
        let token = IdentityToken::next(eval)?;
        let provider = eval.heap().alloc(builder.build_with_identity_token(token));
        let raw = eval.heap().alloc(RawConstructorGen { provider });
        Ok(eval.heap().alloc((provider, raw)))
    }
}

// -------------------------------------------------------------------------------------------
// Test support (shared by the tests of the starlark_defs files of this helper).

#[cfg(test)]
pub(crate) mod test_support {
    use std::sync::Arc;

    use blaze_cmdline::Label;
    use blaze_starlark::EvalError;
    use blaze_starlark::FileKind;
    use blaze_starlark::Phase;
    use blaze_starlark::StarlarkSemantics;
    use blaze_starlark::ThreadContext;
    use blaze_starlark::env::NativeRegistry;
    use blaze_starlark::eval;
    use blaze_starlark::loader::LoadMap;
    use blaze_starlark::thread::CollectingEvents;
    use starlark::environment::FrozenModule;

    use crate::bzl_context::BazelModuleContext;
    use crate::bzl_context::BzlInitThreadContext;

    /// The registry of the test .bzl environment: blaze_starlark's builtins plus this helper's.
    pub fn registry() -> NativeRegistry {
        let mut r = NativeRegistry::with_blaze_starlark_builtins();
        let bzl = [FileKind::Bzl];
        r.register(&bzl, super::register_provider);
        r.register(
            &bzl,
            crate::starlark_defs::builtin_providers::register_builtin_providers,
        );
        r.register(
            &bzl,
            crate::starlark_defs::platform_common::register_platform_common,
        );
        r.register(&bzl, crate::starlark_defs::testing::register_testing);
        r.register(
            &bzl,
            crate::starlark_defs::config_common::register_config_common,
        );
        r
    }

    /// Executes `src` as the top level of `//pkg:<file_name>` (a .bzl initialization thread
    /// with its `BazelModuleContext` registered under `file_name`).
    pub fn exec_bzl(file_name: &str, src: &str) -> Result<FrozenModule, EvalError> {
        let file_name = file_name.to_owned();
        let src = src.to_owned();
        eval::run_on_evaluation_thread(move || {
            let label = Label::parse_canonical(&format!("//pkg:{file_name}")).unwrap();
            let mapping = blaze_cmdline::repository_mapping::EMPTY.clone();
            let module_context = Arc::new(BazelModuleContext {
                label: label.clone(),
                is_builtins: false,
                repo_mapping: mapping.clone(),
                filename: file_name.clone(),
                loads: Vec::new(),
                bzl_transitive_digest: Vec::new(),
            });
            let semantics = StarlarkSemantics::default();
            let ctx = ThreadContext::new(
                Phase::BzlLoad,
                Arc::new(semantics.clone()),
                Arc::new(CollectingEvents::default()),
            )
            .with_file_label(label.clone())
            .with_main_repo_mapping(mapping.clone())
            .with_file_package_context(
                file_name.clone(),
                blaze_cmdline::LabelPackageContext::of(
                    label.get_package_identifier(),
                    mapping.clone(),
                ),
            )
            .with_payload(BzlInitThreadContext {
                bzl_file: label,
                transitive_digest: Vec::new(),
                bzl_visibility: None,
                tools_repository: blaze_cmdline::repository_name::BAZEL_TOOLS.clone(),
                fragment_names: Vec::new(),
                main_repo_mapping: mapping,
            });
            let globals = registry().build(FileKind::Bzl, &semantics);
            eval::compile(FileKind::Bzl, &file_name, src).and_then(|c| {
                eval::exec_module_with_extra_value(
                    c,
                    &globals,
                    &LoadMap::new(),
                    &ctx,
                    &Default::default(),
                    // Java: the module's client data (BazelModuleContext).
                    |module| {
                        module
                            .heap()
                            .alloc(crate::bzl_context::BazelModuleContextValue(
                                module_context.clone(),
                            ))
                    },
                )
            })
        })
    }

    /// The string value of the global `name` of `m`.
    pub fn string_global(m: &FrozenModule, name: &str) -> String {
        m.get(name)
            .unwrap()
            .value()
            .unpack_str()
            .unwrap()
            .to_owned()
    }

    /// The error message of a failed evaluation.
    pub fn error_message(r: Result<FrozenModule, EvalError>) -> String {
        match r {
            Ok(_) => panic!("evaluation succeeded"),
            Err(e) => e.message,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::test_support::error_message;
    use super::test_support::exec_bzl;
    use super::test_support::string_global;
    use super::*;

    fn token(i: u64) -> IdentityToken {
        IdentityToken {
            owner: "test".to_owned(),
            index: i,
        }
    }

    fn key(label: &str, name: &str) -> ProviderKey {
        ProviderKey::Starlark {
            extension_label: Label::parse_canonical(label).unwrap(),
            exported_name: name.to_owned(),
        }
    }

    type P = StarlarkProviderGen<FrozenValue>;

    fn builder() -> StarlarkProviderBuilder<FrozenValue> {
        StarlarkProviderBuilder::new(Location::builtin())
    }

    #[test]
    fn unexported_provider_accessors() {
        let provider: P = builder().build_with_identity_token(token(0));
        assert!(!provider.is_exported());
        assert_eq!(provider.get_name(), "<no name>");
        assert_eq!(provider.get_printable_name(), "<no name>");
        assert_eq!(
            provider.get_error_message_for_unknown_field("foo"),
            "'struct' value has no field or method 'foo'"
        );
        assert!(!provider.is_immutable());
        assert_eq!(provider.to_string(), "<provider>");
        assert!(
            std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| provider.get_key().clone()))
                .is_err()
        );
    }

    #[test]
    fn unexported_provider_raw_constructor_name() {
        let m = exec_bzl(
            "provider_test_raw_name.bzl",
            r#"
def _f():
    _p, raw = provider(init = lambda: {})
    return str(raw) + "|" + type(raw)
result = _f()
"#,
        )
        .unwrap();
        assert_eq!(
            string_global(&m, "result"),
            "<unknown object com.google.devtools.build.lib.packages.StarlarkProvider$RawConstructor>|RawConstructor"
        );
    }

    #[test]
    fn exported_provider_accessors() {
        let k = key("//foo:bar.bzl", "prov");
        let provider: P = builder().build_exported(k.clone());
        assert!(provider.is_exported());
        assert_eq!(provider.get_name(), "prov");
        assert_eq!(provider.get_printable_name(), "prov");
        assert_eq!(
            provider.get_error_message_for_unknown_field("foo"),
            "'prov' value has no field or method 'foo'"
        );
        assert!(provider.is_immutable());
        assert_eq!(provider.to_string(), "<provider>");
        assert_eq!(provider.get_key(), &k);
    }

    #[test]
    fn basic_instantiation() {
        let m = exec_bzl(
            "provider_test_basic.bzl",
            r#"
P = provider()
x = P(a = 1, b = 2, c = 3)
result = repr(x) + "|" + str(dir(x)) + "|" + type(x) + "|" + str(x == P(c = 3, b = 2, a = 1))
"#,
        )
        .unwrap();
        assert_eq!(
            string_global(&m, "result"),
            "struct(a = 1, b = 2, c = 3)|[\"a\", \"b\", \"c\"]|struct|True"
        );
    }

    const INIT_BC: &str = r#"
def initBC(*args, **kwargs):
    if args:
        fail("unexpected positional arguments")
    if len(kwargs) != 1 or "a" not in kwargs:
        fail("expected a single `a` argument")
    a = kwargs["a"]
    return {"a": a, "b": a * 2, "c": a * 3}
"#;

    #[test]
    fn instantiation_with_init() {
        let m = exec_bzl(
            "provider_test_init.bzl",
            &format!("{INIT_BC}\nP, _raw = provider(init = initBC)\nresult = repr(P(a = 1))\n"),
        )
        .unwrap();
        assert_eq!(string_global(&m, "result"), "struct(a = 1, b = 2, c = 3)");
    }

    #[test]
    fn instantiation_with_init_signature_mismatch_fails() {
        let e = error_message(exec_bzl(
            "provider_test_init_mismatch.bzl",
            &format!("{INIT_BC}\nP, _raw = provider(init = initBC)\nP(a = 1, b = 2, c = 3)\n"),
        ));
        assert!(e.contains("expected a single `a` argument"), "{e}");
    }

    #[test]
    fn instantiation_with_init_return_type_mismatch_fails() {
        let e = error_message(exec_bzl(
            "provider_test_init_return.bzl",
            "P, _raw = provider(init = lambda **kw: \"invalid\")\nP(a = 1, b = 2, c = 3)\n",
        ));
        assert!(
            e.contains("got string for 'return value of provider init()', want dict"),
            "{e}"
        );
    }

    #[test]
    fn instantiation_with_failing_init_fails() {
        let e = error_message(exec_bzl(
            "provider_test_init_fails.bzl",
            "def failingInit(**kw):\n    fail(\"failingInit fails\")\nP, _raw = provider(init = failingInit)\nP(a = 1)\n",
        ));
        assert!(e.contains("failingInit fails"), "{e}");
    }

    #[test]
    fn raw_constructor_bypasses_init() {
        let m = exec_bzl(
            "provider_test_raw.bzl",
            "def init(**kw):\n    fail(\"init called\")\nP, raw = provider(init = init)\nx = raw(a = 1, b = 2, c = 3)\nresult = repr(x) + \"|\" + str(x == raw(a = 1, b = 2, c = 3))\n",
        )
        .unwrap();
        assert_eq!(
            string_global(&m, "result"),
            "struct(a = 1, b = 2, c = 3)|True"
        );
    }

    #[test]
    fn basic_instantiation_with_schema_with_some_fields_unset() {
        let m = exec_bzl(
            "provider_test_schema_some.bzl",
            "P = provider(fields = [\"a\", \"b\", \"c\"])\nresult = repr(P(a = 1)) + str(dir(P(a = 1)))\n",
        )
        .unwrap();
        assert_eq!(string_global(&m, "result"), "struct(a = 1)[\"a\"]");
    }

    #[test]
    fn basic_instantiation_with_schema_with_all_fields_set() {
        let m = exec_bzl(
            "provider_test_schema_all.bzl",
            "P = provider(fields = [\"c\", \"a\", \"b\"])\nresult = repr(P(c = 3, a = 1, b = 2))\n",
        )
        .unwrap();
        assert_eq!(string_global(&m, "result"), "struct(a = 1, b = 2, c = 3)");
    }

    #[test]
    fn basic_instantiation_with_documented_schema() {
        let m = exec_bzl(
            "provider_test_schema_doc.bzl",
            "P = provider(fields = {\"a\": \"Parameter a\", \"b\": \"Parameter b\", \"c\": \"Parameter c\"})\nresult = repr(P(a = 1))\n",
        )
        .unwrap();
        assert_eq!(string_global(&m, "result"), "struct(a = 1)");
    }

    #[test]
    fn schema_disallows_unexpected_fields() {
        let e = error_message(exec_bzl(
            "provider_test_schema_unexpected.bzl",
            "P = provider(fields = [\"a\", \"b\"])\nP(a = 1, b = 2, c = 3)\n",
        ));
        assert!(
            e.contains("got unexpected field 'c' in call to instantiate provider P"),
            "{e}"
        );
    }

    #[test]
    fn documented_schema_disallows_unexpected_fields() {
        let e = error_message(exec_bzl(
            "provider_test_doc_schema_unexpected.bzl",
            "P = provider(fields = {\"a\": \"Parameter a\", \"b\": \"Parameter b\"})\nP(a = 1, b = 2, c = 3)\n",
        ));
        assert!(
            e.contains("got unexpected field 'c' in call to instantiate provider"),
            "{e}"
        );
    }

    #[test]
    fn schema_enforced_on_raw_constructor() {
        let e = error_message(exec_bzl(
            "provider_test_schema_raw.bzl",
            "P, raw = provider(fields = [\"a\", \"b\"], init = lambda: {})\nraw(a = 1, b = 2, c = 3)\n",
        ));
        assert!(
            e.contains("got unexpected field 'c' in call to instantiate provider"),
            "{e}"
        );
    }

    #[test]
    fn schema_enforced_on_init() {
        let e = error_message(exec_bzl(
            "provider_test_schema_init.bzl",
            &format!(
                "{INIT_BC}\nP, _raw = provider(fields = [\"a\", \"b\"], init = initBC)\nP(a = 1)\n"
            ),
        ));
        assert!(
            e.contains("got unexpected field 'c' in call to instantiate provider"),
            "{e}"
        );
    }

    #[test]
    fn documented_provider_get_documentation() {
        let provider: P = builder()
            .set_documentation("My doc string".to_owned())
            .build_with_identity_token(token(0));
        assert_eq!(provider.get_documentation(), Some("My doc string"));
    }

    #[test]
    fn undocumented_provider_get_documentation() {
        let provider: P = builder().build_with_identity_token(token(0));
        assert_eq!(provider.get_documentation(), None);
    }

    #[test]
    fn schemaless_provider_get_fields() {
        let provider: P = builder().build_with_identity_token(token(0));
        assert!(provider.get_fields().is_none());
    }

    #[test]
    fn schemaless_provider_get_schema() {
        let provider: P = builder().build_with_identity_token(token(0));
        assert!(provider.get_schema().is_none());
    }

    #[test]
    fn provider_with_undocumented_schema_get_fields() {
        let provider: P = builder()
            .set_schema(vec!["c".to_owned(), "a".to_owned(), "b".to_owned()])
            .build_with_identity_token(token(0));
        assert_eq!(provider.get_fields().unwrap(), ["a", "b", "c"]);
    }

    #[test]
    fn provider_with_undocumented_schema_get_schema() {
        let provider: P = builder()
            .set_schema(vec!["c".to_owned(), "a".to_owned(), "b".to_owned()])
            .build_with_identity_token(token(0));
        assert_eq!(
            provider.get_schema().unwrap(),
            [
                ("c".to_owned(), None),
                ("a".to_owned(), None),
                ("b".to_owned(), None)
            ]
        );
    }

    #[test]
    fn provider_with_documented_schema_get_fields() {
        let provider: P = builder()
            .set_documented_schema(vec![
                ("c".to_owned(), "Parameter c".to_owned()),
                ("a".to_owned(), "Parameter a".to_owned()),
                ("b".to_owned(), "Parameter b".to_owned()),
            ])
            .build_with_identity_token(token(0));
        assert_eq!(provider.get_fields().unwrap(), ["a", "b", "c"]);
    }

    #[test]
    fn provider_with_documented_schema_get_schema() {
        let provider: P = builder()
            .set_documented_schema(vec![
                ("c".to_owned(), "Parameter c".to_owned()),
                ("a".to_owned(), "Parameter a".to_owned()),
                ("b".to_owned(), "Parameter b".to_owned()),
            ])
            .build_with_identity_token(token(0));
        assert_eq!(
            provider.get_schema().unwrap(),
            [
                ("c".to_owned(), Some("Parameter c".to_owned())),
                ("a".to_owned(), Some("Parameter a".to_owned())),
                ("b".to_owned(), Some("Parameter b".to_owned()))
            ]
        );
    }

    #[test]
    #[ignore = "needs depset fields of schemaful instances to be stored as NestedSet (optimizeField is not ported)"]
    fn schemaful_provider_with_depset() {}

    #[test]
    #[ignore = "needs StarlarkProvider.optimizeField (depset type predictor, not ported)"]
    fn schemaful_provider_optimize_field() {}

    #[test]
    fn schemaful_provider_mutable() {
        let m = exec_bzl(
            "provider_test_mutable.bzl",
            "P = provider(fields = [\"a\"])\nx = P(a = [\"x\"])\nx.a.append(\"y\")\nresult = repr(x.a)\n",
        )
        .unwrap();
        assert_eq!(string_global(&m, "result"), "[\"x\", \"y\"]");
    }

    #[test]
    fn schemaful_provider_immutable() {
        let m = exec_bzl(
            "provider_test_immutable.bzl",
            "P = provider(fields = [\"a\"])\nx = P(a = [\"x\"])\n",
        )
        .unwrap();
        let x = m.get("x").unwrap();
        let info = StarlarkInfo::from_value(x.value()).unwrap();
        let list = info.get_field("a").unwrap();
        assert_eq!(list.to_repr(), "[\"x\"]");
        // The fields of the frozen instance are frozen.
        assert!(list.unpack_frozen().is_some());
        assert!(blaze_starlark::structs::is_immutable(x.value()));
    }

    #[test]
    #[ignore = "needs a native-constructed depset of a typed element class (Depset.of(String.class, ...))"]
    fn schemaful_provider_with_depset_is_immutable() {}

    #[test]
    #[ignore = "needs a native-constructed depset of a typed element class (Depset.of(String.class, ...))"]
    fn schemaful_provider_with_depset_becomes_immutable() {}

    #[test]
    #[ignore = "needs StarlarkInfo.unsafeOptimizeMemoryLayout (memory optimisation, not ported)"]
    fn schemaful_provider_optimised_immutable() {}

    #[test]
    fn provider_equals() {
        let prov_foo_a1: P = builder().build_exported(key("//:foo.bzl", "provA"));
        let prov_foo_a2: P = builder().build_exported(key("//:foo.bzl", "provA"));
        let prov_foo_b: P = builder().build_exported(key("//:foo.bzl", "provB"));
        let prov_bar_a: P = builder().build_exported(key("//:bar.bzl", "provA"));
        let prov_bar_b: P = builder().build_exported(key("//:bar.bzl", "provB"));
        let prov_unexported1: P = builder().build_with_identity_token(token(0));
        let prov_unexported2: P = builder().build_with_identity_token(token(1));
        let groups: Vec<Vec<&P>> = vec![
            vec![&prov_foo_a1, &prov_foo_a2],
            vec![&prov_foo_b],
            vec![&prov_bar_a, &prov_bar_a],
            vec![&prov_bar_b],
            vec![&prov_unexported1, &prov_unexported1],
            vec![&prov_unexported2],
        ];
        for (i, gi) in groups.iter().enumerate() {
            for (j, gj) in groups.iter().enumerate() {
                for a in gi {
                    for b in gj {
                        assert_eq!(
                            a.key_or_identity_token() == b.key_or_identity_token(),
                            i == j,
                            "groups {i} and {j}"
                        );
                    }
                }
            }
        }
    }

    // Behaviour recorded from the oracle (Bazel 9.3.2) on .bzl snippets.

    #[test]
    fn provider_repr_type_and_identity() {
        let m = exec_bzl(
            "provider_test_oracle_1.bzl",
            r#"
P = provider()
U = provider(doc = "d", fields = ["b", "a"])
def _init(a, b = 2):
    return {"a": a, "b": b}
I, mkI = provider(init = _init)
result = "|".join([
    type(P), repr(P), str(P), str(P == P), str(P == U), str({P: 1}[P]),
    repr(I(1)), repr(mkI(a = 5)), type(mkI),
])
"#,
        )
        .unwrap();
        assert_eq!(
            string_global(&m, "result"),
            "Provider|<provider>|<provider>|True|False|1|struct(a = 1, b = 2)|struct(a = 5)|RawConstructor"
        );
    }

    #[test]
    fn calling_an_unexported_provider_creates_an_instance() {
        let m = exec_bzl(
            "provider_test_oracle_2.bzl",
            "def f():\n    p = provider()\n    return p(a = 1)\nresult = repr(f())\n",
        )
        .unwrap();
        assert_eq!(string_global(&m, "result"), "struct(a = 1)");
    }

    #[test]
    fn unexported_provider_is_unhashable() {
        let e = error_message(exec_bzl(
            "provider_test_oracle_3.bzl",
            "def f():\n    p = provider()\n    return {p: 1}\nf()\n",
        ));
        assert!(e.contains("unhashable type: 'Provider'"), "{e}");
    }

    #[test]
    fn unexpected_fields_plural() {
        let e = error_message(exec_bzl(
            "provider_test_oracle_4.bzl",
            "P = provider(fields = [\"a\"])\nP(b = 1, c = 2)\n",
        ));
        assert!(
            e.contains("got unexpected fields 'b', 'c' in call to instantiate provider P"),
            "{e}"
        );
    }

    #[test]
    fn positional_arguments_rejected() {
        let e = error_message(exec_bzl(
            "provider_test_oracle_5.bzl",
            "P = provider()\nP(1)\n",
        ));
        assert!(e.contains("P: unexpected positional arguments"), "{e}");
        let e = error_message(exec_bzl(
            "provider_test_oracle_6.bzl",
            "P, R = provider(init = lambda: {})\nR(1)\n",
        ));
        assert!(
            e.contains("<raw constructor for P>: unexpected positional arguments"),
            "{e}"
        );
    }

    #[test]
    fn provider_param_type_errors() {
        for (file, src, want) in [
            (
                "provider_test_oracle_7.bzl",
                "P = provider(fields = 1)\n",
                "in call to provider(), parameter 'fields' got value of type 'int', want 'sequence, dict, or NoneType'",
            ),
            (
                "provider_test_oracle_8.bzl",
                "P = provider(init = 1)\n",
                "in call to provider(), parameter 'init' got value of type 'int', want 'callable or NoneType'",
            ),
            (
                "provider_test_oracle_9.bzl",
                "P = provider(doc = 1)\n",
                "in call to provider(), parameter 'doc' got value of type 'int', want 'string or NoneType'",
            ),
            (
                "provider_test_oracle_10.bzl",
                "P = provider(fields = [1])\n",
                "at index 0 of fields, got element of type int, want string",
            ),
            (
                "provider_test_oracle_11.bzl",
                "P = provider(fields = {\"a\": 1})\n",
                "got dict<string, int> for 'fields', want dict<string, string>",
            ),
            (
                "provider_test_oracle_12.bzl",
                "def _i():\n    return 1\nP, _ = provider(init = _i)\nP()\n",
                "got int for 'return value of provider init()', want dict",
            ),
            (
                "provider_test_oracle_13.bzl",
                "def _i():\n    return {1: 2}\nP, _ = provider(init = _i)\nP()\n",
                "got dict<int, int> for 'return value of provider init()', want dict<string, unknown>",
            ),
            (
                "provider_test_oracle_14.bzl",
                "P = provider()\nQ = provider()\nP(a = 1) + Q(b = 1)\n",
                "Cannot use '+' operator on instances of different providers (P and Q)",
            ),
        ] {
            let e = error_message(exec_bzl(file, src));
            assert!(e.contains(want), "{file}: {e}");
        }
    }

    #[test]
    fn provider_view_of_values() {
        let m = exec_bzl(
            "provider_test_view.bzl",
            "P = provider()\nD = DefaultInfo\nS = struct\nN = 1\n",
        )
        .unwrap();
        let p = m.get("P").unwrap();
        assert_eq!(
            provider_view(p.value()),
            Some(ProviderView {
                exported: true,
                key: Some(key("//pkg:provider_test_view.bzl", "P")),
            })
        );
        let d = m.get("D").unwrap();
        assert_eq!(
            provider_view(d.value()).unwrap().key,
            Some(ProviderKey::Builtin {
                name: "DefaultInfo".to_owned(),
                provider_class: "DefaultInfo".to_owned(),
            })
        );
        let s = m.get("S").unwrap();
        assert_eq!(
            provider_view(s.value()).unwrap().key.unwrap().to_string(),
            "struct"
        );
        assert_eq!(provider_view(m.get("N").unwrap().value()), None);
    }

    // Ported from analysis/starlark StarlarkRuleClassFunctionsTest (the provider tests).

    fn provider_of<'v>(v: Value<'v>) -> &'v StarlarkProvider<'v> {
        StarlarkProvider::from_value(v).expect("a StarlarkProvider")
    }

    fn info_provider<'v>(v: Value<'v>) -> Value<'v> {
        StarlarkInfo::from_value(v)
            .expect("a StarlarkInfo")
            .provider()
    }

    #[test]
    fn declared_providers() {
        let m = exec_bzl(
            "srcf_declared_providers.bzl",
            "data = provider()\nd = data(x = 1, y ='abc')\nd_x = d.x\nd_y = d.y\n",
        )
        .unwrap();
        assert_eq!(m.get("d_x").unwrap().value().unpack_i32(), Some(1));
        assert_eq!(string_global(&m, "d_y"), "abc");
        let data = m.get("data").unwrap();
        let d = m.get("d").unwrap();
        assert!(info_provider(d.value()).equals(data.value()).unwrap());
        let p = provider_of(data.value());
        assert!(p.is_exported());
        assert_eq!(p.get_printable_name(), "data");
        assert_eq!(
            p.get_key(),
            &key("//pkg:srcf_declared_providers.bzl", "data")
        );
    }

    #[test]
    fn declared_provider_documentation() {
        let src = [
            "UndocumentedInfo = provider()",
            "DocumentedInfo = provider(doc = '''",
            "    My documented provider",
            "",
            "    Details''')",
            // Note fields below are not alphabetized
            "SchemafulWithoutDocsInfo = provider(fields = ['b', 'a'])",
            "SchemafulWithDocsInfo = provider(fields = {'b': 'Field b', 'a': 'Field\\n    a'})",
        ]
        .join("\n");
        let m = exec_bzl("srcf_declared_provider_documentation.bzl", &src).unwrap();
        let undocumented = m.get("UndocumentedInfo").unwrap();
        let documented = m.get("DocumentedInfo").unwrap();
        let without_docs = m.get("SchemafulWithoutDocsInfo").unwrap();
        let with_docs = m.get("SchemafulWithDocsInfo").unwrap();
        assert_eq!(provider_of(undocumented.value()).get_documentation(), None);
        assert_eq!(
            provider_of(documented.value()).get_documentation(),
            Some("My documented provider\n\nDetails")
        );
        assert_eq!(
            provider_of(without_docs.value()).get_schema().unwrap(),
            [("b".to_owned(), None), ("a".to_owned(), None)]
        );
        assert_eq!(
            provider_of(with_docs.value()).get_schema().unwrap(),
            [
                ("b".to_owned(), Some("Field b".to_owned())),
                ("a".to_owned(), Some("Field\na".to_owned()))
            ]
        );
    }

    #[test]
    fn declared_providers_with_init() {
        let src = [
            "def _data_init(x, y = 'abc'):",
            "    return {'x': x, 'y': y}",
            "data, _new_data = provider(init = _data_init)",
            "d1 = data(x = 1)  # normal provider constructor",
            "d1_x = d1.x",
            "d1_y = d1.y",
            "d2 = data(1, 'def')  # normal provider constructor invoked with positional arguments",
            "d2_x = d2.x",
            "d2_y = d2.y",
            "d3 = _new_data(x = 2, y = 'xyz')  # raw constructor",
            "d3_x = d3.x",
            "d3_y = d3.y",
            "raw = _new_data",
        ]
        .join("\n");
        let m = exec_bzl("srcf_declared_providers_with_init.bzl", &src).unwrap();
        assert_eq!(m.get("d1_x").unwrap().value().unpack_i32(), Some(1));
        assert_eq!(string_global(&m, "d1_y"), "abc");
        assert_eq!(m.get("d2_x").unwrap().value().unpack_i32(), Some(1));
        assert_eq!(string_global(&m, "d2_y"), "def");
        assert_eq!(m.get("d3_x").unwrap().value().unpack_i32(), Some(2));
        assert_eq!(string_global(&m, "d3_y"), "xyz");
        let data = m.get("data").unwrap();
        let raw = m.get("raw").unwrap();
        assert_eq!(provider_view(raw.value()), None);
        let p = provider_of(data.value());
        assert_eq!(p.get_init().unwrap().function_name(), Some("_data_init"));
        for d in ["d1", "d2", "d3"] {
            let d = m.get(d).unwrap();
            assert!(info_provider(d.value()).equals(data.value()).unwrap());
        }
        assert!(p.is_exported());
        assert_eq!(p.get_printable_name(), "data");
        assert_eq!(
            p.get_key(),
            &key("//pkg:srcf_declared_providers_with_init.bzl", "data")
        );
    }

    const DATA_FAILING_INIT: &str = "def _data_failing_init(x):\n    fail('_data_failing_init fails')\ndata, _new_data = provider(init = _data_failing_init)\n";

    #[test]
    fn declared_providers_with_failing_init_raw_constructor_succeeds() {
        let src = format!("{DATA_FAILING_INIT}d = _new_data(x = 1)  # raw constructor\n");
        let m = exec_bzl("srcf_failing_init_raw.bzl", &src).unwrap();
        let data = m.get("data").unwrap();
        let d = m.get("d").unwrap();
        assert!(info_provider(d.value()).equals(data.value()).unwrap());
    }

    #[test]
    fn declared_providers_with_failing_init_normal_constructor_fails() {
        let src = format!("{DATA_FAILING_INIT}d = data(x = 1)  # normal constructor\n");
        let e = error_message(exec_bzl("srcf_failing_init_normal.bzl", &src));
        assert!(e.contains("_data_failing_init fails"), "{e}");
    }

    #[test]
    fn declared_providers_with_init_returning_invalid_type_normal_constructor_fails() {
        let src = "def _data_invalid_init(x):\n    return 'INVALID'\n\
                   data, _new_data = provider(init = _data_invalid_init)\n\
                   d = data(x = 1)  # normal constructor\n";
        let e = error_message(exec_bzl("srcf_init_invalid_type.bzl", src));
        assert!(
            e.contains("got string for 'return value of provider init()', want dict"),
            "{e}"
        );
    }

    #[test]
    fn declared_providers_with_init_returning_invalid_dict_normal_constructor_fails() {
        let src = "def _data_invalid_init(x):\n    return {('x', 'x', 'x'): x}\n\
                   data, _new_data = provider(init = _data_invalid_init)\n\
                   d = data(x = 1)  # normal constructor\n";
        let e = error_message(exec_bzl("srcf_init_invalid_dict.bzl", src));
        assert!(
            e.contains("got dict<tuple, int> for 'return value of provider init()'"),
            "{e}"
        );
    }

    #[test]
    fn declared_providers_with_init_returning_unexpected_fields_normal_constructor_fails() {
        let src = "def _data_unexpected_fields_init(x):\n    return {'x': x, 'y': x * 2}\n\
                   data, _new_data = provider(fields = ['x'], init = _data_unexpected_fields_init)\n\
                   d = data(x = 1)  # normal constructor\n";
        let e = error_message(exec_bzl("srcf_init_unexpected_fields.bzl", src));
        assert!(
            e.contains("got unexpected field 'y' in call to instantiate provider data"),
            "{e}"
        );
    }

    #[test]
    fn declared_providers_concat_success() {
        let src = "data = provider()\ndx = data(x = 1)\ndy = data(y = 'abc')\ndxy = dx + dy\n\
                   x = dxy.x\ny = dxy.y\n";
        let m = exec_bzl("srcf_concat_success.bzl", src).unwrap();
        assert_eq!(m.get("x").unwrap().value().unpack_i32(), Some(1));
        assert_eq!(string_global(&m, "y"), "abc");
        let data = m.get("data").unwrap();
        for d in ["dx", "dy"] {
            let d = m.get(d).unwrap();
            assert!(info_provider(d.value()).equals(data.value()).unwrap());
        }
    }

    #[test]
    fn declared_providers_with_init_concat_success() {
        let src = "def _data_init(x):\n    return {'x': x}\n\
                   data, _new_data = provider(init = _data_init)\n\
                   dx = data(x = 1)  # normal constructor\n\
                   dy = _new_data(y = 'abc')  # raw constructor\n\
                   dxy = dx + dy\nx = dxy.x\ny = dxy.y\n";
        let m = exec_bzl("srcf_init_concat_success.bzl", src).unwrap();
        assert_eq!(m.get("x").unwrap().value().unpack_i32(), Some(1));
        assert_eq!(string_global(&m, "y"), "abc");
        let data = m.get("data").unwrap();
        for d in ["dx", "dy"] {
            let d = m.get(d).unwrap();
            assert!(info_provider(d.value()).equals(data.value()).unwrap());
        }
    }

    #[test]
    fn declared_providers_concat_error() {
        let src = "data1 = provider()\ndata2 = provider()\n\
                   d1 = data1(x = 1)\nd2 = data2(y = 2)\nd = d1 + d2\n";
        let e = error_message(exec_bzl("srcf_concat_error.bzl", src));
        assert!(
            e.contains(
                "Cannot use '+' operator on instances of different providers (data1 and data2)"
            ),
            "{e}"
        );
    }

    #[test]
    fn declared_providers_with_fields_concat_success() {
        let src = "data = provider(fields=['f1', 'f2'])\nd1 = data(f1 = 4)\nd2 = data(f2 = 5)\n\
                   d3 = d1 + d2\nf1 = d3.f1\nf2 = d3.f2\n";
        let m = exec_bzl("srcf_fields_concat_success.bzl", src).unwrap();
        assert_eq!(m.get("f1").unwrap().value().unpack_i32(), Some(4));
        assert_eq!(m.get("f2").unwrap().value().unpack_i32(), Some(5));
    }

    #[test]
    fn declared_providers_with_fields_concat_error() {
        let src = "data1 = provider(fields=['f1', 'f2'])\ndata2 = provider(fields=['f3'])\n\
                   d1 = data1(f1=1, f2=2)\nd2 = data2(f3=3)\nd = d1 + d2\n";
        let e = error_message(exec_bzl("srcf_fields_concat_error.bzl", src));
        assert!(
            e.contains(
                "Cannot use '+' operator on instances of different providers (data1 and data2)"
            ),
            "{e}"
        );
    }

    #[test]
    fn declared_providers_with_overlapping_fields_concat_error() {
        let src =
            "data = provider(fields=['f1', 'f2'])\nd1 = data(f1 = 4)\nd2 = data(f1 = 5)\nd1 + d2\n";
        let e = error_message(exec_bzl("srcf_overlapping_concat_error.bzl", src));
        assert!(
            e.contains("cannot add struct instances with common field 'f1'"),
            "{e}"
        );
    }

    #[test]
    fn structs_as_declared_providers_test() {
        let m = exec_bzl("srcf_structs_as_providers.bzl", "data = struct(x = 1)\n").unwrap();
        let data = m.get("data").unwrap();
        let view = provider_view(info_provider(data.value())).unwrap();
        assert!(view.exported);
        assert_eq!(
            view.key,
            Some(ProviderKey::Builtin {
                name: "struct".to_owned(),
                provider_class: "StarlarkInfo".to_owned(),
            })
        );
    }

    #[test]
    fn declared_providers_doc() {
        exec_bzl(
            "srcf_declared_providers_doc.bzl",
            "data1 = provider(doc='foo')\n",
        )
        .unwrap();
    }

    #[test]
    fn declared_providers_bad_type_for_doc() {
        let e = error_message(exec_bzl("srcf_bad_type_for_doc.bzl", "provider(doc = 1)\n"));
        assert!(
            e.contains("got value of type 'int', want 'string or NoneType'"),
            "{e}"
        );
    }

    #[test]
    fn provider_with_fields() {
        let src = "p = provider(fields = ['x', 'y'])\np1 = p(x = 1, y = 2)\nx = p1.x\ny = p1.y\n";
        let m = exec_bzl("srcf_provider_with_fields.bzl", src).unwrap();
        let p = m.get("p").unwrap();
        let p1 = m.get("p1").unwrap();
        assert!(info_provider(p1.value()).equals(p.value()).unwrap());
        assert_eq!(m.get("x").unwrap().value().unpack_i32(), Some(1));
        assert_eq!(m.get("y").unwrap().value().unpack_i32(), Some(2));
    }

    #[test]
    fn provider_with_fields_dict() {
        let src = "p = provider(fields = { 'x' : 'I am x', 'y' : 'I am y'})\n\
                   p1 = p(x = 1, y = 2)\nx = p1.x\ny = p1.y\n";
        let m = exec_bzl("srcf_provider_with_fields_dict.bzl", src).unwrap();
        let p = m.get("p").unwrap();
        let p1 = m.get("p1").unwrap();
        assert!(info_provider(p1.value()).equals(p.value()).unwrap());
        assert_eq!(m.get("x").unwrap().value().unpack_i32(), Some(1));
        assert_eq!(m.get("y").unwrap().value().unpack_i32(), Some(2));
    }

    #[test]
    fn provider_with_fields_optional() {
        let src = "p = provider(fields = ['x', 'y'])\np1 = p(y = 2)\ny = p1.y\n";
        let m = exec_bzl("srcf_provider_with_fields_optional.bzl", src).unwrap();
        let p = m.get("p").unwrap();
        let p1 = m.get("p1").unwrap();
        assert!(info_provider(p1.value()).equals(p.value()).unwrap());
        assert_eq!(m.get("y").unwrap().value().unpack_i32(), Some(2));
    }

    #[test]
    #[ignore = "needs blaze_starlark StarlarkInfo to report Bazel's unknown-field message; starlark-rust reports its own"]
    fn provider_with_fields_optional_error() {
        let src = "p = provider(fields = ['x', 'y'])\np1 = p(y = 2)\nx = p1.x\n";
        let e = error_message(exec_bzl(
            "srcf_provider_with_fields_optional_error.bzl",
            src,
        ));
        assert!(e.contains(" 'p' value has no field or method 'x'"), "{e}");
    }

    #[test]
    fn provider_with_extra_fields_error() {
        let src = "p = provider(fields = ['x', 'y'])\np1 = p(x = 1, y = 2, z = 3)\n";
        let e = error_message(exec_bzl("srcf_provider_with_extra_fields_error.bzl", src));
        assert!(
            e.contains("got unexpected field 'z' in call to instantiate provider p"),
            "{e}"
        );
    }

    #[test]
    fn provider_with_empty_fields_error() {
        let src = "p = provider(fields = [])\np1 = p(x = 1, y = 2, z = 3)\n";
        let e = error_message(exec_bzl("srcf_provider_with_empty_fields_error.bzl", src));
        assert!(
            e.contains("got unexpected fields 'x', 'y', 'z' in call to instantiate provider p"),
            "{e}"
        );
    }

    #[test]
    #[ignore = "starlark-rust rejects a name repeated through **kwargs (Arguments::names_map) before the provider's argument processor sees it"]
    fn provider_with_duplicate_fields_error() {
        let src = "p = provider(fields = ['a', 'b'])\np(a = 1, b = 2, **dict(b = 3))\n";
        let e = error_message(exec_bzl(
            "srcf_provider_with_duplicate_fields_error.bzl",
            src,
        ));
        assert!(
            e.contains("got multiple values for parameter b in call to instantiate provider p"),
            "{e}"
        );
    }

    #[test]
    fn test_type_of_struct() {
        let src = "p = type(struct)\ns = type(struct())\n";
        let m = exec_bzl("srcf_type_of_struct.bzl", src).unwrap();
        assert_eq!(string_global(&m, "p"), "Provider");
        assert_eq!(string_global(&m, "s"), "struct");
    }
}
