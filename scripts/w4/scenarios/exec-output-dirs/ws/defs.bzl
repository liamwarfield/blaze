"""Outputs whose paths collide across builds (ActionOutputDirectoryHelper)."""

def _file_impl(ctx):
    out = ctx.actions.declare_file(ctx.attr.path)
    ctx.actions.run_shell(
        outputs = [out],
        command = "echo %s > %s" % (ctx.label.name, out.path),
    )
    return [DefaultInfo(files = depset([out]))]

file_at = rule(implementation = _file_impl, attrs = {"path": attr.string()})

def _tree_impl(ctx):
    out = ctx.actions.declare_directory(ctx.attr.path)
    ctx.actions.run_shell(
        outputs = [out],
        command = "mkdir -p %s/inner && echo a > %s/a.txt && echo b > %s/inner/b.txt" % (
            out.path,
            out.path,
            out.path,
        ),
    )
    return [DefaultInfo(files = depset([out]))]

tree_at = rule(implementation = _tree_impl, attrs = {"path": attr.string()})

def _symlink_impl(ctx):
    out = ctx.actions.declare_file(ctx.attr.path)
    ctx.actions.symlink(output = out, target_file = ctx.file.target)
    return [DefaultInfo(files = depset([out]))]

symlink_at = rule(
    implementation = _symlink_impl,
    attrs = {"path": attr.string(), "target": attr.label(allow_single_file = True)},
)
