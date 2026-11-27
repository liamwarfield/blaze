//! Port of `packages/StarlarkNativeModule.java` (with the parameter declarations of
//! `starlarkbuildapi/StarlarkNativeModuleApi.java`): the non-rule members of `native`, which
//! are also BUILD-file toplevels.

use std::fmt;
use std::sync::Arc;

use allocative::Allocative;
use blaze_cmdline::Label;
use blaze_cmdline::PackageIdentifier;
use blaze_events::EventHandler;
use blaze_starlark::Location;
use blaze_starlark::Phase;
use blaze_starlark::ThreadContext;
use blaze_starlark::label::LabelValue;
use indexmap::IndexMap;
use starlark::any::ProvidesStaticType;
use starlark::environment::GlobalsBuilder;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::Heap;
use starlark::values::NoSerialize;
use starlark::values::StarlarkValue;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::list::AllocList;
use starlark::values::none::NoneOr;
use starlark::values::none::NoneType;
use starlark::values::starlark_value;
use starlark::values::tuple::AllocTuple;

use crate::build::build_globals::check_sequence_or_none_param;
use crate::build::build_globals::check_sequence_param;
use crate::build::build_globals::convert_label_list;
use crate::build::build_globals::convert_string_list;
use crate::build::caller_location;
use crate::build::eval_err;
use crate::exceptions::DetailedExitCode;
use crate::exceptions::PackageLoadingCode;
use crate::package::GlobberError;
use crate::package::GlobberOperation;
use crate::package::Package;
use crate::package::PackageBuilderError;
use crate::package::RuleVisibility;
use crate::package::TargetDefinitionContext;
use crate::target::Rule;
use crate::target::RuleOrMacroInstance;
use crate::types::AttrValue;
use crate::types::BuildTypeSelectorList;
use crate::types::LabelConverter;
use crate::types::SelectorList;
use crate::types::SelectorValue;
use crate::types::TriState;
use crate::types::Type;
use crate::types::type_::alloc_immutable_dict;
use crate::types::type_::alloc_immutable_list;

/// Java `Globber.Operation.toString()` (the enum constant name).
fn operation_name(op: GlobberOperation) -> &'static str {
    match op {
        GlobberOperation::Files => "FILES",
        GlobberOperation::FilesAndDirs => "FILES_AND_DIRS",
        GlobberOperation::Subpackages => "SUBPACKAGES",
    }
}

/// Runs a glob through the package's globber; I/O errors are reported on the package and
/// yield no matches.
// Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:119 StarlarkNativeModule.runGlobOperation
fn run_glob_operation(
    eval: &Evaluator<'_, '_, '_>,
    what: &str,
    includes: &[String],
    excludes: &[String],
    operation: GlobberOperation,
    allow_empty: bool,
) -> starlark::Result<Vec<String>> {
    // PORT: the cpu-bound semaphore is not ported (single build per process). The payload
    // borrow is released while globbing: a Skyframe-backed globber requests dependencies.
    let globber = TargetDefinitionContext::from_or_fail_allow_build_only(eval, what)
        .map_err(eval_err)?
        .get_globber()
        .cloned()
        .expect("NullPointerException: getGlobber()");
    let result = globber
        .run_async(includes, excludes, operation, allow_empty)
        .and_then(|token| globber.fetch_unsorted(token));
    match result {
        Ok(matches) => Ok(matches),
        Err(GlobberError::Io(message)) => {
            let error_message = format!(
                "error globbing [{}]{} op={}: {}",
                includes.join(", "),
                if excludes.is_empty() {
                    String::new()
                } else {
                    format!(" - [{}]", excludes.join(", "))
                },
                operation_name(operation),
                message
            );
            let loc = caller_location(eval);
            // PORT: Java picks the code from the exception class (DetailedIOException ->
            // its own code, FileSymlinkException -> EVAL_GLOBS_SYMLINK_ERROR); Globber
            // errors carry only the message, so every I/O error is GLOB_IO_EXCEPTION. Blaze's
            // only DetailedIOException (the remote repo contents cache's lost file) comes from
            // reading file contents, which globbing (stat, readdir) never does.
            let code = PackageLoadingCode::GlobIoException;
            let error = Package::error(loc.as_ref(), &error_message, code);
            let mut pkg_builder =
                TargetDefinitionContext::from_or_fail_allow_build_only(eval, what)
                    .map_err(eval_err)?;
            pkg_builder.get_local_event_handler().handle(&error);
            pkg_builder.set_io_exception(
                message,
                error_message.clone(),
                DetailedExitCode::package_loading(error_message, code),
            );
            Ok(Vec::new())
        }
        Err(GlobberError::BadGlob(e)) => Err(eval_err(e)),
        // Java propagates InterruptedException out of the evaluation. starlark-rust has no such
        // error: the evaluation stops as a cancelled one (blaze_starlark `EvalErrorKind::Cancelled`,
        // which PackageFactory.executeBuildFile turns into an interrupted package).
        Err(GlobberError::Interrupted) => Err(eval_err("Evaluation cancelled")),
    }
}

/// Java `Starlark.UNBOUND`-aware `allow_empty` of `glob()`.
fn glob_allow_empty(
    eval: &Evaluator<'_, '_, '_>,
    allow_empty_argument: Option<Value<'_>>,
) -> starlark::Result<bool> {
    match allow_empty_argument {
        None => Ok(!ThreadContext::get(eval)
            .map_err(starlark::Error::new_other)?
            .semantics
            .incompatible_disallow_empty_glob),
        Some(v) => match v.unpack_bool() {
            Some(b) => Ok(b),
            None => Err(eval_err(format!(
                "expected boolean for argument `allow_empty`, got `{}`",
                v.to_str()
            ))),
        },
    }
}

/// Java `excludeDirectories.signum() != 0` for the `StarlarkInt` parameter.
fn exclude_directories_nonzero(v: Value<'_>) -> starlark::Result<bool> {
    if let Some(i) = v.unpack_i32() {
        return Ok(i != 0);
    }
    if v.get_type() == "int" {
        // A big integer is never zero.
        return Ok(true);
    }
    Err(eval_err(format!(
        "in call to glob(), parameter 'exclude_directories' got value of type '{}', want 'int'",
        v.get_type()
    )))
}

/// Java `ModuleExtensionEvalStarlarkThreadContext` sets `ExistingRulesShouldBeNoOp`: module
/// extensions may call `native.existing_rule(s)` (legacy WORKSPACE macros) and get nothing.
// PORT: the thread-local is the module-extension phase of the thread context.
fn existing_rules_should_be_no_op(eval: &Evaluator<'_, '_, '_>) -> bool {
    ThreadContext::get(eval).is_ok_and(|c| c.phase == Phase::ModuleExtension)
}

/// Whether two visibilities are the same object (Java `!=` on `RuleVisibility`): the `PUBLIC`
/// and `PRIVATE` constants, or the same `PackageGroupsRuleVisibility` instance.
fn same_visibility_instance(a: &RuleVisibility, b: &RuleVisibility) -> bool {
    match (a, b) {
        (RuleVisibility::Public, RuleVisibility::Public) => true,
        (RuleVisibility::Private, RuleVisibility::Private) => true,
        (RuleVisibility::PackageGroups(x), RuleVisibility::PackageGroups(y)) => Arc::ptr_eq(x, y),
        _ => false,
    }
}

// Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:694 StarlarkNativeModule.isPotentiallyExportableAttribute
fn is_potentially_exportable_attribute(attribute_name: &str) -> bool {
    // The name can only start with a letter.
    attribute_name
        .chars()
        .next()
        .is_some_and(|c| c.is_alphabetic())
}

/// Java `starlarkifyValue` of a Label.
fn starlarkify_label(l: &Label, package_identifier: &PackageIdentifier) -> String {
    if l.get_package_identifier() == package_identifier {
        format!(":{}", l.get_name())
    } else if l.get_repository() == package_identifier.get_repository() {
        format!(
            "//{}:{}",
            l.get_package_fragment().get_path_string(),
            l.get_name()
        )
    } else {
        l.get_unambiguous_canonical_form()
    }
}

/// Converts a native attribute value to a Starlark value for `existing_rule(s)`; `None` means
/// "not representable" (the attribute is skipped).
// Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:757 StarlarkNativeModule.starlarkifyValue
pub fn starlarkify_value<'v>(
    heap: Heap<'v>,
    val: Option<&AttrValue>,
    package_identifier: &PackageIdentifier,
) -> Option<Value<'v>> {
    let val = val?;
    let tuple_of = |items: Vec<Value<'v>>| Some(heap.alloc(AllocTuple(items)));
    // Java `Dict.Builder.build(mu)` with `mu == null /* immutable */` (the only caller is
    // ExistingRuleView.starlarkifyAttribute): mutating the dict fails with "trying to mutate a
    // frozen dict value".
    let dict_of = |items: Vec<(Value<'v>, Value<'v>)>| Some(alloc_immutable_dict(heap, items));
    let label = |l: &Label| heap.alloc(starlarkify_label(l, package_identifier));
    let string = |s: &str| heap.alloc(s);
    match val {
        AttrValue::Bool(b) => Some(Value::new_bool(*b)),
        AttrValue::String(s) => Some(string(s)),
        AttrValue::Int(i) => Some(heap.alloc(*i)),
        AttrValue::TriState(t) => Some(heap.alloc(match t {
            TriState::Auto => -1,
            TriState::Yes => 1,
            TriState::No => 0,
        })),
        AttrValue::Label(l) => Some(label(l)),
        AttrValue::StringList(l) => tuple_of(l.iter().map(|s| string(s)).collect()),
        AttrValue::IntList(l) => tuple_of(l.iter().map(|i| heap.alloc(*i)).collect()),
        AttrValue::LabelList(l) => tuple_of(l.iter().map(label).collect()),
        AttrValue::StringDict(d) => {
            dict_of(d.iter().map(|(k, v)| (string(k), string(v))).collect())
        }
        AttrValue::StringListDict(d) => dict_of(
            d.iter()
                .map(|(k, v)| {
                    (
                        string(k),
                        heap.alloc(AllocTuple(v.iter().map(|s| string(s)).collect::<Vec<_>>())),
                    )
                })
                .collect(),
        ),
        AttrValue::LabelDictUnary(d) => {
            dict_of(d.iter().map(|(k, v)| (string(k), label(v))).collect())
        }
        AttrValue::LabelKeyedStringDict(d) => {
            dict_of(d.iter().map(|(k, v)| (label(k), string(v))).collect())
        }
        AttrValue::LabelListDict(d) => dict_of(
            d.iter()
                .map(|(k, v)| {
                    (
                        string(k),
                        heap.alloc(AllocTuple(v.iter().map(label).collect::<Vec<_>>())),
                    )
                })
                .collect(),
        ),
        AttrValue::SelectorList(selector_list) => {
            starlarkify_selector_list(heap, selector_list, package_identifier)
        }
        AttrValue::License(_) => None,
        // Anonymous `ComputedDefault` subclasses and the non-Starlark default objects.
        AttrValue::ComputedDefault(_)
        | AttrValue::StarlarkComputedDefaultTemplate(_)
        | AttrValue::Materializer(_) => None,
        // Java returns the LateBoundDefault object itself (it is a StarlarkValue).
        AttrValue::LateBound(_) => {
            blaze_util::unported!("StarlarkNativeModule.starlarkifyValue(LateBoundDefault)")
        }
        // Java: a STRING_SET value (a Java `Set`, e.g. the `build_setting_default` of a
        // `config.string_set` flag) is neither a List, a Map nor a StarlarkValue: `default ->
        // null`, the attribute is left out.
        AttrValue::StringSet(_) => None,
    }
}

// Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:714 StarlarkNativeModule.starlarkifyValue
fn starlarkify_selector_list<'v>(
    heap: Heap<'v>,
    selector_list: &BuildTypeSelectorList,
    package_identifier: &PackageIdentifier,
) -> Option<Value<'v>> {
    let mut selectors: Vec<Value<'v>> = Vec::new();
    for selector in selector_list.get_selectors() {
        let mut m: IndexMap<Label, Value<'v>> = IndexMap::new();
        let type_has_no_default = selector.get_original_type().get_default_value().is_none();
        selector.for_each(|label, raw_value| {
            let map_val = if raw_value.is_none() && type_has_no_default {
                Some(Value::new_none())
            } else {
                starlarkify_value(heap, raw_value, package_identifier)
            };
            if let Some(map_val) = map_val {
                // Java: ImmutableMap.Builder.buildKeepingLast.
                m.insert(label.clone(), map_val);
            }
        });
        if !m.is_empty() {
            let dictionary: Vec<(Value<'v>, Value<'v>)> = m
                .into_iter()
                .map(|(k, v)| (heap.alloc(LabelValue(k)), v))
                .collect();
            selectors.push(heap.alloc(SelectorValue::new(
                dictionary,
                selector.get_no_match_error().to_owned(),
            )));
        }
    }
    if selectors.is_empty() {
        return None;
    }
    SelectorList::of(&selectors).ok().map(|l| heap.alloc(l))
}

/// The `existing_rule(name)` view of a rule (Java `StarlarkNativeModule.ExistingRuleView`).
// Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:320 StarlarkNativeModule.ExistingRuleView
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct ExistingRuleView {
    #[allocative(skip)]
    rule: Arc<Rule>,
}

starlark_simple_value!(ExistingRuleView);

impl ExistingRuleView {
    pub fn new(rule: Arc<Rule>) -> Self {
        ExistingRuleView { rule }
    }

    fn package_identifier(&self) -> &PackageIdentifier {
        &self.rule.get_package_metadata().package_identifier
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:337 StarlarkNativeModule.ExistingRuleView.starlarkifyAttribute
    fn starlarkify_attribute<'v>(&self, heap: Heap<'v>, attribute_name: &str) -> Option<Value<'v>> {
        if !is_potentially_exportable_attribute(attribute_name) {
            return None;
        }
        starlarkify_value(
            heap,
            self.rule.get_attr(attribute_name).as_ref(),
            self.package_identifier(),
        )
    }

    /// Java `getOrDefault(key, null)`.
    // Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:349 StarlarkNativeModule.ExistingRuleView.getOrDefault
    fn get_or_none<'v>(&self, heap: Heap<'v>, key: Value<'v>) -> Option<Value<'v>> {
        let attribute_name = key.unpack_str()?;
        match attribute_name {
            "name" => Some(heap.alloc(self.rule.get_name())),
            "kind" => Some(heap.alloc(self.rule.get_rule_class())),
            _ => self.starlarkify_attribute(heap, attribute_name),
        }
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:372 StarlarkNativeModule.ExistingRuleView.iterator
    fn keys(&self, heap: Heap<'_>) -> Vec<String> {
        let mut keys = vec!["name".to_owned(), "kind".to_owned()];
        for attribute in self.rule.get_attributes() {
            let attribute_name = attribute.get_name();
            let relevant = match attribute_name {
                // pseudo-names handled specially
                "name" | "kind" => false,
                _ => self.starlarkify_attribute(heap, attribute_name).is_some(),
            };
            if relevant {
                keys.push(attribute_name.to_owned());
            }
        }
        keys
    }
}

impl fmt::Display for ExistingRuleView {
    // Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:328 StarlarkNativeModule.ExistingRuleView.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "<native.ExistingRuleView for target '{}'>",
            self.rule.get_name()
        )
    }
}

// Java `Starlark.type`: a `Map` without a `@StarlarkBuiltin` is a "dict".
#[starlark_value(type = "dict")]
impl<'v> StarlarkValue<'v> for ExistingRuleView {
    // PORT: Java's views are not `Dict`s, so `view[k] = v` is not a dict mutation but
    // EvalUtils.setIndex's last branch, with `Starlark.type(view)` ("dict" for a java.util.Map).
    // Java: net/starlark/java/eval/EvalUtils.java:522 EvalUtils.setIndex
    fn set_at(&self, _index: Value<'v>, _new_value: Value<'v>) -> starlark::Result<()> {
        Err(starlark::Error::new_other(anyhow::anyhow!(
            "can only assign an element in a dictionary or a list, not in a 'dict'"
        )))
    }

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("ExistingRuleView", dict_like_view_methods);
        Some(RES.methods())
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:220 StarlarkNativeModule.DictLikeView.getIndex
    fn at(&self, index: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        match self.get_or_none(heap, index) {
            Some(v) => Ok(v),
            None => Err(eval_err(format!(
                "key {} not found in view",
                index.to_repr()
            ))),
        }
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:423 StarlarkNativeModule.ExistingRuleView.containsKey
    fn is_in(&self, other: Value<'v>) -> starlark::Result<bool> {
        // PORT: `is_in` has no heap; a key is present iff its value is representable, which
        // does not depend on where the value would be allocated.
        let Some(attribute_name) = other.unpack_str() else {
            return Ok(false);
        };
        Ok(match attribute_name {
            "name" | "kind" => true,
            _ => {
                is_potentially_exportable_attribute(attribute_name)
                    && is_representable(self.rule.get_attr(attribute_name).as_ref())
            }
        })
    }

    fn iterate_collect(&self, heap: Heap<'v>) -> starlark::Result<Vec<Value<'v>>> {
        Ok(self
            .keys(heap)
            .iter()
            .map(|k| heap.alloc(k.as_str()))
            .collect())
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:431 StarlarkNativeModule.ExistingRuleView.size
    fn length(&self) -> starlark::Result<i32> {
        let heap_independent = self
            .rule
            .get_attributes()
            .iter()
            .filter(|a| {
                let n = a.get_name();
                n != "name"
                    && n != "kind"
                    && is_potentially_exportable_attribute(n)
                    && is_representable(self.rule.get_attr(n).as_ref())
            })
            .count();
        Ok((2 + heap_independent) as i32)
    }
}

/// Whether [`starlarkify_value`] returns a value for `val` (without allocating).
fn is_representable(val: Option<&AttrValue>) -> bool {
    let Some(val) = val else {
        return false;
    };
    match val {
        AttrValue::License(_)
        | AttrValue::ComputedDefault(_)
        | AttrValue::StarlarkComputedDefaultTemplate(_)
        | AttrValue::Materializer(_) => false,
        AttrValue::SelectorList(selector_list) => selector_list.get_selectors().iter().any(|s| {
            let type_has_no_default = s.get_original_type().get_default_value().is_none();
            let mut any = false;
            s.for_each(|_, raw| {
                any |= (raw.is_none() && type_has_no_default) || is_representable(raw);
            });
            any
        }),
        _ => true,
    }
}

/// The `existing_rules()` view (Java `StarlarkNativeModule.ExistingRulesView`).
// Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:457 StarlarkNativeModule.ExistingRulesView
#[derive(Debug, ProvidesStaticType, NoSerialize, Allocative)]
pub struct ExistingRulesView {
    /// We take a lightweight snapshot of the rules existing in a Package.Builder to avoid
    /// exposing any rules added to Package.Builder after the existing_rules() call which created
    /// this view.
    #[allocative(skip)]
    rules_snapshot_view: IndexMap<String, Arc<Rule>>,
}

starlark_simple_value!(ExistingRulesView);

impl ExistingRulesView {
    pub fn new(rules_snapshot_view: IndexMap<String, Arc<Rule>>) -> Self {
        ExistingRulesView {
            rules_snapshot_view,
        }
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:473 StarlarkNativeModule.ExistingRulesView.getOrDefault
    fn get_or_none<'v>(&self, heap: Heap<'v>, key: Value<'v>) -> Option<Value<'v>> {
        let key = key.unpack_str()?;
        self.rules_snapshot_view
            .get(key)
            .map(|rule| heap.alloc(ExistingRuleView::new(rule.clone())))
    }
}

impl fmt::Display for ExistingRulesView {
    // Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:467 StarlarkNativeModule.ExistingRulesView.repr
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("<native.ExistingRulesView object>")
    }
}

// Java `Starlark.type`: a `Map` without a `@StarlarkBuiltin` is a "dict".
#[starlark_value(type = "dict")]
impl<'v> StarlarkValue<'v> for ExistingRulesView {
    // PORT: Java's views are not `Dict`s, so `view[k] = v` is not a dict mutation but
    // EvalUtils.setIndex's last branch, with `Starlark.type(view)` ("dict" for a java.util.Map).
    // Java: net/starlark/java/eval/EvalUtils.java:522 EvalUtils.setIndex
    fn set_at(&self, _index: Value<'v>, _new_value: Value<'v>) -> starlark::Result<()> {
        Err(starlark::Error::new_other(anyhow::anyhow!(
            "can only assign an element in a dictionary or a list, not in a 'dict'"
        )))
    }

    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("ExistingRulesView", dict_like_view_methods);
        Some(RES.methods())
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:220 StarlarkNativeModule.DictLikeView.getIndex
    fn at(&self, index: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        match self.get_or_none(heap, index) {
            Some(v) => Ok(v),
            None => Err(eval_err(format!(
                "key {} not found in view",
                index.to_repr()
            ))),
        }
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:491 StarlarkNativeModule.ExistingRulesView.containsKey
    fn is_in(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(other
            .unpack_str()
            .is_some_and(|k| self.rules_snapshot_view.contains_key(k)))
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:486 StarlarkNativeModule.ExistingRulesView.iterator
    fn iterate_collect(&self, heap: Heap<'v>) -> starlark::Result<Vec<Value<'v>>> {
        Ok(self
            .rules_snapshot_view
            .keys()
            .map(|k| heap.alloc(k.as_str()))
            .collect())
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:499 StarlarkNativeModule.ExistingRulesView.size
    fn length(&self) -> starlark::Result<i32> {
        Ok(self.rules_snapshot_view.len() as i32)
    }
}

/// The keys of a view, in iteration order.
fn view_keys<'v>(this: Value<'v>, heap: Heap<'v>) -> starlark::Result<Vec<Value<'v>>> {
    if let Some(v) = this.downcast_ref::<ExistingRuleView>() {
        v.iterate_collect(heap)
    } else if let Some(v) = this.downcast_ref::<ExistingRulesView>() {
        v.iterate_collect(heap)
    } else {
        Err(eval_err(format!("not a view: {}", this.get_type())))
    }
}

/// `x.get(key)` on a view (`None` for absent keys).
fn view_get<'v>(this: Value<'v>, heap: Heap<'v>, key: Value<'v>) -> Option<Value<'v>> {
    if let Some(v) = this.downcast_ref::<ExistingRuleView>() {
        v.get_or_none(heap, key)
    } else if let Some(v) = this.downcast_ref::<ExistingRulesView>() {
        v.get_or_none(heap, key)
    } else {
        None
    }
}

/// The methods of Java's `DictLikeView` (`get`, `keys`, `values`, `items`).
#[starlark_module]
fn dict_like_view_methods(builder: &mut MethodsBuilder) {
    // Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:170 StarlarkNativeModule.DictLikeView.getOrDefault
    fn get<'v>(
        this: Value<'v>,
        #[starlark(require = pos)] key: Value<'v>,
        #[starlark(default = NoneType)] default: Value<'v>,
        heap: Heap<'v>,
    ) -> starlark::Result<Value<'v>> {
        Ok(view_get(this, heap, key).unwrap_or(default))
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:177 StarlarkNativeModule.DictLikeView.keys
    fn keys<'v>(this: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        // For now, we return an immutable list, so that migration to a sequence view is less
        // likely to cause breakage.
        // Java: StarlarkList.immutableCopyOf(this)
        Ok(alloc_immutable_list(heap, view_keys(this, heap)?))
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:194 StarlarkNativeModule.DictLikeView.starlarkValues
    fn values<'v>(this: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let mut value_list = Vec::new();
        for key in view_keys(this, heap)? {
            value_list.push(view_get(this, heap, key).expect("NullPointerException: view value"));
        }
        // Java: StarlarkList.immutableCopyOf(valueList)
        Ok(alloc_immutable_list(heap, value_list))
    }

    // Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:209 StarlarkNativeModule.DictLikeView.items
    fn items<'v>(this: Value<'v>, heap: Heap<'v>) -> starlark::Result<Value<'v>> {
        let mut items_list = Vec::new();
        for key in view_keys(this, heap)? {
            let value = view_get(this, heap, key).expect("NullPointerException: view value");
            items_list.push(heap.alloc((key, value)));
        }
        // Java: StarlarkList.immutableCopyOf(itemsList)
        Ok(alloc_immutable_list(heap, items_list))
    }
}

#[starlark_module]
pub(crate) fn starlark_native_module(builder: &mut GlobalsBuilder) {
    /// Glob returns a new, mutable, sorted list of every file in the current package that
    /// matches at least one pattern in include and none of the patterns in exclude.
    // Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:93 StarlarkNativeModule.glob
    fn glob<'v>(
        include: Option<Value<'v>>,
        exclude: Option<Value<'v>>,
        #[starlark(default = 1)] exclude_directories: Value<'v>,
        allow_empty: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        let heap = eval.heap();
        // Java: the interpreter checks the arguments against the `@Param` types before the
        // method runs: `include` and `exclude` are
        // `@ParamType(type = Sequence.class, generic1 = String.class)`, `exclude_directories` is
        // a `StarlarkInt`.
        if let Some(include) = include {
            check_sequence_param("glob", "include", include)?;
        }
        if let Some(exclude) = exclude {
            check_sequence_param("glob", "exclude", exclude)?;
        }
        let exclude_dirs_nonzero = exclude_directories_nonzero(exclude_directories)?;
        TargetDefinitionContext::from_or_fail_allow_build_only(eval, "glob()").map_err(eval_err)?;
        let empty = heap.alloc(AllocList(Vec::<Value>::new()));
        let includes = convert_string_list(include.unwrap_or(empty), "'glob' argument", heap)?;
        let excludes = convert_string_list(exclude.unwrap_or(empty), "'glob' argument", heap)?;
        let op = if exclude_dirs_nonzero {
            GlobberOperation::Files
        } else {
            GlobberOperation::FilesAndDirs
        };
        let allow_empty = glob_allow_empty(eval, allow_empty)?;

        let matches = run_glob_operation(eval, "glob()", &includes, &excludes, op, allow_empty)?;

        let mut result: Vec<String> = Vec::with_capacity(matches.len());
        for m in matches {
            if m.starts_with('@') {
                // Add explicit colon to disambiguate from external repository.
                result.push(format!(":{m}"));
            } else {
                result.push(m);
            }
        }
        // PORT: Java's natural String order (UTF-16 code units) equals byte order for the
        // file names Blaze supports (decisions.md: code-point string semantics).
        result.sort();
        Ok(heap.alloc(AllocList(result)))
    }

    /// Returns an immutable dict-like object describing the attributes of a rule instantiated
    /// in this thread's package, or `None` if no rule instance of that name exists.
    // Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:437 StarlarkNativeModule.existingRule
    fn existing_rule<'v>(
        #[starlark(require = pos)] name: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        if existing_rules_should_be_no_op(eval) {
            return Ok(Value::new_none());
        }
        let rule = {
            let target_definition_context =
                TargetDefinitionContext::from_or_fail_disallow_non_finalizer_macros(
                    eval,
                    "existing_rule()",
                )
                .map_err(eval_err)?;
            // PORT: PackagePiece.ForBuildFile.Builder (lazy macro expansion) is not ported.
            target_definition_context.get_non_finalizer_instantiated_rule(name)
        };
        Ok(match rule {
            Some(rule) => eval.heap().alloc(ExistingRuleView::new(rule)),
            None => Value::new_none(),
        })
    }

    /// Returns an immutable dict-like object describing the rules so far instantiated in this
    /// thread's package.
    // Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:509 StarlarkNativeModule.existingRules
    fn existing_rules<'v>(eval: &mut Evaluator<'v, '_, '_>) -> starlark::Result<Value<'v>> {
        if existing_rules_should_be_no_op(eval) {
            // Java: Dict.empty() (the immutable empty dict).
            return Ok(alloc_immutable_dict(eval.heap(), Vec::new()));
        }
        let snapshot = {
            let target_definition_context =
                TargetDefinitionContext::from_or_fail_disallow_non_finalizer_macros(
                    eval,
                    "existing_rules()",
                )
                .map_err(eval_err)?;
            // PORT: PackagePiece.ForBuildFile.Builder (lazy macro expansion) is not ported.
            target_definition_context.get_rules_snapshot_view()
        };
        Ok(eval.heap().alloc(ExistingRulesView::new(snapshot)))
    }

    /// This function defines a set of packages and assigns a label to the group.
    // Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:525 StarlarkNativeModule.packageGroup
    fn package_group<'v>(
        #[starlark(require = named)] name: &str,
        #[starlark(require = named)] packages: Option<Value<'v>>,
        #[starlark(require = named)] includes: Option<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        let heap = eval.heap();
        // Java: `packages` and `includes` are
        // `@ParamType(type = Sequence.class, generic1 = String.class)`.
        if let Some(packages) = packages {
            check_sequence_param("package_group", "packages", packages)?;
        }
        if let Some(includes) = includes {
            check_sequence_param("package_group", "includes", includes)?;
        }
        let semantics = ThreadContext::get(eval)
            .map_err(starlark::Error::new_other)?
            .semantics
            .clone();
        let loc = caller_location(eval).unwrap_or_else(Location::builtin);
        let mut target_definition_context =
            TargetDefinitionContext::from_or_fail(eval, "package_group()").map_err(eval_err)?;
        let empty = heap.alloc(AllocList(Vec::<Value>::new()));
        let packages = convert_string_list(
            packages.unwrap_or(empty),
            "'package_group.packages argument'",
            heap,
        )?;
        let includes = convert_label_list(
            includes.unwrap_or(empty),
            "'package_group.includes argument'",
            &target_definition_context,
            heap,
        )?;

        match target_definition_context.add_package_group(
            name,
            &packages,
            includes,
            /* allow_public_private= */
            semantics.incompatible_package_group_has_public_syntax,
            /* repo_root_means_current_repo= */
            semantics.incompatible_fix_package_group_reporoot_syntax,
            loc,
        ) {
            Ok(()) => Ok(NoneType),
            Err(PackageBuilderError::LabelSyntax(e)) => Err(eval_err(format!(
                "package group has invalid name: {}: {}",
                name,
                e.get_message()
            ))),
            Err(PackageBuilderError::NameConflict(e)) => Err(eval_err(e)),
        }
    }

    /// Specifies a list of files belonging to this package that are exported to other
    /// packages.
    // Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:563 StarlarkNativeModule.exportsFiles
    fn exports_files<'v>(
        srcs: Value<'v>,
        #[starlark(default = NoneOr::None)] visibility: NoneOr<Value<'v>>,
        #[starlark(default = NoneOr::None)] licenses: NoneOr<Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        let heap = eval.heap();
        // Java: `srcs` is `@ParamType(type = Sequence.class, generic1 = String.class)`;
        // `visibility` and `licenses` are Sequence or NoneType.
        check_sequence_param("exports_files", "srcs", srcs)?;
        let visibility = visibility.into_option();
        if let Some(visibility) = visibility {
            check_sequence_or_none_param("exports_files", "visibility", visibility)?;
        }
        let licenses = licenses.into_option();
        if let Some(licenses) = licenses {
            check_sequence_or_none_param("exports_files", "licenses", licenses)?;
        }
        let loc = caller_location(eval).unwrap_or_else(Location::builtin);
        let mut target_definition_context =
            TargetDefinitionContext::from_or_fail(eval, "exports_files()").map_err(eval_err)?;
        let files = convert_string_list(srcs, "'exports_files' operand", heap)?;

        let mut visibility = match visibility {
            None => RuleVisibility::Public,
            Some(v) => RuleVisibility::parse(&convert_label_list(
                v,
                "'exports_files' operand",
                &target_definition_context,
                heap,
            )?)
            .map_err(eval_err)?,
        };

        // TODO(b/234815294): Exports should have different visibility semantics.
        if let Some(current_macro) = target_definition_context.current_macro() {
            visibility = visibility.concat_with_package(current_macro.get_definition_package());
        }

        // Java: BuildType.LICENSE.convertOptional (None -> null).
        let license = match licenses {
            None => None,
            Some(v) => match Type::License
                .convert(v, Some(&"'exports_files' operand"), None)
                .map_err(eval_err)?
            {
                AttrValue::License(l) => Some(l),
                other => panic!("ClassCastException: LICENSE.convert returned {other:?}"),
            },
        };

        for file in &files {
            if let Some(error_message) = blaze_cmdline::label_validator::validate_target_name(file)
            {
                return Err(eval_err(error_message));
            }
            let input_file = target_definition_context
                .create_input_file(file, loc.clone())
                .map_err(eval_err)?;
            if input_file.is_visibility_specified()
                && !input_file
                    .get_raw_visibility()
                    .is_some_and(|v| same_visibility_instance(&v, &visibility))
            {
                return Err(eval_err(format!(
                    "visibility for exported file '{}' declared twice",
                    input_file.get_name()
                )));
            }
            if license.is_some() && input_file.is_license_specified() {
                return Err(eval_err(format!(
                    "licenses for exported file '{}' declared twice",
                    input_file.get_name()
                )));
            }

            target_definition_context.set_visibility_and_license(
                &input_file,
                Some(visibility.clone()),
                license.clone(),
            );
        }
        Ok(NoneType)
    }

    /// The name of the package being evaluated, without the repository name.
    // Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:617 StarlarkNativeModule.packageName
    fn package_name<'v>(eval: &mut Evaluator<'v, '_, '_>) -> starlark::Result<String> {
        let target_definition_context =
            TargetDefinitionContext::from_or_fail(eval, "package_name()").map_err(eval_err)?;
        Ok(target_definition_context
            .get_package_identifier()
            .get_package_fragment()
            .get_path_string()
            .to_owned())
    }

    /// Returns the default visibility of the package being evaluated.
    // Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:624 StarlarkNativeModule.packageDefaultVisibility
    fn package_default_visibility<'v>(
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        let labels = {
            let target_definition_context =
                TargetDefinitionContext::from_or_fail(eval, "package_default_visibility()")
                    .map_err(eval_err)?;
            target_definition_context
                .get_partial_package_args()
                .default_visibility()
                .expect("NullPointerException: defaultVisibility()")
                .concat_with_package(target_definition_context.get_package_identifier())
                .get_declared_labels()
        };
        let heap = eval.heap();
        Ok(heap.alloc(AllocList(
            labels
                .into_iter()
                .map(|l| heap.alloc(LabelValue(l)))
                .collect::<Vec<_>>(),
        )))
    }

    /// Deprecated: the canonical name of the repository containing the package, with a single
    /// at-sign prefixed.
    // Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:637 StarlarkNativeModule.repositoryName
    fn repository_name<'v>(eval: &mut Evaluator<'v, '_, '_>) -> starlark::Result<String> {
        // Java: `@StarlarkMethod(enableOnlyWithFlag = INCOMPATIBLE_ENABLE_DEPRECATED_LABEL_APIS)`.
        // The function is always bound (`Starlark.addMethods(bindings, new
        // StarlarkNativeModule())` selects methods with the default semantics, under which the
        // flag is true); calling it checks the thread's semantics first.
        // Java: net/starlark/java/eval/MethodDescriptor.java:409 MethodDescriptor.checkEnabled
        if !ThreadContext::get(eval)
            .map_err(starlark::Error::new_other)?
            .semantics
            .incompatible_enable_deprecated_label_apis
        {
            return Err(eval_err(
                "function repository_name() is experimental and thus unavailable with the \
                 current flags. It may be enabled by setting \
                 --incompatible_enable_deprecated_label_apis",
            ));
        }
        TargetDefinitionContext::from_or_fail(eval, "repository_name()").map_err(eval_err)?;
        Ok(format!("@{}", repo_name_impl(eval)?))
    }

    /// The canonical name of the repository containing the package currently being evaluated,
    /// with no leading at-signs.
    // Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:640 StarlarkNativeModule.repoName
    fn repo_name<'v>(eval: &mut Evaluator<'v, '_, '_>) -> starlark::Result<String> {
        repo_name_impl(eval)
    }

    /// Converts the input string into a Label object, in the context of the package currently
    /// being initialized.
    // Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:651 StarlarkNativeModule.packageRelativeLabel
    fn package_relative_label<'v>(
        #[starlark(require = pos)] input: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        let heap = eval.heap();
        // Java: the interpreter checks `input` against
        // `allowedTypes = {String.class, Label.class}` before the method runs.
        if input.downcast_ref::<LabelValue>().is_none() && input.unpack_str().is_none() {
            return Err(eval_err(format!(
                "in call to package_relative_label(), parameter 'input' got value of type '{}', \
                 want 'string or Label'",
                input.get_type()
            )));
        }
        // In an initializer, BazelStarlarkContext isn't available, just the label converter
        // (Java `thread.getThreadLocal(LabelConverter.class)`, installed by
        // StarlarkRuleFunction.call around the rule's initializers).
        let label_converter: Arc<LabelConverter> = match ThreadContext::get(eval)
            .map_err(starlark::Error::new_other)?
            .get_thread_local::<Arc<LabelConverter>>()
        {
            Some(label_converter) => label_converter,
            None => {
                let target_definition_context =
                    TargetDefinitionContext::from_or_fail(eval, "package_relative_label()")
                        .map_err(eval_err)?;
                target_definition_context.get_label_converter_arc().clone()
            }
        };
        if input.downcast_ref::<LabelValue>().is_some() {
            return Ok(input);
        }
        let input = input
            .unpack_str()
            .expect("ClassCastException: input is not a String");
        match label_converter.convert(input) {
            Ok(label) => Ok(heap.alloc(LabelValue(label))),
            Err(e) => Err(eval_err(format!(
                "invalid label in native.package_relative_label: {}",
                e.get_message()
            ))),
        }
    }

    /// The name of the Bazel module associated with the repo this package is in.
    // Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:671 StarlarkNativeModule.moduleName
    fn module_name<'v>(eval: &mut Evaluator<'v, '_, '_>) -> starlark::Result<NoneOr<String>> {
        let target_definition_context =
            TargetDefinitionContext::from_or_fail(eval, "module_name()").map_err(eval_err)?;
        Ok(NoneOr::from_option(
            target_definition_context
                .get_associated_module_name()
                .map(str::to_owned),
        ))
    }

    /// The version of the Bazel module associated with the repo this package is in.
    // Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:679 StarlarkNativeModule.moduleVersion
    fn module_version<'v>(eval: &mut Evaluator<'v, '_, '_>) -> starlark::Result<NoneOr<String>> {
        let target_definition_context =
            TargetDefinitionContext::from_or_fail(eval, "module_version()").map_err(eval_err)?;
        Ok(NoneOr::from_option(
            target_definition_context
                .get_associated_module_version()
                .map(str::to_owned),
        ))
    }

    /// Returns a new mutable list of every direct subpackage of the current package.
    // Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:811 StarlarkNativeModule.subpackages
    fn subpackages<'v>(
        #[starlark(require = named)] include: Value<'v>,
        #[starlark(require = named)] exclude: Option<Value<'v>>,
        #[starlark(require = named, default = false)] allow_empty: bool,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        let heap = eval.heap();
        // Java: `include` and `exclude` are
        // `@ParamType(type = Sequence.class, generic1 = String.class)`.
        check_sequence_param("subpackages", "include", include)?;
        if let Some(exclude) = exclude {
            check_sequence_param("subpackages", "exclude", exclude)?;
        }
        TargetDefinitionContext::from_or_fail_allow_build_only(eval, "subpackages()")
            .map_err(eval_err)?;
        let empty = heap.alloc(AllocList(Vec::<Value>::new()));
        let includes = convert_string_list(include, "'subpackages' argument", heap)?;
        let excludes =
            convert_string_list(exclude.unwrap_or(empty), "'subpackages' argument", heap)?;

        let mut matches = run_glob_operation(
            eval,
            "subpackages()",
            &includes,
            &excludes,
            GlobberOperation::Subpackages,
            allow_empty,
        )?;
        if !matches.is_empty() {
            matches.sort();
        }
        Ok(heap.alloc(AllocList(matches)))
    }
}

// Java: com/google/devtools/build/lib/packages/StarlarkNativeModule.java:640 StarlarkNativeModule.repoName
fn repo_name_impl(eval: &Evaluator<'_, '_, '_>) -> starlark::Result<String> {
    let target_definition_context =
        TargetDefinitionContext::from_or_fail(eval, "repo_name()").map_err(eval_err)?;
    Ok(target_definition_context
        .get_package_identifier()
        .get_repository()
        .get_name()
        .to_owned())
}
