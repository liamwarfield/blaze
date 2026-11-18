"""Rules for the build result probe: targets with chosen outputs, a failing action, tests."""

def _files_impl(ctx):
    outs = []
    for name in ctx.attr.outs:
        f = ctx.actions.declare_file(name)
        ctx.actions.write(f, name + "\n")
        outs.append(f)
    extra = []
    for name in ctx.attr.extra:
        f = ctx.actions.declare_file(name)
        ctx.actions.write(f, name + "\n")
        extra.append(f)
    providers = [DefaultInfo(files = depset(outs))]
    if extra:
        providers.append(OutputGroupInfo(extra = depset(extra)))
    return providers

files = rule(
    implementation = _files_impl,
    attrs = {
        "outs": attr.string_list(),
        "extra": attr.string_list(),
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

def _analysis_error_impl(ctx):
    fail("analysis error of " + ctx.label.name)

analysis_error = rule(implementation = _analysis_error_impl)

def _script_test_impl(ctx):
    exe = ctx.actions.declare_file(ctx.label.name + ".sh")
    ctx.actions.write(
        exe,
        "#!/bin/sh\necho output of %s\nexit %d\n" % (ctx.label.name, ctx.attr.exit_code),
        is_executable = True,
    )
    return [DefaultInfo(executable = exe, runfiles = ctx.runfiles(files = ctx.files.data))]

script_test = rule(
    implementation = _script_test_impl,
    test = True,
    attrs = {
        "exit_code": attr.int(),
        "data": attr.label_list(allow_files = True),
    },
)

analysis_error_test = rule(
    implementation = _analysis_error_impl,
    test = True,
)
