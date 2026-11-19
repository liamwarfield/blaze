# Byte-string cases beyond single chars: escapes, json, multi-char case mapping. The Java output
# is recorded in strings_misc.latin1.txt / strings_misc.unicode.txt (java/StarlarkProbe.java).
# Non-ASCII literals are UTF-8 in this file: in Latin-1 mode each byte is one char.

def p(*xs):
    print("|".join([x if type(x) == "string" else str(x) for x in xs]))

p(len("é"), len("🐷"), len("\303\251"), "é" == "\303\251", len("\0"), len("\00"), len("\000"), len("\0000"))
p(repr("\0\1\12\123\1234\08\377"), len("\377\376"))
p("aé🐷".upper(), "AÉ".lower(), "中".capitalize(), "été".title(), "ÿ".capitalize(), "µ".title())
p("\377".capitalize(), "\265".capitalize(), "\337".capitalize(), "\377\265\337".title(), "a\377".title())
p("\377".upper(), "\265".upper(), "\337".upper(), "\300".lower(), "\330ß".lower())
p(" é ".strip(), "Å".strip(), "\240a\240".strip(), "\205a\205".lstrip(), "\205a\205".rstrip(), "\034a\037".strip(), "\240a".strip("\240"))
p("a\302\205b\nc\rd\r\ne".splitlines(), "a\302\205b".splitlines(True))
p(json.decode('"\\u00e9"'), json.decode('"\\ud83d\\udc37"'), json.decode('"\\ud83d"'), json.decode('"\\udc37\\ud83d"'), json.decode('"é"'), json.decode('"\\u0041\\u00e9x"'))
p(len(json.decode('"\\u00e9"')), len(json.decode('"\\ud83d\\udc37"')), json.decode('["\\u00e9", {"\\u00e9": "\\u0123"}]'))
p(json.encode("é"), json.encode("\377"), json.encode("\001\177\200"), json.encode({"é": "🐷"}), json.encode_indent(["é"], prefix = "\377", indent = "\240"))
p(json.indent('{"é":[1]}'), json.decode('"\\u00e9', "dflt"))
p(str(["é", "\001"]), "%s|%r" % ("é", "é"), "{}|{}".format("é", len("é")), repr({"\377": "\t"}))
p(hash("é"), hash("🐷"), hash("\377\377"), sorted(["z", "é", "\377", "a", "🐷"]), "é" < "z")
p("é🐷".elems(), "aé"[1], "🐷"[1:3], "aé🐷"[::2], "éé".count("\251"), "a€b".split("\202"))
p("é".partition("\251"), "é".replace("\251", "x"), "é".removeprefix("\303"), "aé🐷b".find("b"), "aé🐷b".index("🐷"), "éaé".rfind("é"))
p("é" * 3, len("é" * 3), "%s" % "é", "%d%%" % len("🐷"))
p(int("12"), "١".isdigit(), "1é".isalnum(), "\t\n\013\014\r \205\240".isspace(), "\034".isspace(), "".isspace())
p("ÉCOLE".isupper(), "école".islower(), "Été Été".istitle(), "Ab Cd".istitle(), "aB".istitle(), "É".istitle())
p(r"\é", len(r"\é"), r"a\
b", "a\
b", """a
b""", len("""é"""))
p(str(1.5), "x".join("é".elems()), ",".join(["\303", "\251"]) == "\303,\251")
