BuildSettingInfo = provider(fields = ["value"])

def _flag_impl(ctx):
    return [BuildSettingInfo(value = ctx.build_setting_value)]

bool_flag = rule(implementation = _flag_impl, build_setting = config.bool(flag = True))
int_flag = rule(implementation = _flag_impl, build_setting = config.int(flag = True))
string_flag = rule(
    implementation = _flag_impl,
    build_setting = config.string(flag = True),
    attrs = {"scope": attr.string()},
)
string_list_flag = rule(implementation = _flag_impl, build_setting = config.string_list(flag = True))
multi_string_flag = rule(
    implementation = _flag_impl,
    build_setting = config.string(flag = True, allow_multiple = True),
)
repeatable_flag = rule(
    implementation = _flag_impl,
    build_setting = config.string_list(flag = True, repeatable = True),
)
string_setting = rule(implementation = _flag_impl, build_setting = config.string())
string_set_flag = rule(implementation = _flag_impl, build_setting = config.string_set(flag = True))
repeatable_set_flag = rule(
    implementation = _flag_impl,
    build_setting = config.string_set(flag = True, repeatable = True),
)

def _show_impl(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".txt")
    lines = ["%s=%r" % (f.label, f[BuildSettingInfo].value) for f in ctx.attr.flags]
    lines.append("lf=%s" % ctx.attr.lf.label)
    ctx.actions.run_shell(
        outputs = [out],
        tools = [ctx.executable.tool],
        command = "echo '%s' > %s" % ("\n".join(lines), out.path),
    )
    return [DefaultInfo(files = depset([out]))]

show = rule(
    implementation = _show_impl,
    attrs = {
        "flags": attr.label_list(),
        "lf": attr.label(),
        "tool": attr.label(executable = True, cfg = "exec"),
    },
)

def _tool_impl(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".sh")
    ctx.actions.write(out, "#!/bin/sh\necho %r\n" % ctx.attr.flag[BuildSettingInfo].value, is_executable = True)
    return [DefaultInfo(executable = out)]

tool = rule(implementation = _tool_impl, executable = True, attrs = {"flag": attr.label()})
