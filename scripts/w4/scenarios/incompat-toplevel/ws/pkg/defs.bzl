"""Rules for the incompatible-target scenario."""

def _simple_impl(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".out")
    ctx.actions.write(out, ctx.label.name + "\n")
    return [DefaultInfo(files = depset([out] + ctx.files.deps))]

simple = rule(
    implementation = _simple_impl,
    attrs = {"deps": attr.label_list(allow_files = True)},
)

def _simple_test_impl(ctx):
    exe = ctx.actions.declare_file(ctx.label.name + ".sh")
    ctx.actions.write(exe, "#!/bin/sh\nexit 0\n", is_executable = True)
    return [DefaultInfo(executable = exe)]

simple_test = rule(
    implementation = _simple_test_impl,
    attrs = {"deps": attr.label_list(allow_files = True)},
    test = True,
)
