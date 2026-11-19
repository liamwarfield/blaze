load("//macro_probe/defs:macros.bzl", "configurable", "exports", "finalizer", "inherits", "inherits_common", "inherits_native", "legacy_calling_symbolic", "outer", "pkg_label", "simple", "suite", "uses_private")
load("//macro_probe/defs:rules.bzl", "probe_rule")
load("@probe_ext//:ext.bzl", "ext_macro")

package(default_visibility = ["//macro_probe:__subpackages__"])

probe_rule(name = "plain")

simple(
    name = "s1",
    deps = [":plain"],
    flag = True,
    count = 11,
    opt_label = "//macro_probe/defs:public_target",
    tags_like = ["a", "b"],
)

simple(name = "s2")

configurable(
    name = "c1",
    deps = select({
        "//macro_probe/defs:is_opt": [":plain"],
        "//conditions:default": [],
    }),
    value = select({
        "//macro_probe/defs:is_opt": "opt",
        "//conditions:default": "other",
    }),
)

configurable(
    name = "c2",
    deps = [":plain"] + select({
        "//macro_probe/defs:is_opt": [":s1"],
        "//conditions:default": [],
    }),
    value = "plain",
    visibility = ["//visibility:public"],
)

outer(name = "o1")

inherits(
    name = "i1",
    value = "inherited",
    count = 5,
    deps = [":plain"],
)

inherits_common(
    name = "ic",
    tags = ["manual"],
    testonly = True,
)

inherits_native(
    name = "in1",
    srcs = [":plain"],
)

pkg_label(name = "pl")

uses_private(name = "up")

exports(name = "ex")

suite(name = "ts")

legacy_calling_symbolic(name = "lcs")

ext_macro(name = "e1")

probe_rule(
    name = "uses_macro_targets",
    deps = [
        ":s1",
        ":o1_b",
        ":c2",
    ],
)

finalizer(name = "fin")

probe_rule(name = "after_finalizer")
