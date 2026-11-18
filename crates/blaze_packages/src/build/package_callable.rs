//! Port of `packages/PackageCallable.java`: the BUILD function `package(**kwargs)`.

use starlark::environment::GlobalsBuilder;
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::values::Value;
use starlark::values::none::NoneType;
use starlark_map::small_map::SmallMap;

use crate::build::eval_err;
use crate::package::PackageArgs;
use crate::package::PackageArgsBuilder;
use crate::package::TargetDefinitionContext;

/// Processes one `package()` argument (Java `PackageCallable.processParam`, overridden by
/// subclasses in Google's build).
// Java: com/google/devtools/build/lib/packages/PackageCallable.java:74 PackageCallable.processParam
fn process_param<'v>(
    name: &str,
    raw_value: Value<'v>,
    pkg_builder: &TargetDefinitionContext,
    pkg_args_builder: &mut PackageArgsBuilder,
) -> starlark::Result<()> {
    PackageArgs::process_param(
        name,
        raw_value,
        &format!("package() argument '{name}'"),
        pkg_builder.get_label_converter(),
        pkg_args_builder,
    )
    .map_err(eval_err)
}

#[starlark_module]
pub(crate) fn package_callable(builder: &mut GlobalsBuilder) {
    /// Declares metadata that applies to every rule in the package.
    // Java: com/google/devtools/build/lib/packages/PackageCallable.java:50 PackageCallable.packageCallable
    fn package<'v>(
        #[starlark(kwargs)] kwargs: SmallMap<String, Value<'v>>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<NoneType> {
        // TODO(bazel-team): we should properly ban package() in legacy macros
        let mut pkg_builder =
            match TargetDefinitionContext::from_or_fail_allow_build_only(eval, "package()") {
                Ok(b) => b,
                // The eval exception thrown by fromOrFailAllowBuildOnly() advises the user that
                // using package() in legacy macros is ok. We don't want to give that advice.
                Err(_) => {
                    return Err(eval_err(
                        "package() can only be used while evaluating a BUILD file",
                    ));
                }
            };
        if pkg_builder.is_package_function_used() {
            return Err(eval_err("'package' can only be used once per BUILD file"));
        }
        pkg_builder.set_package_function_used();

        if kwargs.is_empty() {
            return Err(eval_err(
                "at least one argument must be given to the 'package' function",
            ));
        }

        let mut pkg_args_builder = PackageArgs::builder();
        for (name, raw_value) in kwargs.iter() {
            process_param(name, *raw_value, &pkg_builder, &mut pkg_args_builder)?;
        }
        pkg_builder.merge_package_args_from(&pkg_args_builder.build());
        Ok(NoneType)
    }
}
