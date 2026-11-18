def _w_impl(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".txt")
    ctx.actions.write(out, "x\n")
    return [DefaultInfo(files = depset([out]))]

w = rule(implementation = _w_impl, attrs = {"deps": attr.label_list()})
