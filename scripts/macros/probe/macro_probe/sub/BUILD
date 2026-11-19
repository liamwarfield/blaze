load(":defs.bzl", "bad_subrule_use", "with_subrules")

exports_files(["tool_file"])

with_subrules(name = "w", x = "hello")

bad_subrule_use(name = "bad", tags = ["manual"])
