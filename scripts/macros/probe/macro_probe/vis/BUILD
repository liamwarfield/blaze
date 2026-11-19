load("//macro_probe/defs:rules.bzl", "probe_rule")

# Depends on targets declared by macros in another package.
probe_rule(
    name = "consumer",
    deps = [
        "//macro_probe/ok:s1",
        "//macro_probe/ok:c2",
        "//macro_probe/ok:o1_b",
        "//macro_probe/ok:e1",
    ],
)

# o1_a is declared by a macro with default (private) visibility.
probe_rule(
    name = "bad_consumer",
    deps = ["//macro_probe/ok:o1_a"],
    tags = ["manual"],
)

# s1_helper is declared without visibility inside a macro: visible to the macro's package only.
probe_rule(
    name = "bad_consumer2",
    deps = ["//macro_probe/ok:s1_helper"],
    tags = ["manual"],
)
