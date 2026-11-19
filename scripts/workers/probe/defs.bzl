"""A rule whose action runs tools/worker.py, as a persistent worker where the strategy allows."""

def _worker_action_impl(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".out")
    startup = ctx.actions.args()
    if ctx.attr.protocol == "json":
        startup.add("--protocol=json")
    if ctx.attr.multiplex:
        startup.add("--multiplex")
    startup.add_all(ctx.attr.startup_flags)
    work = ctx.actions.args()
    work.add("--in", ctx.file.src)
    work.add("--out", out)
    work.add("--mode", ctx.attr.mode)
    work.use_param_file(ctx.attr.flagfile_format, use_always = True)
    work.set_param_file_format("multiline")
    reqs = {}
    if ctx.attr.multiplex:
        reqs["supports-multiplex-workers"] = "1"
    else:
        reqs["supports-workers"] = "1"
    if ctx.attr.protocol:
        reqs["requires-worker-protocol"] = ctx.attr.protocol
    if ctx.attr.multiplex_sandboxing:
        reqs["supports-multiplex-sandboxing"] = "1"
    if ctx.attr.cancellable:
        reqs["supports-worker-cancellation"] = "1"
    reqs.update(ctx.attr.extra_requirements)
    ctx.actions.run(
        executable = ctx.executable._worker,
        arguments = [startup, work],
        inputs = [ctx.file.src],
        outputs = [out],
        mnemonic = ctx.attr.action_mnemonic,
        progress_message = "Worker probe %{label}",
        execution_requirements = reqs,
    )
    return [DefaultInfo(files = depset([out]))]

worker_action = rule(
    implementation = _worker_action_impl,
    attrs = {
        "src": attr.label(allow_single_file = True, mandatory = True),
        "mode": attr.string(default = "ok"),
        "protocol": attr.string(default = ""),
        "multiplex": attr.bool(default = False),
        "multiplex_sandboxing": attr.bool(default = False),
        "cancellable": attr.bool(default = False),
        "startup_flags": attr.string_list(),
        "extra_requirements": attr.string_dict(),
        "flagfile_format": attr.string(default = "@%s"),
        "action_mnemonic": attr.string(default = "ProbeWork"),
        "_worker": attr.label(
            default = "//tools:worker.py",
            executable = True,
            allow_single_file = True,
            cfg = "exec",
        ),
    },
)
