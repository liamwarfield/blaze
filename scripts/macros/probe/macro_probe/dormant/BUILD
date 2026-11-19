load(":defs.bzl", "consumer", "r")

filegroup(name = "a")

filegroup(name = "b1")

filegroup(name = "b2")

r(
    name = "r",
    dormant = ":a",
    dormant_list = [
        ":b1",
        ":b2",
    ],
)

consumer(name = "c")
