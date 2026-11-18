"""actions review S3-2: ctx.actions.write(content = args) with a directory in the Args writes the
expanded children (ParameterFileWriteAction.newDeterministicWriter uses the context's
InputMetadataProvider)."""

def _write_args_dir_impl(ctx):
    d = ctx.actions.declare_directory(ctx.label.name + ".dir")
    ctx.actions.run_shell(
        outputs = [d],
        command = 'mkdir -p "$1/sub" && echo a > "$1/a.txt" && echo b > "$1/sub/b.txt" && echo c > "$1/c.txt"',
        arguments = [d.path],
    )
    args = ctx.actions.args()
    args.add("--first")
    args.add_all([d])
    args.add_all("--prefixed", [d], format_each = "in=%s")
    args.add_all([d], expand_directories = False, before_each = "--dir")
    out = ctx.actions.declare_file(ctx.label.name + ".params")
    ctx.actions.write(out, content = args)
    return [DefaultInfo(files = depset([out]))]

write_args_dir = rule(implementation = _write_args_dir_impl)
