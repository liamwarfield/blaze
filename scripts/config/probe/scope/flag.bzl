BuildSettingInfo = provider(fields = ["value"])

def _impl(ctx):
    return BuildSettingInfo(value = ctx.build_setting_value)

sample_flag = rule(
    implementation = _impl,
    build_setting = config.string(flag = True),
    attrs = {"scope": attr.string(default = "universal")},
)

def _t_impl(settings, attr):
    return {
        "//scope/in:project_flag": "changed",
        "//scope/out:out_flag": "changed",
        "//scope/in:universal_flag": "changed",
    }

_t = transition(
    implementation = _t_impl,
    inputs = [],
    outputs = ["//scope/in:project_flag", "//scope/out:out_flag", "//scope/in:universal_flag"],
)

def _r_impl(ctx):
    return []

transitioned = rule(
    implementation = _r_impl,
    cfg = _t,
    attrs = {"deps": attr.label_list()},
)

plain = rule(implementation = _r_impl, attrs = {"deps": attr.label_list()})
