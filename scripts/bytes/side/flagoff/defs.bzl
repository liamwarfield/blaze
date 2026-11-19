"""Byte-strings review 2026-11-20 S2-1: the semantics flag at the rule and aspect evaluators."""

def _r_impl(ctx):
    print("CASE flagoff_rule " + repr("\240a\240".strip()))
    print("CASE flagoff_rule_json " + str(len(json.decode('"\\ud83d\\ude00"'))))
    return []

r = rule(implementation = _r_impl)

def _a_impl(target, ctx):
    print("CASE flagoff_aspect " + repr("\240a\240".strip()))
    return []

a = aspect(implementation = _a_impl)

print("CASE flagoff_bzl " + repr("\240a\240".strip()))
