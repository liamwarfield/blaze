load("//macro_probe/defs:rules.bzl", "probe_rule")

def _impl(name, visibility):
    probe_rule(name = name)
    probe_rule(name = "violator_" + name)
    native.genrule(name = name + "_g", outs = ["bad_out_" + name], cmd = "touch $@")

m = macro(implementation = _impl)
