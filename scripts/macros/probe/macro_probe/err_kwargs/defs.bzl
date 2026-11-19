load("//macro_probe/defs:rules.bzl", "probe_rule")

def _impl(name, visibility):
    pass

m = macro(implementation = _impl, inherit_attrs = probe_rule)
