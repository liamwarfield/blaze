def _impl(settings, attr):
    return {
        "//command_line_option:is exec configuration": True,
        "//command_line_option:compilation_mode": "opt",
        "//command_line_option:copt": ["-DCUSTOM_EXEC"],
    }

my_exec = transition(
    implementation = _impl,
    inputs = [],
    outputs = [
        "//command_line_option:is exec configuration",
        "//command_line_option:compilation_mode",
        "//command_line_option:copt",
    ],
)
