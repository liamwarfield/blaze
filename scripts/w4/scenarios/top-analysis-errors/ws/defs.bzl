"""Rules for the analysis-error probe."""

def _w_impl(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".txt")
    ctx.actions.write(out, ctx.label.name + "\n")
    return [DefaultInfo(files = depset([out]))]

w = rule(
    implementation = _w_impl,
    attrs = {"deps": attr.label_list()},
)

def _fails_impl(ctx):
    fail("boom from " + ctx.label.name)

fails = rule(
    implementation = _fails_impl,
    attrs = {"deps": attr.label_list()},
)

# A label attribute restricted to .txt files: a wrong file type is an attribute error raised
# through the rule context when the target is analyzed.
needs_file = rule(
    implementation = _w_impl,
    attrs = {"src": attr.label(allow_single_file = [".txt"])},
)
