p = print
p(repr("\a\b\f\n\r\t\v\\\'\""), repr('\"'), len("\0\7\77\377"), repr("\1"), len("""\
"""))
