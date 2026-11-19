"""A macro defined in another repository."""

def _ext_rule_impl(ctx):
    return []

ext_rule = rule(implementation = _ext_rule_impl, attrs = {"deps": attr.label_list()})

def _ext_macro_impl(name, visibility):
    print("ext macro %s: %s %s" % (name, native.package_relative_label(":x"), Label(":x")))
    ext_rule(name = name, visibility = visibility, deps = ["@probe_ext//:ext_private"])
    ext_rule(name = name + "_inner")

ext_macro = macro(implementation = _ext_macro_impl)
