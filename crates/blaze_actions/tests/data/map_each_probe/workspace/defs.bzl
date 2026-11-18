"""Rule of the map_each probe (see cases.bzl and scripts/actions-probe/run)."""

load(":cases.bzl", "CASES", "MNEMONIC_CASES")

def _probe_impl(ctx):
    out = ctx.actions.declare_file(ctx.label.name + ".out")
    if ctx.label.name in MNEMONIC_CASES:
        ctx.actions.run(
            executable = "/bin/true",
            arguments = [out.path],
            outputs = [out],
            mnemonic = MNEMONIC_CASES[ctx.label.name],
        )
        return [DefaultInfo(files = depset([out]))]
    a = ctx.actions.args()
    a.add(out)
    d = ctx.actions.template_dict()
    CASES[ctx.label.name](a, d)
    ctx.actions.run(executable = "/bin/true", arguments = [a], outputs = [out])
    return [DefaultInfo(files = depset([out]))]

probe = rule(implementation = _probe_impl)

def probes():
    for case in CASES:
        probe(name = case)
    for case in MNEMONIC_CASES:
        probe(name = case)
