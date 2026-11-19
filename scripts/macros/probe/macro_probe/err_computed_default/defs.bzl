def _dflt(name):
    return "x"

def _impl(name, visibility, s):
    pass

m = macro(implementation = _impl, attrs = {"s": attr.label(default = _dflt)})
