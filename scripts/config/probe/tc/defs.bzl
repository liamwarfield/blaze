def _tc_impl(ctx):
    return [platform_common.ToolchainInfo(name = ctx.label.name)]

my_toolchain = rule(implementation = _tc_impl)

def _user_impl(ctx):
    ctx.toolchains["//tc:type"]
    return []

tc_user = rule(implementation = _user_impl, toolchains = ["//tc:type"])
