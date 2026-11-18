"""Non-ASCII placements in `ctx.actions.args()`, one function per case.

Each function gets a fresh `Args` (which already holds the action's output) and a File whose exec
path is not ASCII. `defs.bzl` runs them on the oracle (scripts/actions-probe/run);
crates/blaze_actions/tests/non_ascii_action_keys.rs evaluates this same file and requires the
oracle's action keys. Keep the two in step: a new case needs a new recording.
"""

def _suffix(x):
    return x + "é"

def _path_suffix(f):
    return f.path + "é"

def _add_plain(a, f):
    a.add("é")

def _add_named(a, f):
    a.add("--n", "é")

def _add_arg_name(a, f):
    a.add("é", "x")

def _add_format(a, f):
    a.add("x", format = "é%s")

def _add_all_list(a, f):
    a.add_all(["é"])

def _add_all_depset(a, f):
    a.add_all(depset(["é"]))

def _add_all_arg_name(a, f):
    a.add_all("é", ["x"])

def _format_each(a, f):
    a.add_all(["x"], format_each = "é%s")

def _before_each(a, f):
    a.add_all(["x"], before_each = "é")

def _terminate_with(a, f):
    a.add_all(["x"], terminate_with = "é")

def _join_with(a, f):
    a.add_joined(["x", "y"], join_with = "é")

def _format_joined(a, f):
    a.add_joined(["x", "y"], join_with = ",", format_joined = "é%s")

def _joined_arg_name(a, f):
    a.add_joined("é", ["x", "y"], join_with = ",")

def _map_each_list(a, f):
    a.add_all(["x"], map_each = _suffix)

def _map_each_depset(a, f):
    a.add_all(depset(["x"]), map_each = _suffix)

def _map_each_joined(a, f):
    a.add_joined(depset(["x", "y"]), join_with = "é", map_each = _suffix)

def _param_file(a, f):
    a.use_param_file("é%s", use_always = True)
    a.add("x")

def _param_file_multiline(a, f):
    a.use_param_file("@%s", use_always = True)
    a.set_param_file_format("multiline")
    a.add("é")

def _emoji(a, f):
    a.add("🐷")
    a.add_all(depset(["a🐷"]))

def _raw_byte(a, f):
    # Not UTF-8: one byte, FF.
    a.add("\377")
    a.add_all(["x\377"], format_each = "\351%s")

def _file_add(a, f):
    a.add(f)

def _file_add_all_list(a, f):
    a.add_all([f])

def _file_add_all_depset(a, f):
    a.add_all(depset([f]))

def _file_map_each(a, f):
    a.add_all(depset([f]), map_each = _path_suffix)

def _file_joined(a, f):
    a.add_joined([f, f], join_with = "é", uniquify = False)

def _ascii(a, f):
    a.add("e")
    a.add_all(depset(["x"]), map_each = _suffix, format_each = "f%s")

def _write_args(a, f):
    a.add("--y=é")

def _write_args_multiline(a, f):
    a.set_param_file_format("multiline")
    a.add("--y=é")
    a.add_all(depset(["x"]), map_each = _suffix)

def _write_args_raw_byte(a, f):
    a.add("--y=\377")

def fill_template_dict(d):
    """The `computed_substitutions` of the template_dict probe."""
    d.add("%é%", "é")
    d.add_joined("%k%", depset(["x", "é"]), join_with = "é", map_each = _suffix)

# `ctx.actions.write(output, content = <Args>)`; the Args is passed without the output.
WRITE_ARGS_CASES = {
    "write_args": _write_args,
    "write_args_multiline": _write_args_multiline,
    "write_args_raw_byte": _write_args_raw_byte,
}

# `ctx.actions.run(executable = "/bin/true", arguments = [<Args>], ...)`; the Args already holds
# the output file.
ARGS_CASES = {
    "add_plain": _add_plain,
    "add_named": _add_named,
    "add_arg_name": _add_arg_name,
    "add_format": _add_format,
    "add_all_list": _add_all_list,
    "add_all_depset": _add_all_depset,
    "add_all_arg_name": _add_all_arg_name,
    "format_each": _format_each,
    "before_each": _before_each,
    "terminate_with": _terminate_with,
    "join_with": _join_with,
    "format_joined": _format_joined,
    "joined_arg_name": _joined_arg_name,
    "map_each_list": _map_each_list,
    "map_each_depset": _map_each_depset,
    "map_each_joined": _map_each_joined,
    "param_file": _param_file,
    "param_file_multiline": _param_file_multiline,
    "emoji": _emoji,
    "raw_byte": _raw_byte,
    "file_add": _file_add,
    "file_add_all_list": _file_add_all_list,
    "file_add_all_depset": _file_add_all_depset,
    "file_map_each": _file_map_each,
    "file_joined": _file_joined,
    "ascii": _ascii,
}
