"""An action whose command line cannot be expanded: map_each returns a non-string."""

def _bad(f):
    return 1

def _good(f):
    return f.path

def _r_impl(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".out")
    args = ctx.actions.args()
    args.add_all(ctx.files.srcs, map_each = _bad if ctx.attr.bad else _good)
    ctx.actions.run_shell(
        outputs = [out],
        inputs = ctx.files.srcs,
        command = "echo \"$@\" > " + out.path,
        arguments = [args],
        mnemonic = "MapEach",
    )
    return [DefaultInfo(files = depset([out]))]

r = rule(
    implementation = _r_impl,
    attrs = {
        "srcs": attr.label_list(allow_files = True),
        "bad": attr.bool(),
    },
)
