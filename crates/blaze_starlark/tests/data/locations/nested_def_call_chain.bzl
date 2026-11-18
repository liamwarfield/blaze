def a():
    return {}["missing"]

def b():
    return a()

def c():
    return [b()]

c()
