def _impl(name, visibility, needed):
    pass

m = macro(implementation = _impl, attrs = {"needed": attr.string(mandatory = True)})
