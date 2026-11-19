"""Byte-strings review 2026-11-20 S1-3: repr/str of native providers holding non-ASCII strings.

Each line is `CASE <name> <raw repr>`: the value is printed as it is (not as a byte list), so
that the comparison sees the bytes Bazel's Printer writes.
"""

V = "à🐷\377"

def _prov_impl(ctx):
    print("CASE prov_tvi " + repr(platform_common.TemplateVariableInfo({"K": V})))
    print("CASE prov_ei " + repr(testing.ExecutionInfo({"K": V}, exec_group = "g" + V)))
    print("CASE prov_ei_default " + repr(testing.ExecutionInfo({"K": V})))
    print("CASE prov_rei " + repr(RunEnvironmentInfo({"K": V}, [V])))
    print("CASE prov_rei_str " + str(RunEnvironmentInfo({"K": V}, [V])))
    print("CASE prov_ffi " + repr(ctx.attr.flag[config_common.FeatureFlagInfo]))
    return []

prov_rule = rule(
    implementation = _prov_impl,
    attrs = {"flag": attr.label()},
)
