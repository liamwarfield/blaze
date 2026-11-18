def _impl(ctx):
    return [platform_common.TemplateVariableInfo({
        "MY_VAR": "my value",
        "COMPILATION_MODE": "overridden",
    })]

my_vars = rule(implementation = _impl)
