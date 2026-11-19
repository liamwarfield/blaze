load("//macro_probe/defs:rules.bzl", "probe_rule")

def _impl(name, visibility):
    probe_rule(name = name)

m = macro(implementation = _impl)
