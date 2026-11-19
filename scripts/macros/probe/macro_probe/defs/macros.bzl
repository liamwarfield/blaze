"""Symbolic macros for the probe (valid ones)."""

load(":rules.bzl", "legacy_macro", "probe_rule")

def _simple_impl(name, visibility, deps, value, flag, count, opt_label, tags_like, _hidden):
    print("simple macro %s: deps=%s value=%s flag=%s count=%s opt_label=%s tags_like=%s visibility=%s" % (
        name,
        deps,
        value,
        flag,
        count,
        opt_label,
        tags_like,
        visibility,
    ))
    probe_rule(name = name, deps = deps, value = value, visibility = visibility)
    probe_rule(name = name + "_helper", value = "helper")
    probe_rule(name = name + "-dash", value = "dash")
    native.genrule(
        name = name + "_gen",
        outs = [name + "_gen.out"],
        cmd = "echo " + name + " > $@",
    )

simple = macro(
    implementation = _simple_impl,
    attrs = {
        "deps": attr.label_list(),
        "value": attr.string(configurable = False, default = "dflt"),
        "flag": attr.bool(),
        "count": attr.int(default = 7),
        "opt_label": attr.label(),
        "tags_like": attr.string_list(),
        "_hidden": attr.label(default = "//macro_probe/defs:private_target"),
    },
    doc = "A simple macro.",
)

def _configurable_impl(name, visibility, deps, value):
    print("configurable macro %s: deps=%s value=%s" % (name, deps, value))
    probe_rule(name = name, deps = deps, value = value, visibility = visibility)

configurable = macro(
    implementation = _configurable_impl,
    attrs = {
        "deps": attr.label_list(),
        "value": attr.string(),
    },
)

def _inner_impl(name, visibility, value):
    probe_rule(name = name, value = value, visibility = visibility)
    probe_rule(name = name + "_inner_priv", value = value)

inner = macro(
    implementation = _inner_impl,
    attrs = {"value": attr.string()},
)

def _outer_impl(name, visibility, value):
    inner(name = name + "_a", value = value + "-a")
    inner(name = name + "_b", value = value + "-b", visibility = ["//visibility:public"])
    inner(name = name, value = value + "-same-name")
    legacy_macro(name = name + "_via_legacy", value = value)
    probe_rule(name = name + "_uses_inner", deps = [":" + name + "_a"])

outer = macro(
    implementation = _outer_impl,
    attrs = {"value": attr.string(default = "outer")},
)

def _inherits_impl(name, visibility, **kwargs):
    print("inherits macro %s kwargs=%s" % (name, sorted(kwargs.keys())))
    probe_rule(name = name, visibility = visibility, **kwargs)

inherits = macro(
    implementation = _inherits_impl,
    inherit_attrs = probe_rule,
)

def _inherits_common_impl(name, visibility, **kwargs):
    print("inherits_common macro %s kwargs=%s" % (name, sorted([k for k in kwargs.keys()])))
    native.filegroup(name = name, srcs = [], visibility = visibility, **{k: v for k, v in kwargs.items() if v != None})

inherits_common = macro(
    implementation = _inherits_common_impl,
    inherit_attrs = "common",
)

def _inherits_native_impl(name, visibility, **kwargs):
    print("inherits_native macro %s kwargs=%s" % (name, sorted(kwargs.keys())))
    native.filegroup(name = name, visibility = visibility, **kwargs)

inherits_native = macro(
    implementation = _inherits_native_impl,
    inherit_attrs = native.filegroup,
    attrs = {"data": None},
)

def _finalizer_impl(name, visibility):
    rules = native.existing_rules()
    names = sorted(rules.keys())
    print("finalizer %s sees %s" % (name, names))
    probe_rule(name = name, value = ",".join(names), visibility = visibility)

finalizer = macro(
    implementation = _finalizer_impl,
    finalizer = True,
)

def _pkg_label_impl(name, visibility):
    print("package_relative_label in %s: %s %s %s" % (
        name,
        native.package_relative_label(":foo"),
        native.package_relative_label("//other:bar"),
        native.package_name(),
    ))
    probe_rule(name = name, visibility = visibility, deps = [native.package_relative_label(":" + name + "_dep")])
    probe_rule(name = name + "_dep")

pkg_label = macro(implementation = _pkg_label_impl)

def _uses_private_impl(name, visibility):
    # Targets declared by a macro may see targets in the macro's definition package.
    probe_rule(name = name, visibility = visibility, deps = ["//macro_probe/defs:private_target"])

uses_private = macro(implementation = _uses_private_impl)

def _exports_impl(name, visibility):
    native.exports_files([name + "_file.txt"], visibility = ["//visibility:public"])
    native.filegroup(name = name, srcs = [name + "_file.txt"], visibility = visibility)

exports = macro(implementation = _exports_impl)

def _test_suite_impl(name, visibility):
    native.test_suite(name = name, tests = [], visibility = visibility)

suite = macro(implementation = _test_suite_impl)

def legacy_calling_symbolic(name):
    simple(name = name, value = "from_legacy")
    probe_rule(name = name + "_legacy_rule")

def _inherits_common_raw_impl(name, visibility, **kwargs):
    native.filegroup(name = name, srcs = [], visibility = visibility, **kwargs)

inherits_common_raw = macro(
    implementation = _inherits_common_raw_impl,
    inherit_attrs = "common",
)
