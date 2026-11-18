# The undefined name is reported before the missing file is loaded.
load("//nosuch:file.bzl", "z")

x = nope_one
