"""rule() calls in the main repository."""

load("@rules_a//:impl.bzl", "impl", "lookup")

# The implementation lives in @rules_a, the rule is defined here.
remote_impl = rule(
    implementation = impl,
    toolchains = [config_common.toolchain_type("@rules_a//x:type", mandatory = False)],
)

def _local_impl(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".txt")
    tc = ctx.toolchains["//x:type"]
    ctx.actions.write(out, "toolchain: %s, in: %s\n" % (tc, "//x:type" in ctx.toolchains))
    return [DefaultInfo(files = depset([out]))]

local_impl = rule(
    implementation = _local_impl,
    toolchains = [config_common.toolchain_type("//x:type", mandatory = False)],
)

def _local_wrong_impl(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".txt")

    # "//x:type" is the main repository's type here; the rule only requests @rules_a's.
    ctx.actions.write(out, "toolchain: %s\n" % ctx.toolchains["//x:type"])
    return [DefaultInfo(files = depset([out]))]

local_wrong = rule(
    implementation = _local_wrong_impl,
    toolchains = [config_common.toolchain_type("@rules_a//x:type", mandatory = False)],
)

def _helper_impl(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".txt")

    # The key is converted inside lookup(), a function of @rules_a//:impl.bzl.
    ctx.actions.write(out, lookup(ctx, "//x:type"))
    return [DefaultInfo(files = depset([out]))]

helper = rule(
    implementation = _helper_impl,
    toolchains = [config_common.toolchain_type("@rules_a//x:type", mandatory = False)],
)
