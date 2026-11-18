"""Natives that Bazel hashes by identity (or by value) used as dict keys and depset elements."""

load("@rules_cc//cc/common:cc_info.bzl", "CcInfo")

def _hash_impl(ctx):
    cpp = ctx.fragments.cpp
    d = {cpp: 1}
    d[ctx.fragments.cpp] = 2
    d[ctx.fragments.platform] = 3
    d[ctx.fragments.apple] = 4
    d[ctx.fragments.proto] = 5
    lines = [
        "cpp key count: %d" % len([k for k in d.keys() if type(k) == "cpp"]),
        "fragment keys: %d" % len(d),
        "cpp value: %d" % d[cpp],
        "cpp == cpp: %s" % (ctx.fragments.cpp == ctx.fragments.cpp),
        "cpp == platform: %s" % (ctx.fragments.cpp == ctx.fragments.platform),
        "apple == apple: %s" % (ctx.fragments.apple == ctx.fragments.apple),
    ]

    contexts = [dep[CcInfo].compilation_context for dep in ctx.attr.deps]
    structs = depset([struct(cc = c) for c in contexts] + [struct(cc = c) for c in contexts])
    lines.append("depset of structs: %d" % len(structs.to_list()))
    by_context = {}
    for c in contexts + contexts:
        by_context[struct(cc = c)] = True
    lines.append("dict of structs: %d" % len(by_context))
    lines.append("same context equal: %s" % (contexts[0] == contexts[0]))
    lines.append("different contexts equal: %s" % (contexts[0] == contexts[1]))

    files = ctx.attr.deps[0].files
    default_info = ctx.attr.deps[0][DefaultInfo]
    targets = {dep: dep.label.name for dep in ctx.attr.deps + ctx.attr.deps}
    lines.append("dict of targets: %d" % len(targets))
    lines.append("depset of depsets: %d" % len(depset([struct(f = files), struct(f = files)]).to_list()))
    lines.append("files_to_run in dict: %d" % len({default_info.files_to_run: 1}))
    lines.append("runfiles in dict: %d" % len({default_info.default_runfiles: 1}))

    out = ctx.actions.declare_file(ctx.label.name + ".txt")
    ctx.actions.write(out, "\n".join(lines) + "\n")
    return [DefaultInfo(files = depset([out]))]

hash_probe = rule(
    implementation = _hash_impl,
    attrs = {"deps": attr.label_list(providers = [CcInfo])},
    fragments = ["cpp", "platform", "apple", "proto"],
)
