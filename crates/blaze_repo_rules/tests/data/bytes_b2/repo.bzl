"""Byte-string probe for the repository_ctx / module_ctx boundaries (Blaze byte-strings B2).

The oracle (Bazel 9.3.2) ran this workspace; its RCASE lines, fetched files and marker file are
the expected values of crates/blaze_repo_rules/tests/bytes_b2_test.rs. Run with BYTES_B2_E=é in
the client environment and --repo_env=PATH=<workspace>/tôols:<workspace>/tools:/usr/bin:/bin, where both directories
hold an executable `tôol`.
"""

load("//:bytes_util.bzl", "b")

E = "é"
PIG = "🐷"

def _p(name, v):
    print("RCASE %s %s" % (name, v))

def _names(path):
    return sorted([b(p.basename) for p in path.readdir()])

def _impl(rctx):
    # read() decodes ISO-8859-1; file() writes the internal string's bytes.
    data = rctx.read(rctx.attr.src)
    _p("read_len", len(data))
    _p("read_bytes", b(data))
    rctx.file("full.txt", data, executable = False)
    rctx.file("sliced.txt", data[:3], executable = False)
    rctx.file(E + ".txt", "e" + E + "\377\n", executable = False)
    _p("attr_text", b(rctx.attr.text))

    # execute(): argv, environment, working directory, stdout/stderr.
    rctx.file("d" + E + "/" + E + ".txt", "in d" + E + "\n", executable = False)
    r = rctx.execute(
        [
            "sh",
            "-c",
            "printf '%s|%s|' \"$1\" \"$BYTES_EXEC\"; printf '\\377|'; basename \"$(pwd)\"; printf 'err%s' \"$1\" >&2",
            "sh",
            "a" + E,
        ],
        environment = {"BYTES_EXEC": E + PIG},
        working_directory = "d" + E,
    )
    _p("exec_rc", r.return_code)
    _p("exec_stdout", b(r.stdout))
    _p("exec_stderr", b(r.stderr))
    rctx.file("exec_stdout.txt", r.stdout, executable = False)
    rctx.execute(["printf", "QUIET_FALSE q" + E + "\377\n"], quiet = False)

    # Client environment and PATH lookup.
    _p("getenv", b(rctx.getenv("BYTES_B2_E", "unset")))
    _p("environ", b(rctx.os.environ.get("BYTES_B2_E", "unset")))
    path_env = rctx.os.environ.get("PATH", "")
    _p("path_env", [b(d.split("/")[-1]) for d in path_env.split(":")[:2]])
    w = rctx.which("tôol")
    _p("which", [b(w.dirname.basename), b(w.basename)] if w else "None")

    # Paths: readdir, basename, symlink, realpath.
    _p("readdir", _names(rctx.path("d" + E)))
    _p("basename", b(rctx.path(E + ".txt").basename))
    rctx.symlink(E + ".txt", "link_" + E)
    _p("symlink_read", b(rctx.read("link_" + E)))
    _p("realpath_basename", b(rctx.path("link_" + E).realpath.basename))

    # template(): read and written as ISO-8859-1, substitutions are internal strings.
    rctx.template("tpl_out.txt", rctx.attr.tpl, {"%NAME%": "w" + E + "rk", "\251": "<A9>"}, executable = False)
    _p("template", b(rctx.read("tpl_out.txt")))

    # patch(): the patch and the patched files are read and written as UTF-8.
    rctx.file("p.txt", "h" + E + "llo\nsecond\n", executable = False)
    rctx.patch(rctx.attr.patch_content, strip = 1)
    _p("patched", b(rctx.read("p.txt")))
    rctx.patch(rctx.attr.patch_newfile, strip = 1)

    # download() of a file: URL holding raw non-ASCII chars.
    rctx.file("dl-" + E + ".txt", "dl" + E + "\n", executable = False)
    rctx.download("file://" + str(rctx.path("dl-" + E + ".txt")), "dl_out.txt")
    _p("download", b(rctx.read("dl_out.txt")))

    # download_and_extract() / extract(): entry names, strip_prefix, rename_files.
    pre = "t" + "\364" + "p"  # "tôp" with ô as the single byte F4 in the archive names
    rename = {pre + "/" + E + ".txt": pre + "/renamed-" + E + ".txt"}
    rctx.download_and_extract(
        "file://" + str(rctx.path(rctx.attr.tar)),
        "tar_out",
        strip_prefix = pre,
        rename_files = rename,
    )
    _p("tar_names", _names(rctx.path("tar_out")))
    rctx.download_and_extract(
        "file://" + str(rctx.path(rctx.attr.zip)),
        "zip_out",
        rename_files = rename,
    )
    _p("zip_names", _names(rctx.path("zip_out")))
    _p("zip_sub_names", [_names(p) for p in sorted(rctx.path("zip_out").readdir(), key = lambda p: p.basename)])
    rctx.extract(rctx.attr.ar, "ar_out")
    _p("ar_names", _names(rctx.path("ar_out")))

    _p("root_names", _names(rctx.path("")))
    rctx.file("BUILD.bazel", "exports_files(glob(['*.txt']))\n")

bytes_b2_repo = repository_rule(
    implementation = _impl,
    attrs = {
        "ar": attr.label(),
        "patch_content": attr.label(),
        "patch_newfile": attr.label(),
        "src": attr.label(),
        "tar": attr.label(),
        "text": attr.string(),
        "tpl": attr.label(),
        "zip": attr.label(),
    },
)
