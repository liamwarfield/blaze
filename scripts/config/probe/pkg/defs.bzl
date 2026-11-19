load("//flags:defs.bzl", "BuildSettingInfo")

def _impl(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".txt")
    ctx.actions.write(out, "x")
    return [DefaultInfo(files = depset([out]))]

leaf = rule(implementation = _impl, attrs = {"deps": attr.label_list()})
