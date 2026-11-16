# Frozen fieldless Wire enum layout pins

The regression is deliberately frozen to the 151 affected derives existing at
`5c6c41f19f64c26231e0e49be505c5e2b1439be9`. It is not a registry that new enums
must join. New enums, including a new enum sharing an old leaf name in another
module, require no central manifest or generated-test edit. Discovery uses
qualified module identities and checks that every active frozen pin still exists.
Deleting a required guard fails against the separate frozen identity inventory.
Removing a type or its fieldless Wire derive is not a hash regression: remove its
matching entries from both fixture files in the SAME PR as the source change and
record the identity in the manifest's `retired_identities`. Retiring an enum that
still derives fieldless Wire is rejected. The retirement list is not a type
registry and is never extended for new enums. The two keyframe animation enums
intentionally became non-Wire in arismoko/horizon#2983; their obsolete pins are now retired.
There are 148 absolute hash pins plus a same-source equivalence guard for HookId.
HookId is the only fieldless enum among these 151 defined in generator output;
the other generated provider/host enums have payloads and are not in this set.

HookId grows whenever hooks are added. Its captured `baseline_hash` is historical,
not an assertion about future builds. The vocabulary build script emits a private
OUT_DIR array from that build's generated variant names. The ordinary test compares
the const-folded actual layout with a runtime loop over those names, using the
existing layout_name/layout_mix functions in the original order. A black-box slice
keeps the reference loop from becoming another huge compile-time expression.
No changing variant list is checked in, and the reference is cfg(test)-only.

`tools/fixtures/wire_enum_layouts.json` contains hashes captured from the actual
pre-fold library at that base, normal native/default-feature opt0.
`wire_enum_baseline_identities.json` freezes those canonical identities. Neither
file is extended for future enum additions. The generated ordinary Rust test
calls each actual old type without unsafe code. Two fixture paths were corrected
using rustc E0603 suggestions (EditKey and ProvidedBy); the manifest retains their
canonical identities and correction provenance. This is not an automatic alias
resolver.

## Reproduce the original capture

Use the frozen base BEFORE the fold, with the same compiler for the library and
diagnostic. The producer is committed as `tools/capture_wire_enum_layouts.py`.
The following commands are for an explicit developer capture, not CI or the carry
validation. Keep their artifacts together; do not substitute incompatible rlibs.

```sh
mkdir -p target/wire-layout-capture
# The diagnostic flag forces a recorded rustc invocation even with warm outputs.
env -u RUSTC_WRAPPER RUSTC_BOOTSTRAP=1 CARGO_INCREMENTAL=0 \
  cargo rustc --locked -p horizon-mod-vocab --lib -vv -- -Ztime-passes \
  2> target/wire-layout-capture/build.log
env -u RUSTC_WRAPPER RUSTC_BOOTSTRAP=1 cargo rustc --locked \
  -p horizon-mod-vocab --lib -- -Zunpretty=expanded \
  > target/wire-layout-capture/expanded.rs
python3 tools/capture_wire_enum_layouts.py \
  --base 5c6c41f19f64c26231e0e49be505c5e2b1439be9 \
  --build-log target/wire-layout-capture/build.log \
  --expanded target/wire-layout-capture/expanded.rs \
  --output target/wire-layout-capture/pins.json \
  --identities-output target/wire-layout-capture/identities.json
```

The producer discovers all local affected enums, requires exact raw/demangled
symbol coverage, links their actual layout functions, and saves the command and
hash output under `target/wire-layout-capture/`. It generates an unsafe Rust-ABI
probe only in that private diagnostic directory, never in runtime or golden-test
source. It invokes rustc directly, not Cargo, using the recorded runtime dependency
artifacts; proc-macro metadata is not substituted. Inspect generated data before
copying it into the frozen fixtures. Preserve the two compiler-guided alias
corrections if regenerating the original capture. Intentional changes to an OLD
enum's encoding require an independently reviewed contract change, not blind
replacement of expected values.

## Check after the fold

```sh
mkdir -p target/wire-goldens
env -u RUSTC_WRAPPER RUSTC_BOOTSTRAP=1 cargo rustc --locked \
  -p horizon-mod-vocab --lib -- -Zunpretty=expanded \
  > target/wire-goldens/expanded.rs
python3 tools/generate_wire_enum_goldens.py --expanded target/wire-goldens/expanded.rs --check
cargo test --locked -p horizon-mod-vocab --lib existing_fieldless_enum_layouts_match_baseline
```

Omit `--check` only to regenerate the ordinary Rust file from the frozen pins.
CI runs this discovery/generated-file check and the Rust test in the existing
wasm-mods fixture job. These verify the 148 pins and same-source HookId guard,
not the capture producer's compiler invocation. Expansion alone supplies no rlib;
the fixture does not add an extra library build solely for a producer check.
Producer invocation on a new source revision requires matching compiled artifacts
and must not be claimed from expansion-only or old-base evidence. Expansion alone does not emit an rlib: its wrapper must be
unset because sccache expects that artifact and otherwise fails. Bootstrap enables
diagnostics only; neither flag changes production compilation.

Coverage is native/default-feature affected derives, not inactive target/feature
branches or manual codecs. Explicit unchanged manual exclusions: external
Direction, EquipmentSlot, GameMode, UseAnimation, ConnectionKind, AttachmentWrite;
and local Reduction's deliberate compact-u8 encoding. Payload enums/structs are
outside the const-fold change. Small independent golden-byte/hash tests separately
guard against mechanically captured implementation mistakes.

## Intentional ingredient-shape contract change

Issue arismoko/horizon#3737 adds `Shape::CustomDisplay` at ordinal 3. Compound, intersection and
difference retain ordinals 0, 1 and 2; `ingredient_shape_contract_keeps_existing_tags_and_adds_custom_display_at_three`
checks their literal bytes. The Shape pin records the approved four-variant
contract, independently mixing the literal variant names with the original
FNV layout algorithm. Its original captured hash remains in `baseline_hash`.
The other frozen pins and the identity inventory remain unchanged.

## Intentional explosion-interaction contract change

Issue arismoko/horizon#3929 gives `ExplosionInteraction` vanilla's five values in vanilla's
order: `None` 0, `Block` 1, `Mob` 2, `Tnt` 3 (was 2) and `Trigger` 4, a clean
break with the host call that takes it. `explosion_interactions_cross_in_vanillas_order`
checks their literal bytes. The pin records the five-variant contract,
independently mixing the literal variant names with the original FNV layout
algorithm; the captured hash remains in `baseline_hash`.
