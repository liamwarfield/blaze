def f():
  l = [1]
  return [l.append(x) for x in l]
f()
