"""A plain rule the probe macros instantiate."""

ProbeInfo = provider(fields = ["value"])

def _probe_rule_impl(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".txt")
    ctx.actions.write(out, "%s %s %s\n" % (ctx.label, ctx.attr.value, [str(d.label) for d in ctx.attr.deps]))
    return [DefaultInfo(files = depset([out])), ProbeInfo(value = ctx.attr.value)]

probe_rule = rule(
    implementation = _probe_rule_impl,
    attrs = {
        "deps": attr.label_list(),
        "srcs": attr.label_list(allow_files = True),
        "value": attr.string(default = "v"),
        "flag": attr.bool(),
        "count": attr.int(default = 3),
    },
)

def legacy_macro(name, **kwargs):
    """A legacy macro: instantiates a rule directly."""
    probe_rule(name = name, **kwargs)
    probe_rule(name = name + "_legacy_extra")
