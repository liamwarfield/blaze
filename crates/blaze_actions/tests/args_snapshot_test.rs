//! `snapshot_args` / `snapshot_template_dict`: an action sees its `Args` and `TemplateDict` as
//! they were when the action was created (review 2026-11-18-analysis S1-3).
//!
//! Java: `StarlarkActionFactory.buildCommandLine` calls `args.build()` and `getParamFileInfo()` at
//! the `ctx.actions.run` call (`StarlarkCustomCommandLine.Builder.build` copies the arguments);
//! `expand_template` calls `TemplateDict.getAll()` at the call.

use std::sync::Arc;

use blaze_actions::analysis::starlark::args::{build_args, snapshot_args};
use blaze_actions::analysis::starlark::template_dict::{get_all, snapshot_template_dict};

#[starlark::starlark_module]
fn natives(builder: &mut starlark::environment::GlobalsBuilder) {
    fn make_args<'v>(
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<starlark::values::Value<'v>> {
        let semantics = blaze_starlark::thread::ThreadContext::get(eval)?
            .semantics
            .clone();
        Ok(blaze_actions::analysis::starlark::args::new_args(
            eval.heap(),
            semantics,
        ))
    }

    fn make_template_dict<'v>(
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<starlark::values::Value<'v>> {
        Ok(blaze_actions::analysis::starlark::template_dict::new_template_dict(eval.heap()))
    }

    /// What `ctx.actions.run(arguments = [a])` / `expand_template(computed_substitutions = d)`
    /// keep of their argument.
    fn snapshot<'v>(
        value: starlark::values::Value<'v>,
        eval: &mut starlark::eval::Evaluator<'v, '_, '_>,
    ) -> starlark::Result<starlark::values::Value<'v>> {
        Ok(snapshot_template_dict(
            snapshot_args(value, eval.heap()),
            eval.heap(),
        ))
    }
}

fn eval(code: &str) -> starlark::environment::FrozenModule {
    let semantics = Arc::new(blaze_starlark::StarlarkSemantics::default());
    let globals = blaze_starlark::env::universe_builder(&semantics)
        .with(blaze_starlark::depset::register_depset)
        .with(natives)
        .build();
    let compiled =
        blaze_starlark::eval::compile(blaze_starlark::FileKind::Bzl, "t.bzl", code.to_owned())
            .unwrap();
    let ctx = blaze_starlark::thread::ThreadContext::new(
        blaze_starlark::thread::Phase::BzlLoad,
        semantics,
        Arc::new(blaze_starlark::thread::CollectingEvents::default()),
    );
    blaze_starlark::eval::exec_module(
        compiled,
        &globals,
        &blaze_starlark::loader::LoadMap::new(),
        &ctx,
        &blaze_starlark::eval::EvalOptions::default(),
    )
    .unwrap()
}

fn arguments(module: &starlark::environment::FrozenModule, name: &str) -> (Vec<String>, bool) {
    let built = build_args(&module.get(name).unwrap(), &mut || {
        (*blaze_cmdline::repository_mapping::EMPTY).clone()
    })
    .unwrap();
    (
        built.command_line.arguments().unwrap(),
        built.param_file_info.is_some(),
    )
}

/// The review's scenario: `a.add("x"); run(arguments = [a]); a.add("y"); run(arguments = [a])`.
/// Bazel: the first action has `[x]`, the second `[x, y]`.
#[test]
fn args_added_after_the_action_was_created_do_not_reach_it() {
    let module = eval(
        "def _suffix(x):\n    return x + \"!\"\n\
         a = make_args()\n\
         a.add(\"x\")\n\
         a.add_all(depset([\"d\"]), map_each = _suffix)\n\
         first = snapshot(a)\n\
         a.add(\"y\")\n\
         a.use_param_file(\"@%s\", use_always = True)\n\
         a.set_param_file_format(\"multiline\")\n\
         second = snapshot(a)\n\
         a.add(\"z\")\n",
    );
    assert_eq!(
        arguments(&module, "first"),
        (vec!["x".to_string(), "d!".to_string()], false)
    );
    assert_eq!(
        arguments(&module, "second"),
        (
            vec!["x".to_string(), "d!".to_string(), "y".to_string()],
            true
        )
    );
    assert_eq!(arguments(&module, "a").0, ["x", "d!", "y", "z"]);
    let second = build_args(&module.get("second").unwrap(), &mut || {
        (*blaze_cmdline::repository_mapping::EMPTY).clone()
    })
    .unwrap();
    assert_eq!(
        second.parameter_file_type,
        blaze_actions::actions::parameter_file::ParameterFileType::UNQUOTED
    );
}

#[test]
fn template_dict_entries_added_after_the_action_was_created_do_not_reach_it() {
    let module = eval(
        "def _suffix(x):\n    return x + \"!\"\n\
         d = make_template_dict()\n\
         d.add(\"%a%\", \"1\")\n\
         first = snapshot(d)\n\
         d.add_joined(\"%b%\", depset([\"x\", \"y\"]), join_with = \",\", map_each = _suffix)\n\
         second = snapshot(d)\n\
         d.add(\"%c%\", \"3\")\n",
    );
    let entries = |name: &str| -> Vec<(String, String)> {
        get_all(&module.get(name).unwrap())
            .unwrap()
            .iter()
            .map(|s| (s.get_key().to_string(), s.get_value().unwrap()))
            .collect()
    };
    let kv = |k: &str, v: &str| (k.to_string(), v.to_string());
    assert_eq!(entries("first"), [kv("%a%", "1")]);
    assert_eq!(entries("second"), [kv("%a%", "1"), kv("%b%", "x!,y!")]);
    assert_eq!(entries("d").len(), 3);
}

/// A frozen `Args` cannot change any more; it is its own snapshot.
#[test]
fn snapshot_of_a_value_that_is_not_a_mutable_args_is_the_value() {
    let module = eval("x = snapshot(\"s\")\nn = snapshot(1)\n");
    assert_eq!(module.get("x").unwrap().value().unpack_str(), Some("s"));
    assert_eq!(module.get("n").unwrap().value().unpack_i32(), Some(1));
}
