"""Rules whose actions carry non-ASCII strings (byte-string semantics, docs/research/07)."""

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
        command = "printf '%s|%s' \"$1\" \"$ENV_E\" > \"$2\"",
        env = {"ENV_E": "é"},
        mnemonic = "ShellE",
        progress_message = "Shelling é %{output}",
    )
    return [DefaultInfo(files = depset([out]))]

shell_rule = rule(
    implementation = _shell_impl,
    attrs = {"arg": attr.string()},
)

def _fail_impl(ctx):
    fail("failing with é🐷 and " + repr(ctx.attr.msg))

fail_rule = rule(
    implementation = _fail_impl,
    attrs = {"msg": attr.string()},
)

def report():
    e = "é"
    pig = "🐷"
    print("PRINT %s%s|" % (e, pig) + str(len(e + pig)))
    print("REPR " + repr(e + "\377") + " " + str([pig]))
    print("UPPER " + (e + "a").upper() + " " + str((e + pig).elems()[0] == "\303"))
    print("GLOB " + str(native.glob(["*.txt"])))
