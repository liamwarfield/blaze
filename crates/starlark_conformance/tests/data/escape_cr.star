p = print
p(len("a\b"), len("a\
b"), len("""a
b"""), len(r"a\
b"), len(r"""a\b"""), repr("a\
b"), repr(r"a\b"), repr("""xy"""))
