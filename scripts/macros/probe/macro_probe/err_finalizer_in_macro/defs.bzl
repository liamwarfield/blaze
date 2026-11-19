def _fin(name, visibility):
    pass

fin = macro(implementation = _fin, finalizer = True)

def _impl(name, visibility):
    fin(name = name + "_fin")

m = macro(implementation = _impl)
