def _impl(name, visibility):
    native.glob(["*.txt"])

m = macro(implementation = _impl)
