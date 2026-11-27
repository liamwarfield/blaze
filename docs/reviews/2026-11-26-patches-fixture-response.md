# Response: patches review, Part 1 (2026-11-26-patches-fixture.md, STOP on 0068 at 8ecc520)

Implementer: server-4 (Opus 5.5). Tree: HEAD 8ecc5205 plus the uncommitted work of several agents.
The fixes are a new patch, 0069 (PATCHES.md), and two Blaze crates; 0068 is unchanged.

## P1-1 (S1): `dict == view`

- `third_party/starlark-rust/starlark/src/values/types/dict/value.rs`: `DictGen::equals` compares
  with a value that has `java_map_entries` as Java's `AbstractMap.equals` does (same size, then
  each key's value equal to the map's value for that key), in `java_map_equals`. `equals` has no
  heap, so the map's entries are made on a temporary heap (`Heap::temp`); only lifetimes are
  cast, nothing is stored. The view's own `equals` stays identity, so `view == dict` is false,
  as in Java. `!=`, and list or tuple equality with a dict element, follow.
- Test: `patch_0069_dict_equals_java_map` (crates/starlark_conformance/tests/patches.rs).
- Oracle: case q32 (below).

## P1-2 (S1): `json.encode(view)`

- `crates/blaze_starlark/src/json.rs`: the object branch takes a dict or any value with
  `java_map_entries` (Java `x instanceof Map`, Json.java:171); errors name the type `Map`. The
  keys now sort in UTF-16 order (Java `Arrays.sort` on Strings) instead of byte order.
- `json.encode_indent` and `json.indent` go through the same encoder. `proto.encode_text` is
  unchanged: Java's `Proto` checks `instanceof Dict` (Proto.java:148), so a view is an error
  there too ("got Map, want ..."), which case q35 confirms.
- The hook was crate-private; patch 0069 adds the public `starlark::values::dict::java_map_entries`.
- Oracle: case q33.

## P1-3 (S2): attribute conversion

- `crates/blaze_packages/src/types/type_.rs`: `convert_dict` and `copy_and_lift_starlark_value`
  for dict types take a dict or a native map (Type.java:567, :585), through one helper
  (`java_map_entry_list`). A view is also a Java `StarlarkIterable` over its keys
  (`DictLikeView extends StarlarkIterable<String>`), so list conversion (`iterable_elements`)
  now gives its keys, where Blaze failed before (`tags = native.existing_rule("x")`).
- Oracle: case q34 (a list attribute set from a view, and a dict attribute whose value element
  fails to convert).

## S4

- The test value `JavaMap` now declares type `"Map"`.
- `creation_module_extra` still formats the extra value when a function is created by its
  module's own code: the string has to exist before the module is frozen, and keeping the value
  instead would need tracing and freezing it with the function. Left as it is (a label's text
  per top-level `def`/`lambda`).

## Evidence

See the server-4 report for the binary and the case-by-case results (interp cases q32-q36 in
`~/.cache/blaze/work/server-4/interp-ws`, run with `interp-probe.sh`).
