BuildSettingInfo = provider(fields = ["value"])

def _flag_impl(ctx):
    return [BuildSettingInfo(value = ctx.build_setting_value)]

bool_flag = rule(implementation = _flag_impl, build_setting = config.bool(flag = True))
int_flag = rule(implementation = _flag_impl, build_setting = config.int(flag = True))
string_flag = rule(implementation = _flag_impl, build_setting = config.string(flag = True))
string_list_flag = rule(implementation = _flag_impl, build_setting = config.string_list(flag = True))
multi_flag = rule(
    implementation = _flag_impl,
    build_setting = config.string(flag = True, allow_multiple = True),
)

def _noop_impl(ctx):
    return []

def _with(transition_object):
    return rule(
        implementation = _noop_impl,
        cfg = transition_object,
        attrs = {"deps": attr.label_list()},
    )

def _t(impl, inputs, outputs):
    return transition(implementation = impl, inputs = inputs, outputs = outputs)

def _list_impl(settings, attr):
    print("copts was", settings["//:copts"])
    return {"//:copts": ["-DX", "-DY"]}

def _tuple_impl(settings, attr):
    return {"//:copts": ("-DX", "-DY")}

def _list_default_impl(settings, attr):
    return {"//:copts": ["c0"]}

def _list_empty_impl(settings, attr):
    return {"//:copts": []}

def _multi_default_impl(settings, attr):
    return {"//:multi": ["m0"]}

def _multi_two_impl(settings, attr):
    return {"//:multi": ["a", "b"]}

def _multi_scalar_impl(settings, attr):
    return {"//:multi": "a"}

def _int_impl(settings, attr):
    return {"//:level": settings["//:level"] + 1}

def _int_default_impl(settings, attr):
    return {"//:level": 1}

def _bad_int_impl(settings, attr):
    return {"//:level": "abc"}

def _bad_int_bool_impl(settings, attr):
    return {"//:level": True}

def _bad_list_elem_impl(settings, attr):
    return {"//:copts": ["a", 1]}

def _bad_list_impl(settings, attr):
    return {"//:copts": "a"}

def _bad_bool_impl(settings, attr):
    return {"//:enabled": "yes"}

def _bool_int_impl(settings, attr):
    return {"//:enabled": 0}

def _bad_string_impl(settings, attr):
    return {"//:mode": 5}

def _print_only_impl(settings, attr):
    print("mode is %s, level is %d" % (settings["//:mode"], settings["//:level"]))
    return {}

def _fail_after_print_impl(settings, attr):
    print("about to fail")
    fail("transition failed")

list_rule = _with(_t(_list_impl, ["//:copts"], ["//:copts"]))
tuple_rule = _with(_t(_tuple_impl, [], ["//:copts"]))
list_default_rule = _with(_t(_list_default_impl, [], ["//:copts"]))
list_empty_rule = _with(_t(_list_empty_impl, [], ["//:copts"]))
multi_default_rule = _with(_t(_multi_default_impl, [], ["//:multi"]))
multi_two_rule = _with(_t(_multi_two_impl, [], ["//:multi"]))
multi_scalar_rule = _with(_t(_multi_scalar_impl, [], ["//:multi"]))
int_rule = _with(_t(_int_impl, ["//:level"], ["//:level"]))
int_default_rule = _with(_t(_int_default_impl, [], ["//:level"]))
bad_int_rule = _with(_t(_bad_int_impl, [], ["//:level"]))
bad_int_bool_rule = _with(_t(_bad_int_bool_impl, [], ["//:level"]))
bad_list_elem_rule = _with(_t(_bad_list_elem_impl, [], ["//:copts"]))
bad_list_rule = _with(_t(_bad_list_impl, [], ["//:copts"]))
bad_bool_rule = _with(_t(_bad_bool_impl, [], ["//:enabled"]))
bool_int_rule = _with(_t(_bool_int_impl, [], ["//:enabled"]))
bad_string_rule = _with(_t(_bad_string_impl, [], ["//:mode"]))
print_only_rule = _with(_t(_print_only_impl, ["//:mode", "//:level"], []))
fail_after_print_rule = _with(_t(_fail_after_print_impl, [], ["//:mode"]))

def _split_bad_list_impl(settings, attr):
    return [{"//:level": 2}, "x", 3]

def _split_bad_key_impl(settings, attr):
    return [{"//:level": 2}, {5: 3}]

def _split_ok_impl(settings, attr):
    return [{"//:level": 2}, {"//:level": 3}]

def _split_rule(impl):
    return rule(
        implementation = _noop_impl,
        attrs = {"dep": attr.label(cfg = _t(impl, [], ["//:level"]))},
    )

split_bad_list_rule = _split_rule(_split_bad_list_impl)
split_bad_key_rule = _split_rule(_split_bad_key_impl)
split_ok_rule = _split_rule(_split_ok_impl)
