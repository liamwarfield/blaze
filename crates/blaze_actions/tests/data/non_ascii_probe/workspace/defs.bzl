"""Rules of the non-ASCII action key probe (see cases.bzl and scripts/actions-probe/run)."""

load(":cases.bzl", "ARGS_CASES", "WRITE_ARGS_CASES", "fill_template_dict")

def _probe_impl(ctx):
    kind = ctx.attr.kind
    name = ctx.label.name
    out = None
    if not kind.startswith("symlink_unresolved"):
        out = ctx.actions.declare_file(name + ".out")
    if kind == "args":
        f = ctx.actions.declare_file("dir_é/" + name + "_é.txt")
        ctx.actions.write(output = f, content = "")
        a = ctx.actions.args()
        a.add(out)
        ARGS_CASES[name](a, f)
        ctx.actions.run(executable = "/bin/true", arguments = [a], inputs = [f], outputs = [out])
    elif kind == "env_value":
        ctx.actions.run(executable = "/bin/true", arguments = [out.path], outputs = [out], env = {"K": "é"})
    elif kind == "env_key":
        ctx.actions.run(executable = "/bin/true", arguments = [out.path], outputs = [out], env = {"é": "v"})
    elif kind == "env_default_shell":
        ctx.actions.run(
            executable = "/bin/true",
            arguments = [out.path],
            outputs = [out],
            env = {"K": "é"},
            use_default_shell_env = True,
        )
    elif kind == "exec_info":
        ctx.actions.run(
            executable = "/bin/true",
            arguments = [out.path],
            outputs = [out],
            execution_requirements = {"requires-é": "é", "no-cache": "é", "dropped-é": "é"},
        )
    elif kind == "plain_args":
        ctx.actions.run(executable = "/bin/true", arguments = [out.path, "é", "🐷"], outputs = [out])
    elif kind == "plain_args_raw_byte":
        ctx.actions.run(executable = "/bin/true", arguments = [out.path, "\377"], outputs = [out])
    elif kind == "run_shell":
        ctx.actions.run_shell(command = "echo é > " + out.path, outputs = [out])
    elif kind == "run_shell_args":
        ctx.actions.run_shell(command = "echo $1 > $2", arguments = ["é", out.path], outputs = [out])
    elif kind == "write_plain":
        ctx.actions.write(output = out, content = "héllo wörld ☃\n")
    elif kind == "write_raw_byte":
        ctx.actions.write(output = out, content = "é\377\n")
    elif kind == "write_executable":
        ctx.actions.write(output = out, content = "#!/bin/sh\necho é\n", is_executable = True)
    elif kind == "write_long":
        # Above the compression threshold.
        ctx.actions.write(output = out, content = "".join(["ligne %d: é🐷\n" % i for i in range(200)]))
    elif kind == "write_args":
        a = ctx.actions.args()
        WRITE_ARGS_CASES[name](a, None)
        ctx.actions.write(output = out, content = a)
    elif kind == "expand_template":
        ctx.actions.expand_template(
            template = ctx.file.template,
            output = out,
            substitutions = {"%é%": "é", "%k%": "🐷"},
        )
    elif kind == "template_dict":
        d = ctx.actions.template_dict()
        fill_template_dict(d)
        ctx.actions.expand_template(template = ctx.file.template, output = out, computed_substitutions = d)
    elif kind == "ascii_write_long":
        ctx.actions.write(
            output = out,
            content = "".join(["line %d of a long file write action\n" % i for i in range(40)]),
        )
    elif kind == "ascii_write_long_exec":
        ctx.actions.write(
            output = out,
            content = "#!/bin/sh\n" + "".join(["echo %d\n" % i for i in range(100)]),
            is_executable = True,
        )
    elif kind == "template_executable":
        ctx.actions.expand_template(
            template = ctx.file.template,
            output = out,
            substitutions = {"%NAME%": "wörld", "%EMPTY%": ""},
            is_executable = True,
        )
    elif kind == "file_symlink":
        ctx.actions.symlink(output = out, target_file = ctx.file.template)
    elif kind == "file_symlink_executable":
        ctx.actions.symlink(output = out, target_file = ctx.file.template, is_executable = True)
    elif kind == "symlink_unresolved":
        out = ctx.actions.declare_symlink(name + ".link")
        ctx.actions.symlink(output = out, target_path = "é/target")
    elif kind == "symlink_unresolved_ascii":
        out = ctx.actions.declare_symlink("g_link")
        ctx.actions.symlink(output = out, target_path = "../some/./target")
    elif kind == "symlink_unresolved_dir":
        out = ctx.actions.declare_symlink("g_link2")
        ctx.actions.symlink(output = out, target_path = "/abs/dir", target_type = "directory")
    else:
        fail("unknown kind " + kind)
    return [DefaultInfo(files = depset([out]))]

probe = rule(
    implementation = _probe_impl,
    attrs = {
        "kind": attr.string(mandatory = True),
        "template": attr.label(allow_single_file = True),
    },
)

KINDS = [
    "env_value",
    "env_key",
    "env_default_shell",
    "exec_info",
    "plain_args",
    "plain_args_raw_byte",
    "run_shell",
    "run_shell_args",
    "write_plain",
    "write_raw_byte",
    "write_executable",
    "write_long",
    "ascii_write_long",
    "ascii_write_long_exec",
    "symlink_unresolved",
    "symlink_unresolved_ascii",
    "symlink_unresolved_dir",
]

def probes():
    for case in ARGS_CASES:
        probe(name = case, kind = "args")
    for case in WRITE_ARGS_CASES:
        probe(name = case, kind = "write_args")
    for kind in KINDS:
        probe(name = kind, kind = kind)
    probe(name = "expand_template", kind = "expand_template", template = "template.txt")
    probe(name = "template_dict", kind = "template_dict", template = "template.txt")
    probe(name = "template_executable", kind = "template_executable", template = "tpl.txt")
    probe(name = "file_symlink", kind = "file_symlink", template = "tpl.txt")
    probe(name = "file_symlink_executable", kind = "file_symlink_executable", template = "tpl.txt")
    probe(name = "exec_properties", kind = "plain_args", exec_properties = {"prop_é": "é"})
