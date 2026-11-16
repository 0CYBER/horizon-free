# Emitted point record Wire proof

This standalone fixture calls `point_shape::shape` and `Shape::record_item` from
its build script. The generated record has a `Cow<str>` field, a scalar, a nested
API field and an `Option<bool>` return outcome. It compiles against the real
`mod_api` with `wasm-guest` active, including its actual derive macro and codecs.
No record declaration or Wire implementation is copied into the fixture.

From the repository root:

```sh
CARGO_INCREMENTAL=0 CARGO_TARGET_DIR="$PWD/target" cargo test --manifest-path crates/horizon-mod-point-shape/tests/record-wire/Cargo.toml --features wasm-guest
CARGO_INCREMENTAL=0 CARGO_TARGET_DIR="$PWD/target" cargo check --manifest-path crates/horizon-mod-point-shape/tests/record-wire/Cargo.toml --features wasm-guest --target wasm32-unknown-unknown
```

The three tests cover independent expected bytes and round-trip behavior,
listener replacement of text, every truncated prefix, invalid nested bool and
outcome tags, trailing bytes, and a changed grandchild encoding propagating to
the emitted record's runtime layout while its leaf seed stays identical.

A separate workspace avoids a dependency cycle: the API generator depends on
point-shape, while this proof depends on the API and builds through point-shape.
