"""Which values of ctx are immutable (StarlarkList.immutableCopyOf / Dict.immutableCopyOf)."""

def _mut_impl(ctx):
    op = ctx.attr.op
    out = ctx.actions.declare_file(ctx.label.name + ".txt")
    r = "unknown op"
    if op == "files_append":
        ctx.files.srcs.append(ctx.files.srcs[0])
        r = str(len(ctx.files.srcs))
    elif op == "files_empty_append":
        ctx.files.empty.append(ctx.files.srcs[0])
        r = str(len(ctx.files.empty))
    elif op == "attr_label_list_append":
        ctx.attr.srcs.append(ctx.attr.srcs[0])
        r = str(len(ctx.attr.srcs))
    elif op == "attr_string_list_append":
        ctx.attr.strs.append("x")
        r = str(ctx.attr.strs)
    elif op == "attr_string_dict_set":
        ctx.attr.sdict["k2"] = "v2"
        r = str(ctx.attr.sdict)
    elif op == "attr_label_keyed_string_dict_clear":
        ctx.attr.lkd.clear()
        r = str(len(ctx.attr.lkd))
    elif op == "attr_string_keyed_label_dict_set":
        ctx.attr.skl["k2"] = None
        r = str(len(ctx.attr.skl))
    elif op == "outputs_list_append":
        ctx.outputs.outs.append(out)
        r = str(len(ctx.outputs.outs))
    elif op == "var_set":
        ctx.var["PROBE"] = "y"
        r = ctx.var["PROBE"]
    elif op == "tokenize_append":
        t = ctx.tokenize("a b")
        t.append("c")
        r = str(t)
    elif op == "features_append":
        f = ctx.features
        f.append("zz")
        r = str(f) + " " + str(ctx.features)
    elif op == "disabled_features_append":
        f = ctx.disabled_features
        f.append("zz")
        r = str(f) + " " + str(ctx.disabled_features)
    elif op == "toolchain_types_append":
        t = ctx.toolchains.toolchain_types()
        t.append(Label("//:x"))
        r = str(t)
    elif op == "read_only":
        r = "%d %d %s %s %s" % (
            len(ctx.files.srcs),
            len(ctx.attr.srcs),
            ctx.attr.strs,
            ctx.attr.sdict,
            ctx.tokenize("a 'b c'"),
        )
    ctx.actions.write(out, r + "\n")
    for o in ctx.outputs.outs:
        ctx.actions.write(o, "o\n")
    return [DefaultInfo(files = depset([out]))]

mut = rule(
    implementation = _mut_impl,
    attrs = {
        "op": attr.string(),
        "srcs": attr.label_list(allow_files = True),
        "empty": attr.label_list(allow_files = True),
        "strs": attr.string_list(),
        "sdict": attr.string_dict(),
        "lkd": attr.label_keyed_string_dict(allow_files = True),
        "skl": attr.string_keyed_label_dict(allow_files = True),
        "outs": attr.output_list(),
    },
)

OPS = [
    "read_only",
    "files_append",
    "files_empty_append",
    "attr_label_list_append",
    "attr_string_list_append",
    "attr_string_dict_set",
    "attr_label_keyed_string_dict_clear",
    "attr_string_keyed_label_dict_set",
    "outputs_list_append",
    "var_set",
    "tokenize_append",
    "features_append",
    "disabled_features_append",
    "toolchain_types_append",
]
