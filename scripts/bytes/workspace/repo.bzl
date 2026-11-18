"""A repository rule that reads, slices, hashes and writes non-ASCII content."""

load("//:bytes_util.bzl", "b")
load("//:sha1_shim.bzl", "sha1")

def _impl(rctx):
    data = rctx.read(rctx.attr.src)
    head = "\n".join(data.split("\n")[:4])
    print("RCASE read_len %s" % len(data))
    print("RCASE read_bytes_first8 %s" % b(data[:8]))
    print("RCASE head_sha1 %s" % sha1(head))
    r = rctx.execute(["printf", "é\\377"])
    print("RCASE exec_stdout %s" % b(r.stdout))
    rctx.file("sliced.txt", data[:3], executable = False)
    rctx.file("full.txt", data, executable = False)
    rctx.template("tpl.txt", rctx.attr.src, substitutions = {"workerd": "wörkerd"}, executable = False)
    rctx.file("BUILD.bazel", "exports_files(glob(['*.txt']))\n")

bytes_repo = repository_rule(
    implementation = _impl,
    attrs = {"src": attr.label()},
)
