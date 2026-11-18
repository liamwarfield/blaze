"""Rules for the action-conflict probe."""

def _fixed_impl(ctx):
    # Every target of this rule in one package declares the same output path.
    out = ctx.actions.declare_file(ctx.attr.out)
    ctx.actions.write(out, ctx.attr.content)
    return [DefaultInfo(files = depset([out]))]

fixed = rule(
    implementation = _fixed_impl,
    attrs = {
        "out": attr.string(mandatory = True),
        "content": attr.string(),
    },
)

def _user_impl(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".user")
    ctx.actions.run_shell(
        outputs = [out],
        inputs = ctx.files.deps,
        command = "cat $@ > %s" % out.path,
        arguments = [f.path for f in ctx.files.deps],
    )
    return [DefaultInfo(files = depset([out]))]

user = rule(
    implementation = _user_impl,
    attrs = {"deps": attr.label_list(allow_files = True)},
)

def _two_impl(ctx):
    # Two actions of one target that write the same file: an intra-target conflict.
    out = ctx.actions.declare_file(ctx.label.name + ".txt")
    ctx.actions.write(out, "first")
    ctx.actions.write(out, "second")
    return [DefaultInfo(files = depset([out]))]

two = rule(implementation = _two_impl)
