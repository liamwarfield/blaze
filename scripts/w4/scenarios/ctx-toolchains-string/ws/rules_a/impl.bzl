"""An implementation function and a helper that index ctx.toolchains with a string."""

def impl(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".txt")

    # A string key is resolved against the innermost .bzl on the call stack: this file, so
    # "//x:type" is @rules_a//x:type (StarlarkToolchainContext.transformKey).
    tc = ctx.toolchains["//x:type"]
    ctx.actions.write(out, "toolchain: %s, in: %s\n" % (tc, "//x:type" in ctx.toolchains))
    return [DefaultInfo(files = depset([out]))]

def lookup(ctx, key):
    return "toolchain: %s, in: %s\n" % (ctx.toolchains[key], key in ctx.toolchains)
