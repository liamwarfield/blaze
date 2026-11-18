"""exec review S2-3 / S3-1 / S3-10: tree artifact values (TreeArtifactValue.visitTree: symlinks
inside the tree are followed, dangling symlinks and special files are errors, every visited entry
is chmod'ed to the output mode) and tree artifacts with the action cache."""

def _tree_impl(ctx):
    d = ctx.actions.declare_directory(ctx.label.name + ".dir")

    # Unsandboxed, so that each execution leaves a line in the workspace (producer.log): an
    # action cache hit leaves none.
    ctx.actions.run_shell(
        outputs = [d],
        inputs = [ctx.file.marker],
        command = """
set -e
mkdir -p "$1/sub/deep" "$1/empty"
echo top > "$1/top.txt"
echo nested > "$1/sub/n.txt"
echo deep > "$1/sub/deep/d.txt"
ln -s sub "$1/link_to_sub"
ln -s top.txt "$1/link_to_file"
echo run >> "$(dirname "$(readlink -f "$2")")/producer.log"
""",
        arguments = [d.path, ctx.file.marker.path],
        execution_requirements = {"no-sandbox": "1"},
    )
    return [DefaultInfo(files = depset([d]))]

tree = rule(
    implementation = _tree_impl,
    attrs = {"marker": attr.label(allow_single_file = True, default = "//:marker.txt")},
)

def _consume_impl(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".txt")
    args = ctx.actions.args()
    args.add_all([ctx.file.tree])
    ctx.actions.run_shell(
        outputs = [out],
        inputs = [ctx.file.tree, ctx.file.marker],
        command = """
out="$1"; marker="$2"; shift; shift
for f in "$@"; do echo "$f: $(cat "$f")"; done > "$out"
echo run >> "$(dirname "$(readlink -f "$marker")")/consumer.log"
""",
        arguments = [out.path, ctx.file.marker.path, args],
        execution_requirements = {"no-sandbox": "1"},
    )
    return [DefaultInfo(files = depset([out]))]

consume = rule(
    implementation = _consume_impl,
    attrs = {
        "tree": attr.label(allow_single_file = True),
        "marker": attr.label(allow_single_file = True, default = "//:marker.txt"),
    },
)

def _sandboxed_consume_impl(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".txt")
    ctx.actions.run_shell(
        outputs = [out],
        inputs = [ctx.file.tree],
        command = '(cd "$1" && find . | LC_ALL=C sort) > "$2"',
        arguments = [ctx.file.tree.path, out.path],
    )
    return [DefaultInfo(files = depset([out]))]

sandboxed_consume = rule(
    implementation = _sandboxed_consume_impl,
    attrs = {"tree": attr.label(allow_single_file = True)},
)

def _bad_tree_impl(ctx):
    d = ctx.actions.declare_directory(ctx.label.name + ".dir")
    ctx.actions.run_shell(
        outputs = [d],
        command = ctx.attr.cmd,
        arguments = [d.path],
        execution_requirements = {"no-sandbox": "1"},
    )
    return [DefaultInfo(files = depset([d]))]

bad_tree = rule(
    implementation = _bad_tree_impl,
    attrs = {"cmd": attr.string()},
)
