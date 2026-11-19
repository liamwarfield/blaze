load(":rules.bzl", "probe_rule")

probe_rule(name = "private_target")

probe_rule(
    name = "public_target",
    visibility = ["//visibility:public"],
)

config_setting(
    name = "is_opt",
    values = {"compilation_mode": "opt"},
    visibility = ["//visibility:public"],
)
