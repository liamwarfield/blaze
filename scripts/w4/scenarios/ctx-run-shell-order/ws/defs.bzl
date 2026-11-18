"""The helper script of a long run_shell command is registered in place (StarlarkActionFactory.runShell)."""

def _order_impl(ctx):
    def out(suffix):
        return ctx.actions.declare_file(ctx.label.name + "." + suffix)

    a = out("a")
    ctx.actions.write(a, "x")

    # A short command: no helper script, but it takes helper script number 0.
    s = out("short")
    ctx.actions.run_shell(outputs = [s], command = "echo short > " + s.path)

    # More than 64000 characters: written to <name>.run_shell_1.sh, registered before the action.
    l = out("long")
    ctx.actions.run_shell(
        outputs = [l],
        inputs = [a],
        arguments = ["arg"],
        command = "echo long > " + l.path + " #" + "a" * 64000,
    )

    b = out("b")
    ctx.actions.write(b, "y")
    return [DefaultInfo(files = depset([a, s, l, b]))]

order = rule(implementation = _order_impl)
