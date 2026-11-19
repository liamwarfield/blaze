load("//macro_probe/defs:rules.bzl", "probe_rule")
load(":defs.bzl", "m")

m(name = "a")

probe_rule(name = "uses_violator", deps = [":violator_a"], tags = ["manual"])
