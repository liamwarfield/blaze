load("//flags:defs.bzl", "BuildSettingInfo")

def _r_impl(ctx):
    return []

def _make(impl, inputs, outputs):
    return transition(implementation = impl, inputs = inputs, outputs = outputs)

def _rule_with(t):
    return rule(implementation = _r_impl, cfg = t, attrs = {"deps": attr.label_list()})

# string_set build setting as input and output (a set value).
def _set_impl(settings, attr):
    s = settings["//flags:set"]
    return {"//flags:set": s | set(["from_transition"])}

set_rule = _rule_with(_make(_set_impl, ["//flags:set"], ["//flags:set"]))

# A list given to a string_set build setting (converted to a set).
def _set_list_impl(settings, attr):
    return {"//flags:set": ["b", "a", "b"]}

set_list_rule = _rule_with(_make(_set_list_impl, [], ["//flags:set"]))

# A wrong type for a string_set build setting.
def _set_bad_impl(settings, attr):
    return {"//flags:set": "a"}

set_bad_rule = _rule_with(_make(_set_bad_impl, [], ["//flags:set"]))

# A tuple given to a native list option and to a string_list build setting.
def _tuple_impl(settings, attr):
    return {
        "//command_line_option:copt": ("-DA", "-DB"),
        "//flags:tags": ("x", "y"),
    }

tuple_rule = _rule_with(_make(_tuple_impl, [], ["//command_line_option:copt", "//flags:tags"]))

# A tuple nested in a list given to a native allowMultiple option and to a non-allowMultiple one.
def _nested_tuple_impl(settings, attr):
    return {
        "//command_line_option:copt": ["-DA", ("x", "y"), ["z"]],
    }

nested_tuple_rule = _rule_with(_make(_nested_tuple_impl, [], ["//command_line_option:copt"]))

# A dict given to a native option.
def _dict_impl(settings, attr):
    return {"//command_line_option:copt": {"a": "b"}}

dict_rule = _rule_with(_make(_dict_impl, [], ["//command_line_option:copt"]))

# A native list option read as input (list of strings) and a tuple returned for a string flag.
def _read_impl(settings, attr):
    return {"//flags:color": "copts=" + ",".join(settings["//command_line_option:copt"])}

read_rule = _rule_with(_make(_read_impl, ["//command_line_option:copt"], ["//flags:color"]))

# An invalid output directory mnemonic from a transition.
def _bad_cpu_impl(settings, attr):
    return {"//command_line_option:cpu": "//bad:cpu"}

bad_cpu_rule = _rule_with(_make(_bad_cpu_impl, [], ["//command_line_option:cpu"]))

# Exec transition through an attribute (the default exec transition).
def _exec_user_impl(ctx):
    return []

exec_user_rule = rule(
    implementation = _exec_user_impl,
    attrs = {"tool": attr.label(cfg = "exec")},
)
