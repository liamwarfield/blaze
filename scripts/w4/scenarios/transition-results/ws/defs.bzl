"""Results of a split transition that StarlarkDefinedConfigTransition.evaluate must classify."""

def _t_impl(settings, attr):
    k = attr.kind
    if k == "dict_of_dicts_int_key":
        # Inner Dict.cast fails: Java falls through to the patch-transition attempt.
        return {"a": {1: "x"}}
    if k == "dict_of_dicts_second_int_key":
        return {"a": {"//command_line_option:copt": ["-Da"]}, "b": {2: "x"}}
    if k == "dict_of_dicts_ok":
        return {"a": {"//command_line_option:copt": ["-Da"]}, "b": {"//command_line_option:copt": ["-Db"]}}
    if k == "dict_of_dicts_first_undeclared":
        return {"a": {"//command_line_option:cpu": "x"}, "b": {3: "x"}}
    return {}

t = transition(
    implementation = _t_impl,
    inputs = [],
    outputs = ["//command_line_option:copt"],
)

def _r_impl(ctx):
    return []

r = rule(
    implementation = _r_impl,
    attrs = {
        "kind": attr.string(),
        "dep": attr.label(cfg = t),
    },
)
