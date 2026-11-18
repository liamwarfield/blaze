"""A rule that prints at analysis (DEBUG, never filtered) and an action that writes to stderr
(its output is tagged with the owner's label)."""

def _noisy_impl(ctx):
    print("analysis of %s" % ctx.label)
    out = ctx.actions.declare_file(ctx.label.name + ".out")
    ctx.actions.run_shell(
        outputs = [out],
        command = "echo stderr of %s >&2; echo done > %s" % (ctx.label.name, out.path),
    )
    return [DefaultInfo(files = depset([out]))]

noisy = rule(implementation = _noisy_impl, attrs = {"deps": attr.label_list()})
