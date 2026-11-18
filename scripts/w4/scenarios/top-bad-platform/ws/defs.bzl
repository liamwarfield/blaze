"""A rule that needs a toolchain, and a toolchain implementation."""

def _needs_tt_impl(ctx):
    return []

needs_tt = rule(
    implementation = _needs_tt_impl,
    toolchains = ["//:tt"],
)

def _tc_impl(ctx):
    return [platform_common.ToolchainInfo()]

tc_impl = rule(implementation = _tc_impl)

def _to_platform_impl(settings, attr):
    return {"//command_line_option:platforms": attr.platform}

to_platform = transition(
    implementation = _to_platform_impl,
    inputs = [],
    outputs = ["//command_line_option:platforms"],
)

def _with_dep_impl(ctx):
    return []

# The dependency is built for the platform named by the `platform` attribute.
with_dep = rule(
    implementation = _with_dep_impl,
    attrs = {
        "dep": attr.label(cfg = to_platform),
        "platform": attr.string(),
    },
)
