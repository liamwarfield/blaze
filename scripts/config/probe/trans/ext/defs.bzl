def _r_impl(ctx):
    return []

def _make(impl, inputs, outputs):
    return transition(implementation = impl, inputs = inputs, outputs = outputs)

def _rule_with(t):
    return rule(implementation = _r_impl, cfg = t, attrs = {"deps": attr.label_list()})

# Rule extension: the child's transition composed with the parent's.
def _parent_t_impl(settings, attr):
    return {"//flags:color": "parent"}

def _child_t_impl(settings, attr):
    return {"//flags:level": 7}

parent_rule = rule(
    implementation = _r_impl,
    cfg = _make(_parent_t_impl, [], ["//flags:color"]),
    extendable = True,
    attrs = {"deps": attr.label_list()},
)

def _child_impl(ctx):
    return ctx.super()

child_rule = rule(
    implementation = _child_impl,
    parent = parent_rule,
    cfg = _make(_child_t_impl, [], ["//flags:level"]),
)

