# The native package

The project and its `.hmod` have the same flat layout, with no mod-id directory level inside the archive:

```text
my_mod/
  horizon-mod.toml        # the only manifest; mod identity and build pins
  icon.png               # optional launcher icon
  assets/my_mod/
  data/my_mod/
  Cargo.toml             # one crate; Cargo metadata is not mod identity
  Cargo.lock             # registry dependency pins, when needed
  src/lib.rs             # impl Hooks
  src/frame.rs           # optional injection callback module
```

A native source package requires both `Cargo.toml` and `[native]`; inconsistent declarations
are refused. A content-only package declares neither `[native]` nor `[wasm]`.
[Wasm packages](../wasm-mods.md) declare `[wasm]` and contain `mod.wasm`. The mod's id and version come only from `horizon-mod.toml`,
not Cargo's package name or version. An id is a plain `[a-z0-9_]{1,64}` name, also used as its resource namespace.
A native mod has no `rust-toolchain.toml`: the pinned Horizon commit decides the toolchain that builds it.
There is no input `native/` or `rust/` directory, `horizon-native.json` or hand-written `horizon-mixin.toml`.
The launcher writes the tool's manifest and stages the crate itself; those internal paths are not package layouts.

The manifest keeps the [content fields](../content-mods.md#the-manifest), including the exact `api = "0.2"`
contract and `capabilities = ["registries"]`. A native mod adds:

```toml
[native]
game = "26.3"
horizon = "<exact 40-character lowercase Horizon commit>"
mixins = "<exact 40-character lowercase horizon-mixins commit>"
```

Replace the placeholders with exact commits; the native template fills them in.
Unknown fields are refused. `icon.png` has no manifest key: when present it must be a valid,
nonzero square PNG, at most 512 × 512 pixels and 256 KiB encoded. It is optional for both content
and code mods; the scaffold does not create one.

Injections belong in the same manifest as top-level `[[inject]]` tables. Changing game code itself is `[[native.mixin]]` ([Mixins](../../concepts/native-route/2-mixins.md)). See [Injections](../../concepts/injections.md) for the available forms and fields.
