def never_called():
    return set([1])

def _impl(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".txt")
    content = "ok"
    if ctx.attr.call:
        content = str(set([1, 2]))
    ctx.actions.write(out, content)
    return [DefaultInfo(files = depset([out]))]

probe = rule(implementation = _impl, attrs = {"call": attr.bool()})
