"""Rules for the query-rdeps scenario: one write action per target."""

def _write(ctx, deps):
    out = ctx.actions.declare_file(ctx.label.name + ".txt")
    ctx.actions.write(out, "\n".join([str(d.label) for d in deps]) + "\n")
    return [DefaultInfo(files = depset([out]))]

def _tool_impl(ctx):
    return _write(ctx, [])

tool = rule(implementation = _tool_impl)

def _implicit_user_impl(ctx):
    return _write(ctx, [ctx.attr._tool])

implicit_user = rule(
    implementation = _implicit_user_impl,
    attrs = {"_tool": attr.label(default = "//a:tool")},
)

def _explicit_user_impl(ctx):
    return _write(ctx, ctx.attr.deps)

explicit_user = rule(
    implementation = _explicit_user_impl,
    attrs = {"deps": attr.label_list()},
)

def _reader_impl(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".txt")
    ctx.actions.run_shell(
        outputs = [out],
        inputs = ctx.files.srcs,
        command = "cat \"$@\" > " + out.path,
        arguments = [f.path for f in ctx.files.srcs],
        mnemonic = "Reader",
    )
    return [DefaultInfo(files = depset([out]))]

reader = rule(
    implementation = _reader_impl,
    attrs = {"srcs": attr.label_list(allow_files = True)},
)
