"""Rules for the progress and summary probe: slow and chained actions, a failing action, tests."""

def _slow_impl(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".out")
    ctx.actions.run_shell(
        inputs = ctx.files.srcs,
        outputs = [out],
        command = "sleep %d; echo %s > %s" % (ctx.attr.seconds, ctx.label.name, out.path),
        progress_message = "Sleeping for %s" % ctx.label.name,
        mnemonic = "Sleep",
    )
    return [DefaultInfo(files = depset([out]))]

slow = rule(
    implementation = _slow_impl,
    attrs = {
        "seconds": attr.int(default = 2),
        "srcs": attr.label_list(allow_files = True),
    },
)

def _failing_impl(ctx):
    f = ctx.actions.declare_file(ctx.label.name + ".out")
    ctx.actions.run_shell(
        outputs = [f],
        command = "echo failing action of %s >&2; exit 1" % ctx.label.name,
    )
    return [DefaultInfo(files = depset([f]))]

failing = rule(implementation = _failing_impl)

def _script_test_impl(ctx):
    exe = ctx.actions.declare_file(ctx.label.name + ".sh")
    ctx.actions.write(
        exe,
        "#!/bin/sh\necho output of %s\nsleep %d\nexit %d\n" % (
            ctx.label.name,
            ctx.attr.sleep,
            ctx.attr.exit_code,
        ),
        is_executable = True,
    )
    return [DefaultInfo(executable = exe)]

script_test = rule(
    implementation = _script_test_impl,
    test = True,
    attrs = {
        "exit_code": attr.int(),
        "sleep": attr.int(),
    },
)
