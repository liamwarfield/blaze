def _impl(name, visibility):
    pass

m = macro(implementation = _impl, attrs = {"visibility": attr.label_list()})
