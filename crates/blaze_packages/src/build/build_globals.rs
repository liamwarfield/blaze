//! Port of `packages/BuildGlobals.java`: the BUILD-only functions `environment_group` and
//! `licenses`.

use blaze_cmdline::Label;
use blaze_events::EventHandler;
use blaze_starlark::Location;
use starlark::environment::GlobalsBuilder;
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::values::Heap;
use starlark::values::Value;
use starlark::values::list::ListRef;
use starlark::values::none::NoneType;
use starlark::values::tuple::TupleRef;

use crate::build::caller_location;
use crate::build::eval_err;
use crate::exceptions::PackageLoadingCode;
use crate::package::Package;
use crate::package::PackageArgs;
use crate::package::PackageBuilderError;
use crate::package::TargetDefinitionContext;
use crate::types::AttrValue;
use crate::types::Type;

/// Java `x instanceof Sequence`: a list, a tuple or a range
/// (net/starlark/java/eval/{StarlarkList,Tuple,RangeList}.java). Not a dict, set or depset.
// Java: net/starlark/java/eval/Sequence.java:36 Sequence
pub(crate) fn is_java_sequence(x: Value<'_>) -> bool {
    ListRef::from_value(x).is_some() || TupleRef::from_value(x).is_some() || x.get_type() == "range"
}

/// The interpreter's check of an argument whose `@Param` allows only `Sequence` (declared with
/// `@ParamType(type = Sequence.class, ...)` or as a `Sequence<?>` Java parameter; the element
/// type `generic1` is not checked at the call). `func` is the `@StarlarkMethod` name.
// Java: net/starlark/java/eval/BuiltinFunction.java:498 BuiltinFunction.checkParamValue
// Java: net/starlark/java/eval/ParamDescriptor.java:123 ParamDescriptor.getTypeErrorMessage
pub(crate) fn check_sequence_param(func: &str, param: &str, x: Value<'_>) -> starlark::Result<()> {
    if is_java_sequence(x) {
        return Ok(());
    }
    Err(eval_err(format!(
        "in call to {func}(), parameter '{param}' got value of type '{}', want 'sequence'",
        x.get_type()
    )))
}

/// Like [`check_sequence_param`] for `allowedTypes = {Sequence, NoneType}`.
// Java: net/starlark/java/eval/BuiltinFunction.java:498 BuiltinFunction.checkParamValue
pub(crate) fn check_sequence_or_none_param(
    func: &str,
    param: &str,
    x: Value<'_>,
) -> starlark::Result<()> {
    if is_java_sequence(x) || x.is_none() {
        return Ok(());
    }
    Err(eval_err(format!(
        "in call to {func}(), parameter '{param}' got value of type '{}', want 'sequence or \
         NoneType'",
        x.get_type()
    )))
}

/// Converts `x` with `BuildType.LABEL_LIST` (Java `LABEL_LIST.convert(x, what, converter)`).
/// `heap` is the heap of `x` (starlark-rust iterates a `range` only with a heap).
pub(crate) fn convert_label_list<'v>(
    x: Value<'v>,
    what: &str,
    ctx: &TargetDefinitionContext,
    heap: Heap<'v>,
) -> starlark::Result<Vec<Label>> {
    match Type::LabelList
        .convert_with_heap(x, Some(&what), Some(ctx.get_label_converter()), heap)
        .map_err(eval_err)?
    {
        AttrValue::LabelList(l) => Ok(l),
        other => panic!("ClassCastException: LABEL_LIST.convert returned {other:?}"),
    }
}

/// Converts `x` with `Types.STRING_LIST` (Java `STRING_LIST.convert(x, what)`). `heap` is the
/// heap of `x` (starlark-rust iterates a `range` only with a heap).
pub(crate) fn convert_string_list<'v>(
    x: Value<'v>,
    what: &str,
    heap: Heap<'v>,
) -> starlark::Result<Vec<String>> {
    match Type::StringList
        .convert_with_heap(x, Some(&what), None, heap)
        .map_err(eval_err)?
    {
        AttrValue::StringList(l) => Ok(l),
        other => panic!("ClassCastException: STRING_LIST.convert returned {other:?}"),
    }
}

#[starlark_module]
pub(crate) fn build_globals(builder: &mut GlobalsBuilder) {
    /// Defines a set of related environments that can be tagged onto rules to prevent
    /// incompatible rules from depending on each other.
    // Java: com/google/devtools/build/lib/packages/BuildGlobals.java:71 BuildGlobals.environmentGroup
    fn environment_group<'v>(
        #[starlark(require = named)] name: &str,
        #[starlark(require = named)] environments: Value<'v>,
        #[starlark(require = named)] defaults: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        // Java: both parameters are `@ParamType(type = Sequence.class, generic1 = Label.class)`.
        check_sequence_param("environment_group", "environments", environments)?;
        check_sequence_param("environment_group", "defaults", defaults)?;
        let heap = eval.heap();
        let loc = caller_location(eval).unwrap_or_else(Location::builtin);
        let mut pkg_builder =
            TargetDefinitionContext::from_or_fail_allow_build_only(eval, "environment_group()")
                .map_err(eval_err)?;
        let environments = convert_label_list(
            environments,
            "'environment_group argument'",
            &pkg_builder,
            heap,
        )?;
        let defaults =
            convert_label_list(defaults, "'environment_group argument'", &pkg_builder, heap)?;

        if environments.is_empty() {
            return Err(eval_err(format!(
                "environment group {name} must contain at least one environment"
            )));
        }
        match pkg_builder.add_environment_group(name, environments, defaults, loc) {
            Ok(()) => Ok(NoneType),
            Err(PackageBuilderError::LabelSyntax(e)) => Err(eval_err(format!(
                "environment group has invalid name: {}: {}",
                name,
                e.get_message()
            ))),
            Err(PackageBuilderError::NameConflict(e)) => Err(eval_err(e)),
        }
    }

    /// Declare the license(s) for the code in the current package.
    // Java: com/google/devtools/build/lib/packages/BuildGlobals.java:114 BuildGlobals.licenses
    fn licenses<'v>(
        #[starlark(require = pos)] license_strings: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        // Java: `@ParamType(type = Sequence.class, generic1 = String.class)`.
        check_sequence_param("licenses", "license_strings", license_strings)?;
        let loc = caller_location(eval);
        let mut pkg_builder =
            TargetDefinitionContext::from_or_fail_allow_build_only(eval, "licenses()")
                .map_err(eval_err)?;
        match Type::License.convert(license_strings, Some(&"'licenses' operand"), None) {
            Ok(AttrValue::License(license)) => {
                let mut args = PackageArgs::builder();
                args.set_license(license);
                pkg_builder.merge_package_args_from(&args.build());
            }
            Ok(other) => panic!("ClassCastException: LICENSE.convert returned {other:?}"),
            Err(e) => {
                pkg_builder
                    .get_local_event_handler()
                    .handle(&Package::error(
                        loc.as_ref(),
                        &e.to_string(),
                        PackageLoadingCode::LicenseParseFailure,
                    ));
                pkg_builder.set_contains_errors();
            }
        }
        Ok(NoneType)
    }
}
