"""Labels and paths with non-ASCII names (may fail; kept apart from cases.bzl)."""

load("//:bytes_util.bzl", "b")

def report():
    l = Label("//:fg_é")
    print("LCASE label_name %s" % b(l.name))
    print("LCASE label_name_len %s" % len(l.name))
    print("LCASE label_str %s" % b(str(l)))
