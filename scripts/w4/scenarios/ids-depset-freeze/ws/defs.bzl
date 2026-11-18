"""A depset used by an action of the target that made it and, through a provider, by actions
of its dependents: one NestedSet in Bazel, so one depSetOfFiles id in aquery."""

HdrsInfo = provider(fields = ["hdrs"])

def _lib_impl(ctx):
    hdrs = depset(ctx.files.hdrs, transitive = [dep[HdrsInfo].hdrs for dep in ctx.attr.deps])
    out = ctx.actions.declare_file(ctx.label.name + ".list")
    args = ctx.actions.args()
    args.add_all(hdrs)
    ctx.actions.run_shell(
        inputs = hdrs,
        outputs = [out],
        arguments = [args],
        command = "echo \"$@\" > " + out.path,
        mnemonic = "ListHdrs",
    )
    return [DefaultInfo(files = depset([out])), HdrsInfo(hdrs = hdrs)]

lib = rule(
    implementation = _lib_impl,
    attrs = {
        "hdrs": attr.label_list(allow_files = True),
        "deps": attr.label_list(providers = [HdrsInfo]),
    },
)
