"""Rules whose actions carry non-ASCII strings."""

def _write_impl(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".txt")
    ctx.actions.write(out, ctx.attr.content)
    return [DefaultInfo(files = depset([out]))]

write_rule = rule(
    implementation = _write_impl,
    attrs = {"content": attr.string()},
)

def _shell_impl(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".txt")
    args = ctx.actions.args()
    args.add(ctx.attr.arg)
    args.add(out)
    ctx.actions.run_shell(
        outputs = [out],
        arguments = [args],
        command = "printf '%s' \"$1\" > \"$2\"",
        env = {"ENV_E": "é"},
        mnemonic = "ShellE",
    )
    return [DefaultInfo(files = depset([out]))]

shell_rule = rule(
    implementation = _shell_impl,
    attrs = {"arg": attr.string()},
)
