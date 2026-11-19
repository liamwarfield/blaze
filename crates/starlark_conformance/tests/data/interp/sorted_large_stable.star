def lcg(n, m):
    out = []
    x = 12345
    for _ in range(n):
        x = (x * 1103515245 + 12345) % 2147483648
        out.append(x % m)
    return out

def check():
    for n in [31, 32, 33, 64, 100, 257, 1000, 3000]:
        for m in [3, 50, 100000]:
            l = lcg(n, m)
            s = sorted(l)
            print(n, m, s[:5], s[-5:], len(s))
            pairs = [(v, i) for i, v in enumerate(l)]
            k = sorted(pairs, key = lambda p: p[0] % 7)
            print([p[1] for p in k[:12]])
            r = sorted(pairs, key = lambda p: p[0] % 5, reverse = True)
            print([p[1] for p in r[:12]])

check()
