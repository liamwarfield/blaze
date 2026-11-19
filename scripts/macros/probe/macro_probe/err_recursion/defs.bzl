def _impl_a(name, visibility):
    b(name = name + "_b")

def _impl_b(name, visibility):
    a(name = name + "_a")

a = macro(implementation = _impl_a)
b = macro(implementation = _impl_b)
