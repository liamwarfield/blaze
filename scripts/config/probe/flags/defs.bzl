BuildSettingInfo = provider(fields = ["value"])

def _impl(ctx):
    return [BuildSettingInfo(value = ctx.build_setting_value)]

string_flag = rule(implementation = _impl, build_setting = config.string(flag = True))
bool_flag = rule(implementation = _impl, build_setting = config.bool(flag = True))
int_flag = rule(implementation = _impl, build_setting = config.int(flag = True))
string_list_flag = rule(implementation = _impl, build_setting = config.string_list(flag = True))
string_set_flag = rule(implementation = _impl, build_setting = config.string_set(flag = True))
repeatable_string_flag = rule(implementation = _impl, build_setting = config.string(flag = True, allow_multiple = True))
