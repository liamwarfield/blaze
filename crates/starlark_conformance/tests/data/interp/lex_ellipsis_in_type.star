def f(x: tuple[int, ...], *a: Callable[..., None]) -> list[...]:
    return x
print(f(1), cast(tuple[int, ...], 2))
