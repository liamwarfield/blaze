def _impl(name, visibility):
    pass

def make():
    return macro(implementation = _impl)

holder = struct(m = make())
