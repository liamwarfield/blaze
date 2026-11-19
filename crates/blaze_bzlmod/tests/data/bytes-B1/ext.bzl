_ATTRS = {
    "s": attr.string(),
    "l": attr.string_list(),
    "d": attr.string_dict(),
    "x": attr.string(),
}

def _repo_impl(rctx):
    rctx.file("BUILD", "")

repo = repository_rule(_repo_impl, attrs = _ATTRS)

_tag = tag_class(attrs = _ATTRS)

def _ext_impl(mctx):
    for mod in mctx.modules:
        for t in mod.tags.tag:
            repo(name = "ext_repo", s = t.s, l = t.l, d = t.d, x = t.x)
    return mctx.extension_metadata(
        reproducible = False,
        facts = {"é": "🐷", "k": ["é", "\377"]},
    )

ext = module_extension(_ext_impl, tag_classes = {"tag": _tag})
