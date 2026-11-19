def _impl(name, visibility):
    native.existing_rules()

m = macro(implementation = _impl)
