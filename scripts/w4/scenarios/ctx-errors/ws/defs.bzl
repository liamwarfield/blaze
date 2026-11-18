"""Errors Bazel raises for ctx fields and for mistyped parameters of ctx.runfiles / ctx.actions.run."""

def _err_impl(ctx):
    op = ctx.attr.op
    out = ctx.actions.declare_file(ctx.label.name + ".txt")
    tmp = ctx.actions.declare_file(ctx.label.name + ".tmp")
    r = "unknown op"
    ran = False

    # Undeclared and unknown fields.
    if op == "frag_undeclared":
        r = str(ctx.fragments.cpp)
    elif op == "frag_undeclared_getattr_default":
        r = str(getattr(ctx.fragments, "cpp", None))
    elif op == "frag_undeclared_hasattr":
        r = str(hasattr(ctx.fragments, "cpp"))
    elif op == "frag_unknown":
        r = str(ctx.fragments.nosuch)
    elif op == "frag_unknown_getattr_default":
        r = str(getattr(ctx.fragments, "nosuch", "default"))
    elif op == "frag_declared":
        r = str(type(ctx.fragments.platform))
    elif op == "outputs_unknown":
        r = str(ctx.outputs.nosuch)
    elif op == "outputs_unknown_getattr_default":
        r = str(getattr(ctx.outputs, "nosuch", "default"))
    elif op == "attr_unknown":
        r = str(ctx.attr.nosuch)
    elif op == "attr_unknown_getattr_default":
        r = str(getattr(ctx.attr, "nosuch", "default"))

        # ctx.runfiles parameter types.
    elif op == "runfiles_symlinks_list":
        r = str(ctx.runfiles(symlinks = []))
    elif op == "runfiles_root_symlinks_list":
        r = str(ctx.runfiles(root_symlinks = ["x"]))
    elif op == "runfiles_collect_data_int":
        r = str(ctx.runfiles(collect_data = 1))
    elif op == "runfiles_collect_default_none":
        r = str(ctx.runfiles(collect_default = None))
    elif op == "runfiles_ok":
        rf = ctx.runfiles(files = [], collect_data = True, collect_default = False, symlinks = {}, root_symlinks = depset())
        r = str(len(rf.files.to_list()))

        # ctx.actions.run / run_shell parameter types.
    elif op == "run_use_default_shell_env_int":
        ctx.actions.run(outputs = [tmp], executable = "/bin/true", use_default_shell_env = 1)
        ran = True
    elif op == "run_shell_use_default_shell_env_none":
        ctx.actions.run_shell(outputs = [tmp], command = "true", use_default_shell_env = None)
        ran = True
    elif op == "run_toolchain_int":
        ctx.actions.run(outputs = [tmp], executable = "/bin/true", toolchain = 1)
        ran = True
    elif op == "run_shell_toolchain_list":
        ctx.actions.run_shell(outputs = [tmp], command = "true", toolchain = [])
        ran = True
    elif op == "run_outputs_depset":
        ctx.actions.run(outputs = depset([tmp]), executable = "/bin/true")
        ran = True
    elif op == "run_inputs_string":
        ctx.actions.run(outputs = [tmp], inputs = "x", executable = "/bin/true")
        ran = True
    elif op == "run_env_list":
        ctx.actions.run(outputs = [tmp], executable = "/bin/true", env = [])
        ran = True
    elif op == "run_tools_string":
        ctx.actions.run(outputs = [tmp], executable = "/bin/true", tools = "x")
        ran = True
    elif op == "run_unused_inputs_list_string":
        ctx.actions.run(outputs = [tmp], executable = "/bin/true", unused_inputs_list = "x")
        ran = True
    elif op == "run_resource_set_int":
        ctx.actions.run(outputs = [tmp], executable = "/bin/true", resource_set = 1)
        ran = True
    elif op == "run_outputs_strings":
        ctx.actions.run(outputs = ["x"], executable = "/bin/true")
        ran = True
    elif op == "run_ok":
        ctx.actions.run(
            outputs = [tmp],
            executable = "/bin/true",
            use_default_shell_env = True,
            env = {"A": "b"},
            toolchain = None,
            resource_set = {"cpu": 2, "memory": 100},
        )
        ran = True
        r = "ok"
    if not ran:
        ctx.actions.write(tmp, "")
    ctx.actions.write(out, r + "\n")
    return [DefaultInfo(files = depset([out]))]

err = rule(
    implementation = _err_impl,
    attrs = {"op": attr.string()},
    fragments = ["platform"],
)

OPS = [
    "frag_undeclared",
    "frag_undeclared_getattr_default",
    "frag_undeclared_hasattr",
    "frag_unknown",
    "frag_unknown_getattr_default",
    "frag_declared",
    "outputs_unknown",
    "outputs_unknown_getattr_default",
    "attr_unknown",
    "attr_unknown_getattr_default",
    "runfiles_symlinks_list",
    "runfiles_root_symlinks_list",
    "runfiles_collect_data_int",
    "runfiles_collect_default_none",
    "runfiles_ok",
    "run_use_default_shell_env_int",
    "run_shell_use_default_shell_env_none",
    "run_toolchain_int",
    "run_shell_toolchain_list",
    "run_outputs_depset",
    "run_inputs_string",
    "run_env_list",
    "run_tools_string",
    "run_unused_inputs_list_string",
    "run_resource_set_int",
    "run_outputs_strings",
    "run_ok",
]
