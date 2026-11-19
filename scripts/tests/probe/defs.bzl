"""A minimal shell test rule for the probe package (`sh_test` is not native in Bazel 9 and the
corpus project has no rules_shell dependency). Its rule class name gives test language `sh`."""

def _sh_test_impl(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".sh")
    ctx.actions.write(out, "#!/bin/sh\n" + ctx.attr.cmd + "\n", is_executable = True)
    return [DefaultInfo(executable = out)]

sh_test = rule(
    implementation = _sh_test_impl,
    attrs = {"cmd": attr.string(mandatory = True)},
    test = True,
)
