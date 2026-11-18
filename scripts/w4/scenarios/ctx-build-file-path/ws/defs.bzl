def _impl(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".txt")
    ctx.actions.write(out, ctx.build_file_path + "\n")
    return [DefaultInfo(files = depset([out]))]

bfp = rule(implementation = _impl)
