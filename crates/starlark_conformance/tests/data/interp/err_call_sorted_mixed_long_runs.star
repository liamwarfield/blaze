def f():
    l = []
    for i in range(80):
        l.append((i * 37) % 101)
    l[60] = "a"
    return sorted(l)
f()
