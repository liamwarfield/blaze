maxint = 2147483647
minint = -2147483648
print(list(range(0, 10, 2)[1:5:maxint]))
print(list(range(0, 10, 2)[4:1:minint]))
print(list(range(minint, maxint, maxint)[0:2:2]))
print(list(range(-1, -10, -2)[1:5:maxint]))
print(list(range(-1, -10, -2)[4:1:minint]))
print(list(range(maxint, minint, minint)[0:2:2]))
