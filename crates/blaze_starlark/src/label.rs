//! The Starlark `Label` value and the `Label()` constructor.
//!
//! Java: com/google/devtools/build/lib/cmdline/Label.java (the `@StarlarkBuiltin` methods, repr,
//! str, debugPrint) and analysis/starlark/StarlarkRuleClassFunctions.java:2124 `label()`.
//! The label itself is `blaze_cmdline::Label`; [`LabelValue`] wraps it as a Starlark value (the
//! wrapper lives here because blaze_starlark sits above blaze_cmdline in the crate graph).

use std::fmt;
use std::hash::Hash;

use allocative::Allocative;
use blaze_cmdline::Label;
use blaze_cmdline::LabelPackageContext;
use starlark::any::ProvidesStaticType;
use starlark::environment::GlobalsBuilder;
use starlark::environment::Methods;
use starlark::environment::MethodsBuilder;
use starlark::environment::MethodsStatic;
use starlark::eval::Evaluator;
use starlark::starlark_module;
use starlark::starlark_simple_value;
use starlark::values::Demand;
use starlark::values::NoSerialize;
use starlark::values::StarlarkValue;
use starlark::values::Value;
use starlark::values::ValueLike;
use starlark::values::starlark_value;
use starlark_map::StarlarkHasher;

use crate::structs::DeeplyImmutable;
use crate::thread::ThreadContext;

/// A `blaze_cmdline::Label` as a Starlark value (type `Label`).
#[derive(Debug, Clone, PartialEq, Eq, Hash, ProvidesStaticType, NoSerialize, Allocative)]
pub struct LabelValue(#[allocative(skip)] pub Label);

starlark_simple_value!(LabelValue);

impl LabelValue {
    pub fn label(&self) -> &Label {
        &self.0
    }
}

impl fmt::Display for LabelValue {
    // Java: Label.repr — Label("<canonical form>")
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "Label({})",
            starlark_repr_str(&self.0.get_canonical_form())
        )
    }
}

fn eval_err(e: impl fmt::Display) -> starlark::Error {
    starlark::Error::new_other(anyhow::anyhow!("{e}"))
}

#[starlark_value(type = "Label")]
impl<'v> StarlarkValue<'v> for LabelValue {
    fn get_methods() -> Option<&'static Methods> {
        static RES: MethodsStatic = MethodsStatic::new("Label", label_methods);
        Some(RES.methods())
    }

    fn collect_repr(&self, collector: &mut String) {
        collector.push_str(&format!(
            "Label({})",
            starlark_repr_str(&self.0.get_canonical_form())
        ));
    }

    // Java: Label.str with --incompatible_unambiguous_label_stringification (default true; the
    // value cannot see the thread's semantics here).
    fn collect_str(&self, collector: &mut String) {
        collector.push_str(&self.0.str(true));
    }

    fn equals(&self, other: Value<'v>) -> starlark::Result<bool> {
        Ok(other
            .downcast_ref::<LabelValue>()
            .is_some_and(|o| o.0 == self.0))
    }

    fn write_hash(&self, hasher: &mut StarlarkHasher) -> starlark::Result<()> {
        self.0.hash(hasher);
        Ok(())
    }

    // Java: Label implements Comparable.
    fn compare(&self, other: Value<'v>) -> starlark::Result<std::cmp::Ordering> {
        match other.downcast_ref::<LabelValue>() {
            Some(o) => Ok(self.0.cmp(&o.0)),
            None => Err(eval_err(format!(
                "unsupported comparison: Label <=> {}",
                other.get_type()
            ))),
        }
    }

    fn provide(&'v self, demand: &mut Demand<'_, 'v>) {
        demand.provide_value(DeeplyImmutable);
    }
}

/// Starlark `repr` of a string (Java Printer.appendQuoted; label strings need no other escapes).
fn starlark_repr_str(s: &str) -> String {
    format!("\"{}\"", s.replace('\\', "\\\\").replace('"', "\\\""))
}

#[starlark_module]
fn label_methods(builder: &mut MethodsBuilder) {
    /// Java: Label.getName (Label.java:462)
    #[starlark(attribute)]
    fn name(this: &LabelValue) -> anyhow::Result<String> {
        Ok(this.0.get_name().to_owned())
    }

    /// Java: Label.getPackageName (Label.java:360)
    #[starlark(attribute)]
    fn package(this: &LabelValue) -> anyhow::Result<String> {
        Ok(this.0.get_package_name().to_owned())
    }

    /// Java: Label.getWorkspaceName (Label.java:543)
    #[starlark(attribute)]
    fn workspace_name(this: &LabelValue) -> starlark::Result<String> {
        this.0
            .get_workspace_name()
            .map_err(|e| eval_err(e.get_message()))
    }

    /// Java: Label.getRepoName (Label.java:560)
    #[starlark(attribute)]
    fn repo_name(this: &LabelValue) -> starlark::Result<String> {
        this.0
            .get_repo_name()
            .map_err(|e| eval_err(e.get_message()))
    }

    /// Java: Label.getWorkspaceRootForStarlarkOnly (Label.java:380). Reads
    /// --experimental_sibling_repository_layout as its default (false); attributes cannot see
    /// the thread.
    #[starlark(attribute)]
    fn workspace_root(this: &LabelValue) -> starlark::Result<String> {
        this.0
            .get_workspace_root_for_starlark_only(false)
            .map_err(|e| eval_err(e.get_message()))
    }

    /// Java: Label.getRepoRootForStarlarkOnly (Label.java:409)
    #[starlark(attribute)]
    fn repo_root(this: &LabelValue) -> starlark::Result<String> {
        this.0
            .get_repo_root_for_starlark_only(false)
            .map_err(|e| eval_err(e.get_message()))
    }

    /// Java: Label.getSamePackageLabel (Label.java:577)
    fn same_package_label(this: &LabelValue, target_name: &str) -> starlark::Result<LabelValue> {
        this.0
            .get_same_package_label(target_name)
            .map(LabelValue)
            .map_err(|e| eval_err(e.get_message()))
    }

    /// Java: Label.getRelative (Label.java:594); needs
    /// --incompatible_enable_deprecated_label_apis (default true).
    fn relative<'v>(
        this: &LabelValue,
        #[starlark(require = pos)] rel_name: &str,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<LabelValue> {
        let cx = ThreadContext::get(eval)?;
        if !cx.semantics.incompatible_enable_deprecated_label_apis {
            return Err(eval_err("'Label' value has no field or method 'relative'"));
        }
        let mapping = cx
            .label_repo_context
            .as_ref()
            .map(|c| blaze_cmdline::LabelRepoContext::repo_mapping(c).clone())
            .ok_or_else(|| eval_err("Label.relative() requires a repository mapping"))?;
        let pkg = LabelPackageContext::of(this.0.get_package_identifier(), mapping);
        Label::parse_with_package_context(rel_name, &pkg)
            .map(LabelValue)
            .map_err(|e| eval_err(e.get_message()))
    }
}

/// Registers `Label()` (Java: StarlarkRuleClassFunctions.label, in the .bzl environment).
#[starlark_module]
pub fn register_label(builder: &mut GlobalsBuilder) {
    /// Java: analysis/starlark/StarlarkRuleClassFunctions.java:2124 label
    #[starlark(as_type = LabelValue)]
    fn Label<'v>(
        #[starlark(require = pos)] input: Value<'v>,
        eval: &mut Evaluator<'v, '_, '_>,
    ) -> starlark::Result<Value<'v>> {
        if input.downcast_ref::<LabelValue>().is_some() {
            return Ok(input);
        }
        let Some(s) = input.unpack_str() else {
            return Err(eval_err(format!(
                "in call to Label(), parameter 'input' got value of type '{}', want 'string or Label'",
                input.get_type()
            )));
        };
        let cx = ThreadContext::get(eval)?;
        // The label string is interpreted with respect to the .bzl module containing the call
        // to `Label()`: the module of the innermost Starlark function, whatever thread runs it.
        // Java: com/google/devtools/build/lib/cmdline/BazelModuleContext.java:174 BazelModuleContext.ofInnermostBzlOrFail
        let pkg = crate::defining_module::package_context_of_innermost_enclosing_starlark_function(
            eval,
        )
        .or_else(|| {
            // PORT: a context registered on the thread for the call site's file
            // (`ThreadContext::with_file_package_context`), for modules executed without an
            // extra value. There is no fallback to the thread's own file label: Java fails.
            let file = eval
                .call_stack_top_location()
                .map(|l| l.filename().to_owned())?;
            cx.file_package_contexts.get(&file).cloned()
        })
        .ok_or_else(|| {
            eval_err("Label() can only be used during .bzl initialization (top-level evaluation)")
        })?;
        // Java: thread.getThreadLocal(Label.RepoMappingRecorder.class)
        let mut recorder = cx.repo_mapping_recorder();
        let label = Label::parse_with_package_context_recording(
            s,
            &pkg,
            recorder
                .as_mut()
                .map(|r| r as &mut dyn blaze_cmdline::LabelRepoMappingRecorder),
        )
        .map_err(|e| eval_err(format!("invalid label in Label(): {}", e.get_message())))?;
        Ok(eval.heap().alloc(LabelValue(label)))
    }
}
