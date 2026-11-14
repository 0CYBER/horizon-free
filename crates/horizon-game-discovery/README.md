# Game module discovery

This engine-layer build helper owns `.game-area` discovery. Both the game build script and source traversal tools call `declarations`. The generated `game_modules.rs` keeps each area root in the same logical module scope; physical area folders do not become Rust modules.

Each area contains a `.game-area` marker. Its immediate `.rs` files (except `mod.rs`) and child directories containing `mod.rs` are module roots. Names are emitted in deterministic filename order. Duplicate physical root names are rejected. The caller supplies any root files or directories that it declares separately; other unmarked directories or loose Rust roots are errors.

Optional source headers:

- `// game-module-visibility: pub` or `pub(crate)`; otherwise private.
- `// game-module-cfg: <cfg expression>`; the expression is parsed as Rust metadata. `always` disables the implicit `cfg(test)` for roots ending in `_tests`.
- `// game-module-name: <identifier>` gives a cfg-selected implementation another logical name.

There are no game feature names, platform dependencies, or unsafe operations in this crate. Native-hook, LAN action, host, hook-fire and goal-view generation remain in their existing generators.

`folder_modules` supplies the wire-loopback test target with sorted, absolute-path module declarations from a flat folder, refusing directories and `mod.rs` by name. The wasm guest fixture follows that convention in `crates/horizon-mod-wasm/tests/fixture/build.rs`, without adding a build dependency: its source folder also contains `lib.rs`, scanner-only `points.rs` and the math checker's nested `strict_math_vectors.rs`, which are not independent modules. All other Rust files are discovered automatically; the native benchmark uses the same generator. Keep guest registrations in individual files, not directories or `mod.rs`.

The extraction was scripted before boundary changes. Verify the verbatim move with:

```sh
python3 tools/refactors/extract_game_discovery.py --verify --moved-commit f38483fc33
```
