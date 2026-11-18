"""Args and TemplateDict are read when the action is registered (StarlarkActionFactory.buildCommandLine)."""

def _snap_impl(ctx):
    def out(suffix):
        return ctx.actions.declare_file(ctx.label.name + "." + suffix)

    # run(arguments = [a]) twice with a mutation in between.
    a = ctx.actions.args()
    a.add("x")
    o1 = out("o1")
    ctx.actions.run(outputs = [o1], arguments = [a], executable = "/bin/true", mnemonic = "First")
    a.add("y")
    o2 = out("o2")
    ctx.actions.run(outputs = [o2], arguments = ["s", a], executable = "/bin/true", mnemonic = "Second")
    a.add_all(["p", "q"])
    o3 = out("o3")
    ctx.actions.run_shell(outputs = [o3], arguments = [a], command = "true", mnemonic = "Third")

    # write(content = a), then a mutation.
    w1 = out("w1")
    ctx.actions.write(w1, a)
    a.add("z")
    w2 = out("w2")
    ctx.actions.write(w2, a)

    # use_param_file / set_param_file_format after the action is registered.
    b = ctx.actions.args()
    b.add("--flag", "v w")
    p1 = out("p1")
    ctx.actions.run(outputs = [p1], arguments = [b], executable = "/bin/true", mnemonic = "NoParamFile")
    b.use_param_file("@%s", use_always = True)
    p2 = out("p2")
    ctx.actions.run(outputs = [p2], arguments = [b], executable = "/bin/true", mnemonic = "ParamFileShell")
    b.set_param_file_format("multiline")
    p3 = out("p3")
    ctx.actions.run(outputs = [p3], arguments = [b], executable = "/bin/true", mnemonic = "ParamFileMultiline")

    # set_param_file_format after write(content = c).
    c = ctx.actions.args()
    c.add("a b")
    f1 = out("f1")
    ctx.actions.write(f1, c)
    c.set_param_file_format("multiline")
    f2 = out("f2")
    ctx.actions.write(f2, c)

    # expand_template(computed_substitutions = d), then d.add(...).
    d = ctx.actions.template_dict()
    d.add("%A%", "one")
    t1 = out("t1")
    ctx.actions.expand_template(template = ctx.file.template, output = t1, computed_substitutions = d)
    d.add("%B%", "two")
    t2 = out("t2")
    ctx.actions.expand_template(template = ctx.file.template, output = t2, computed_substitutions = d)

    # Only the written files are built; the spawn actions are compared through aquery.
    return [DefaultInfo(files = depset([w1, w2, f1, f2, t1, t2]))]

snap = rule(
    implementation = _snap_impl,
    attrs = {"template": attr.label(allow_single_file = True)},
)
