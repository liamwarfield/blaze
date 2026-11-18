"""A test rule whose script exits with a chosen code (TestRunnerAction, --test_keep_going)."""

def _script_test_impl(ctx):
    exe = ctx.actions.declare_file(ctx.label.name + ".sh")
    ctx.actions.write(
        exe,
        "#!/bin/sh\necho output of %s\nexit %d\n" % (ctx.label.name, ctx.attr.exit_code),
        is_executable = True,
    )
    return [DefaultInfo(executable = exe)]

script_test = rule(
    implementation = _script_test_impl,
    test = True,
    attrs = {"exit_code": attr.int()},
)
