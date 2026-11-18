"""Rules whose dependencies resolve in different numbers of Skyframe lookup rounds."""

def _flag_impl(ctx):
    return []

flag = rule(
    implementation = _flag_impl,
    build_setting = config.string(flag = True),
)

def _set_flag_impl(settings, attr):
    return {"//pkg:flag": "changed"}

set_flag = transition(
    implementation = _set_flag_impl,
    inputs = [],
    outputs = ["//pkg:flag"],
)

def _leaf_impl(ctx):
    return []

leaf = rule(implementation = _leaf_impl)

def _tc_impl(ctx):
    return [platform_common.ToolchainInfo()]

tc_impl = rule(implementation = _tc_impl)

def _node_impl(ctx):
    return []

# Attribute order is the declaration order below.
node = rule(
    implementation = _node_impl,
    attrs = {
        "exec_dep": attr.label(cfg = "exec"),
        "target_deps": attr.label_list(),
        "none_dep": attr.label(cfg = config.none()),
        "srcs": attr.label_list(allow_files = True),
        "starlark_dep": attr.label(cfg = set_flag),
        "last_dep": attr.label(),
        "none_last": attr.label(cfg = config.none()),
    },
    toolchains = ["//pkg:tt"],
)
