load("//macro_probe/defs:macros.bzl", "finalizer", "outer", "simple")
load("//macro_probe/defs:rules.bzl", "probe_rule")

probe_rule(name = "top")

simple(name = "ls", deps = [":top"])

outer(name = "lo")

finalizer(name = "lfin")
