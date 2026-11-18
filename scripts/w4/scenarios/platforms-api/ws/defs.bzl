"""Fields gated by --experimental_platforms_api (enableOnlyWithFlag): hidden from getattr,
hasattr and dir when the flag is off."""

def _impl(ctx):
    p = ctx.attr.platform[platform_common.PlatformInfo]
    s = ctx.attr.setting[platform_common.ConstraintSettingInfo]
    v = ctx.attr.value[platform_common.ConstraintValueInfo]
    op = ctx.attr.op
    lines = []
    if op == "inspect":
        for name, x in [("platform", p), ("setting", s), ("value", v)]:
            lines.append("%s dir %s" % (name, dir(x)))
        for name, x, field in [
            ("platform", p, "label"),
            ("platform", p, "constraints"),
            ("setting", s, "label"),
            ("setting", s, "default_constraint_value"),
            ("setting", s, "has_default_constraint_value"),
            ("value", v, "label"),
            ("value", v, "constraint"),
        ]:
            lines.append("%s hasattr %s %s" % (name, field, hasattr(x, field)))
            lines.append("%s getattr %s %s" % (name, field, getattr(x, field, "<default>")))
    elif op == "platform_label":
        lines.append(str(p.label))
    elif op == "setting_label":
        lines.append(str(s.label))
    elif op == "value_constraint":
        lines.append(str(v.constraint))
    elif op == "constraints_get":
        lines.append(str(p.constraints.get(s)))
        lines.append(str(p.constraints.has(s)))
        lines.append(str(p.constraints.has_constraint_value(v)))
        lines.append(str(p.constraints.constraint_settings))
        lines.append(str(s in p.constraints))
        lines.append(str(p.constraints[s]))
    out = ctx.actions.declare_file(ctx.label.name + ".txt")
    ctx.actions.write(out, "\n".join(lines) + "\n")
    return [DefaultInfo(files = depset([out]))]

probe = rule(
    implementation = _impl,
    attrs = {
        "op": attr.string(),
        "platform": attr.label(),
        "setting": attr.label(),
        "value": attr.label(),
    },
)
