"""Rules with subrules."""

SubInfo = provider(fields = ["msg"])

def _write_sub_impl(ctx, prefix, _tool):
    out = ctx.actions.declare_file(ctx.label.name + "_" + prefix + ".txt")
    ctx.actions.write(out, prefix + " " + _tool.short_path + "\n")
    return out

write_sub = subrule(
    implementation = _write_sub_impl,
    attrs = {"_tool": attr.label(default = "//macro_probe/sub:tool_file", allow_single_file = True)},
)

def _nested_sub_impl(ctx, x):
    return write_sub(prefix = "nested_" + x)

nested_sub = subrule(implementation = _nested_sub_impl, subrules = [write_sub])

def _with_rule_impl(ctx):
    a = write_sub(prefix = "first")
    b = nested_sub(x = ctx.attr.x)
    return [DefaultInfo(files = depset([a, b])), SubInfo(msg = ctx.attr.x)]

with_subrules = rule(
    implementation = _with_rule_impl,
    attrs = {"x": attr.string(default = "x")},
    subrules = [write_sub, nested_sub],
)

def _bad_rule_impl(ctx):
    # write_sub is not declared by this rule.
    return [DefaultInfo(files = depset([write_sub(prefix = "bad")]))]

bad_subrule_use = rule(implementation = _bad_rule_impl)
