# analysis_test_transition.
def _at_impl(ctx):
    return [AnalysisTestResultInfo(success = True, message = "")]

at_test = rule(
    implementation = _at_impl,
    analysis_test = True,
    attrs = {
        "dep": attr.label(cfg = analysis_test_transition(settings = {
            "//command_line_option:copt": ["-DAT"],
            "//flags:color": "at",
        })),
    },
)

