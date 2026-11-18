# starlark-rust 0.14.2 (vendored)

Upstream: https://github.com/facebook/starlark-rust, crates.io release 0.14.2 (git
0c8d107ab963362ba47ee3ab9e0173bc39bd101d), copied from the crates.io packages of `starlark`,
`starlark_syntax`, `starlark_derive` and `starlark_map`. License: Apache-2.0 (`LICENSE`); the
original copyright headers ("The Starlark in Rust Authors", "Facebook, Inc. and its affiliates")
are kept in every file.

Wired in through `[patch.crates-io]` in the root `Cargo.toml`; the directory is excluded from the
workspace, so these crates are path dependencies, not members.

Local changes are listed in `PATCHES.md`. Every changed line is marked `BLAZE PATCH NNNN`.
`patches/blaze.diff` is the full diff against the crates.io sources; regenerate it with

    R=~/.cargo/registry/src/index.crates.io-*
    for c in starlark starlark_syntax starlark_derive starlark_map; do
      diff -ru --exclude=Cargo.lock $R/$c-0.14.2 $c; done > patches/blaze.diff
