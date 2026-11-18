"""exec review S1-5: missing source inputs of a top-level target and of an action."""

def _copy_impl(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".out")
    ctx.actions.run_shell(
        outputs = [out],
        inputs = ctx.files.srcs,
        command = 'out="$1"; shift; cat "$@" > "$out"',
        arguments = [out.path] + [f.path for f in ctx.files.srcs],
    )
    return [DefaultInfo(files = depset([out]))]

copy = rule(
    implementation = _copy_impl,
    attrs = {"srcs": attr.label_list(allow_files = True)},
)
