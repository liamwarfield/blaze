"""Rules for the query-order scenario: one action per target, an exec-configured implicit tool."""

def _tool_impl(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".sh")
    ctx.actions.write(out, "#!/bin/sh\ncat \"$@\"\n", is_executable = True)
    return [DefaultInfo(executable = out)]

tool = rule(implementation = _tool_impl, executable = True)

def _lib_impl(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".txt")
    transitive = [d[DefaultInfo].files for d in ctx.attr.deps]
    args = ctx.actions.args()
    args.add_all(ctx.files.srcs)
    ctx.actions.run_shell(
        outputs = [out],
        inputs = depset(ctx.files.srcs, transitive = transitive),
        tools = [ctx.executable._tool],
        command = ctx.executable._tool.path + " \"$@\" > " + out.path,
        arguments = [args],
        mnemonic = "Lib",
    )
    return [DefaultInfo(files = depset([out], transitive = transitive))]

lib = rule(
    implementation = _lib_impl,
    attrs = {
        "srcs": attr.label_list(allow_files = True),
        "deps": attr.label_list(),
        "_tool": attr.label(default = "//tools:tool", executable = True, cfg = "exec"),
    },
)
