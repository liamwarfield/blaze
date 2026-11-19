def _helper(name):
    fail("macro %s failed on purpose" % name)

def _impl(name, visibility):
    _helper(name)

m = macro(implementation = _impl)

def legacy(name):
    m(name = name)
