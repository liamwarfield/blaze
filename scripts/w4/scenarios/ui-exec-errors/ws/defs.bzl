"""Rules for the execution error probe: failing actions, consumers of their outputs, tests."""

def _files_impl(ctx):
    outs = []
    for name in ctx.attr.outs:
        f = ctx.actions.declare_file(name)
        ctx.actions.write(f, name + "\n")
        outs.append(f)
    return [DefaultInfo(files = depset(outs))]

files = rule(
    implementation = _files_impl,
    attrs = {"outs": attr.string_list()},
)

def _failing_impl(ctx):
    outs = [ctx.actions.declare_file(ctx.label.name + "." + ext) for ext in ctx.attr.exts]
    ctx.actions.run_shell(
        outputs = outs,
        command = "echo failing action of %s >&2; exit 1" % ctx.label.name,
    )
    return [DefaultInfo(files = depset(outs))]

failing = rule(
    implementation = _failing_impl,
    attrs = {"exts": attr.string_list(default = ["out"])},
)

def _concat_impl(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".cat")
    ctx.actions.run_shell(
        inputs = ctx.files.srcs,
        outputs = [out],
        command = "cat %s > %s" % (" ".join([f.path for f in ctx.files.srcs]), out.path),
    )
    return [DefaultInfo(files = depset([out]))]

concat = rule(
    implementation = _concat_impl,
    attrs = {"srcs": attr.label_list(allow_files = True)},
)

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
