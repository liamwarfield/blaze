def _r_impl(ctx):
    return []

def _make(impl, inputs, outputs):
    return transition(implementation = impl, inputs = inputs, outputs = outputs)

def _rule_with(t):
    return rule(implementation = _r_impl, cfg = t, attrs = {"deps": attr.label_list()})

# A feature flag read as a transition input.
def _ff_input_impl(settings, attr):
    return {"//flags:color": str(settings["//trans/ff:ff"])}

ff_input_rule = _rule_with(_make(_ff_input_impl, ["//trans/ff:ff"], ["//flags:color"]))

# A feature flag set as a transition output.
def _ff_output_impl(settings, attr):
    return {"//trans/ff:ff": "on"}

ff_output_rule = _rule_with(_make(_ff_output_impl, [], ["//trans/ff:ff"]))

# The config_feature_flag transition factory.
def _ff_user_impl(ctx):
    return []

ff_setter_rule = rule(
    implementation = _ff_user_impl,
    cfg = config_common.config_feature_flag_transition("flag_values"),
    attrs = {
        "flag_values": attr.label_keyed_string_dict(),
        "deps": attr.label_list(),
    },
)

