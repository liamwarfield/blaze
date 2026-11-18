"""Two targets register equal actions for one output (shared actions, as in Bazel's
RewindingTestsHelper.setUpParallelTrackSharedActionPackage), a file and a tree artifact."""

def _shared_impl(ctx):
    in_file = ctx.file.src
    a_shared_out = ctx.actions.declare_file("A-shared.out")
    ctx.actions.run_shell(
        inputs = [in_file],
        outputs = [a_shared_out],
        progress_message = "Copying %s input %s to A-shared.out" % (ctx.attr.name, in_file.short_path),
        command = "cp %s %s" % (in_file.path, a_shared_out.path),
    )
    tree = ctx.actions.declare_directory("shared-tree")
    ctx.actions.run_shell(
        inputs = [a_shared_out],
        outputs = [tree],
        command = "mkdir -p %s/d && cp %s %s/d/copy.txt && echo x > %s/x.txt" % (
            tree.path,
            a_shared_out.path,
            tree.path,
            tree.path,
        ),
    )
    out = ctx.outputs.out
    ctx.actions.run_shell(
        inputs = [a_shared_out, tree],
        outputs = [out],
        command = "(cat %s; ls -R %s | sed 's|^%s|TREE|') > %s" % (
            a_shared_out.path,
            tree.path,
            tree.path,
            out.path,
        ),
    )
    return [DefaultInfo(files = depset([out]))]

shared = rule(
    implementation = _shared_impl,
    attrs = {
        "src": attr.label(mandatory = True, allow_single_file = True),
        "out": attr.output(mandatory = True),
    },
)
