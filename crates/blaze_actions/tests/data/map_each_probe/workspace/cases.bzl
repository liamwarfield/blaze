"""`map_each` functions that are or are not globals of their module, one function per case.

`defs.bzl` runs them on the oracle (scripts/actions-probe/run records the analysis errors in
../expected.txt); crates/blaze_actions/tests/map_each_global_test.rs evaluates this same file.
Line and column numbers of this file are part of the recorded errors.
"""

def _mk():
    def inner(x):
        return x

    return inner

_nested = _mk()

def _top(x):
    return x

_alias = _top

_lambda = lambda x: x

def _nested_add_all(a, d):
    a.add_all(["x"], map_each = _nested)

def _nested_add_joined(a, d):
    a.add_joined(["x"], join_with = ",", map_each = _nested)

def _nested_allow_closure(a, d):
    a.add_all(["x"], map_each = _nested, allow_closure = True)

def _lambda_global(a, d):
    a.add_all(["x"], map_each = _lambda)

def _lambda_local(a, d):
    a.add_all(["x"], map_each = lambda x: x)

def _lambda_allow_closure(a, d):
    a.add_joined(["x"], join_with = ",", map_each = lambda x: x, allow_closure = True)

def _top_level(a, d):
    a.add_all(["x"], map_each = _top)

def _top_level_alias(a, d):
    a.add_all(["x"], map_each = _alias)

def _builtin(a, d):
    a.add_all(["x"], map_each = str)

def _template_dict_nested(a, d):
    d.add_joined("k", depset(["x"]), join_with = ",", map_each = _nested)

def _template_dict_lambda(a, d):
    d.add_joined("k", depset(["x"]), join_with = ",", map_each = lambda x: x)

def _template_dict_allow_closure(a, d):
    d.add_joined("k", depset(["x"]), join_with = ",", map_each = _nested, allow_closure = True)

def _template_dict_top_level(a, d):
    d.add_joined("k", depset(["x"]), join_with = ",", map_each = _alias)

CASES = {
    "nested_add_all": _nested_add_all,
    "nested_add_joined": _nested_add_joined,
    "nested_allow_closure": _nested_allow_closure,
    "lambda_global": _lambda_global,
    "lambda_local": _lambda_local,
    "lambda_allow_closure": _lambda_allow_closure,
    "top_level": _top_level,
    "top_level_alias": _top_level_alias,
    "builtin": _builtin,
    "template_dict_nested": _template_dict_nested,
    "template_dict_lambda": _template_dict_lambda,
    "template_dict_allow_closure": _template_dict_allow_closure,
    "template_dict_top_level": _template_dict_top_level,
}
