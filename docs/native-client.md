# Horizon native development client

`horizon-client` opens a native Rust window and renders a controlled block scene
stored in Horizon's resident chunks, alongside twelve real resource models. It
loads the block registry, models and sprites from pinned Java Edition 26.3
resources. A padded chunk mesh
culls hidden faces across chunk boundaries and merges adjacent matching faces;
the GPU draws the rectangles as instances. A separate instanced draw renders
baked cuboid faces without expanding them into cube approximations. The camera
is a free inspection tool.

This checkpoint has no player simulation, vanilla world generation, block
interaction, inventory, sound, networking or world save/load. It does not meet
the [playable-build acceptance criteria](rust-parity.md#completion-criteria-and-playable-builds).
Blockstate dispatch, weighted variants, model transforms/UV locking, fractional
transparency, tinting, lighting, resource packs, chunk streaming and full visual
parity remain unfinished.

The resource gallery sits beside the pyramid and includes oak stairs, a slab,
an anvil, a torch, a lantern, fence parts, iron bars, a cobweb, a grindstone, a
ladder and an amethyst cluster. These are explicit model selections for geometry
inspection, not simulated block placements. Binary-alpha cutouts are rendered;
the lantern's three frames advance every eight inspection ticks at 20 ticks/s.
The animation metadata codec and frame selection now use the shared
`horizon-assets` implementation checked against the pinned client. GPU
interpolation and synchronization with the eventual game loop remain unfinished.
Sprites currently require matching square frame dimensions.

## Run from a checkout

Use current stable Rust (the locked client dependencies require Rust 1.90 or
newer), Python 3.10+ and JDK 25. A native desktop display and compatible graphics
driver are required. Resource preparation verifies the pinned game artifacts,
exports the registry using the reference server, and extracts selected client
resources into ignored `.local/horizon-client/26.3/`.

```sh
python3 tools/prepare_native_client.py --prepare
cargo +stable run --locked -p horizon-client
```

Java is used during preparation. The native executable itself runs without a
JVM. Prepared resources stay private; do not commit or publish the extracted game
files. Final distributable builds still need resource provisioning and packaging.

Use `--resources PATH` for a prepared resource directory. The executable also
looks for a sibling `resources/` directory before the checkout default. Use
`--radius N` to select 1–16 chunks around the origin; this is a finite test scene,
not a render-distance or world-streaming implementation.

| Control | Action |
| --- | --- |
| WASD | Move horizontally |
| Space / either Shift | Rise / descend |
| Either Ctrl | Boost inspection speed |
| Arrow keys | Rotate camera |
| Left click | Capture mouse for camera rotation |
| Escape | Release mouse |
| F3 | Toggle FPS/chunk/mesh diagnostics in the window title |
| Ctrl+Q or window close | Exit |

Controls bind physical keyboard positions. Losing focus clears held keys and
releases the mouse. `--trace-input` logs keyboard, mouse-button and focus events
for diagnosis. On headless Linux use `vscreen run ./target/debug/horizon-client`,
then `vscreen shot`. The `wtype` tool's default generated keymap does not preserve
physical key positions; typing the character W can report physical Escape.

## Validation

The separate `horizon-assets` crate decodes the pinned 26.3 cuboid model format.
Its native comparison covers all **2,883 block-model JSON files** in the client
JAR plus **2,457 synthetic cases**: element bounds, optional geometry, nullable
children, face UVs and quarter turns, single-axis/Euler rotations, texture
references and translucency materials, light emission, display transforms and
inheritance markers. Floating-point fields compare by their exact bits. The
comparison also covers primitive coercion, integer truncation/wraparound and
selected strict-reader numeric/syntax limits. The numeric scanner also preserves
vanilla's rejection when its wrapping integer accumulator hits zero before
another digit, including in ignored fields. Twenty newly exposed acceptance
mismatches were corrected; the complete comparison now covers 5,340 models.
The [resolution](../benchmarks/rust-resource-number-scanner-2026-10-06/model-resolution.json),
[baking](../benchmarks/rust-resource-number-scanner-2026-10-06/model-baking.json) and
[animation](../benchmarks/rust-resource-number-scanner-2026-10-06/sprite-animation.json)
comparisons also pass with the shared scanner correction.

```sh
python3 tools/test_cuboid_model_parity.py
```

The [archived decoder result](../benchmarks/rust-resource-number-scanner-2026-10-06/model-decoding.json)
records source, fixture and reference hashes. The oracle uses actual client
classes; those rendering APIs are absent from the server. The decoder, parent
resolver and baker now feed the development gallery. Blockstate selection, transformed
models, UV locking and rendered appearance still need their own checks.

`ModelLibrary` resolves parent geometry, ambient occlusion, GUI lighting and
each display context. It shares inherited elements rather than cloning them.
Texture aliases bind after child overrides are applied; successful and failed
alias walks are cached. Missing resources use the native missing-model cube;
parent cycles invalidate their descendants. The
[resolution comparison](../benchmarks/rust-model-resolution-2026-10-06/scoped-parity.json)
matches **5,648 requested roots across 439 graphs**, including the full bundled
block-model catalog, missing parents, 702 cyclic roots, 1,024-model parent chains,
512-slot alias chains, explicit empty geometry and identity/null transforms.
It calls native `ModelDiscovery`, `ResolvedModel` and `TextureSlots` APIs.

```sh
python3 tools/test_model_resolution_parity.py
cargo +stable test --locked -p horizon-assets
```

Cuboid baking now produces positions in block units, normalized unit-atlas UVs,
face winding and facing, material references, tint indices, shade overrides and
light-emission metadata. It supports single-axis and Euler element rotations
with the pinned rescaling rule. The
[baking comparison](../benchmarks/rust-model-baking-2026-10-06/scoped-parity.json)
matches **7,100 roots across 440 graphs**, using native `FaceBakery` with a real
sprite and the default JOML math options. All bundled block models bake without
errors. Synthetic coverage includes reversed/degenerate bounds, signed zero,
arbitrary rotations, rescaling, null geometry children and parent cycles.
Position, UV and matrix fields compare by exact float bits for these fixtures.

```sh
python3 tools/test_model_baking_parity.py
```

This comparison uses the identity blockstate transform. Atlas packing,
transparency classification, actual tint/light evaluation and GPU rendering
remain separate from the geometry result. Other operating systems still need
their native validation runs.

Blockstate resources now decode into ordered variant selectors and multipart
conditions. The [native blockstate comparison](../benchmarks/rust-blockstate-model-decoding-2026-10-06/scoped-parity.json)
covers all 1,288 bundled definitions and 1,351 synthetic cases (2,639 total,
468 expected rejections). It uses the same `StrictJsonParser` and
`BlockStateModelDispatcher.CODEC` as the pinned loader. The Rust representation
preserves single versus weighted variants, all three quarter-turn axes, UV-lock
flags, positive weights and total-weight limits, ordered selectors, nested
AND/OR conditions and per-term negation. Transformed baking remains separate work.

```sh
python3 tools/test_blockstate_model_parity.py
```

Property binding now selects variant roots or multipart part indices for an
ordered native state list. The [binding comparison](../benchmarks/rust-blockstate-model-binding-2026-10-06/scoped-parity.json)
covers all 1,288 bundled definitions against 35,723 registered block states plus
the four special item-frame states, followed by synthetic overlap, malformed
selector and predicate cases: 3,802 definitions and 687,466 state results match,
with 220 expected binding errors. It checks the native replacement-before-abort
behavior for overlapping variants, eager multipart validation even when variants
cover every state, integer-property aliases, nested conditions and empty part
selections. The actual renderer has not yet adopted these selections.

The Java integer-property parser is shared by `horizon-core`, client binding and
world-state decoding. Its [codec regression](../benchmarks/rust-blockstate-model-binding-2026-10-06/shared-integer-codec.json)
matches 221,261 output lines against each authenticated client/server JAR.

```sh
python3 tools/test_blockstate_binding_parity.py
```

`CompiledChoices` prepares cumulative weights once per definition and shares
those choices across bound states. Selecting an ordinary single variant consumes
no random draw; a weighted singleton still does. Multipart collection draws a
seed even for an empty subset and resets the same stream before each chosen
part. The [native choice comparison](../benchmarks/rust-blockstate-model-choice-2026-10-06/scoped-parity.json)
uses actual `WeightedVariants.collectParts` and `MultiPartModel.collectParts`
with inert observable leaves. All seven distinct bundled weight patterns,
synthetic large weights and random multipart graphs produce matching selections
and subsequent random values: 6,960 fixtures, 111,360 collections, both legacy
and xoroshiro streams. This does not yet bake or render the selected models.

```sh
python3 tools/test_model_choice_parity.py
```

The sprite animation module reproduces the pinned metadata codec's integer
conversion, strict primitive types, optional null fields and frame dimensions.
It preserves repeated frames, filters out-of-image frame indices, and treats
sequences with fewer than two valid entries as static physical frame zero.
Frame lookup uses cumulative durations and binary search, so skipped rendering
frames do not require replaying all elapsed animation ticks. The native oracle
also checks dirty flags, draw requirements and interpolation timing, including
transitions near the maximum Java integer duration. Interpolation pixel output
and resource-loader failure recovery are outside this comparison.

```sh
python3 tools/test_sprite_animation_parity.py
```

The [animation comparison](../benchmarks/rust-sprite-animation-2026-10-06/scoped-parity.json)
records 907 fixtures and 258,996 native tick states from the actual client codec
and `SpriteContents.AnimationState.tick`, including all 63 bundled animated
texture metadata sections and synthetic boundary cases. The associated
[startup report](../benchmarks/rust-sprite-animation-2026-10-06/startup-smoke.json)
checks the renderer using this shared implementation. Images, input
fixtures and native traces stay private under `.local/`.

```sh
cargo +stable test -p horizon-client
cargo +stable clippy --workspace --all-targets -- -D warnings
python3 tools/test_native_client.py
# Headless Linux alternative:
python3 tools/test_native_client.py --vscreen
```

The mesh tests compare expanded rectangles against independently enumerated
exposed voxel faces, including solid neighbors, cavities, material boundaries,
negative chunk origins and random surroundings. The startup runner launches a
nine-chunk scene, requires 30 presented frames and a clean exit, verifies 145
baked gallery faces across 18 sprite layers, and writes GPU,
platform, binary and source information to ignored
`.local/horizon-client/startup-smoke.json`. It has an external timeout as well as
the application's frame deadline. These checks do not establish gameplay or
pixel parity with Java. Source and resource fingerprints must remain unchanged
during the run. An animation test covers duration boundaries, wrapping and
repeated physical frames independently of render frequency.

Initial local validation uses Linux x86_64, a headless Wayland compositor and
Vulkan llvmpipe software rendering. Textured geometry and resizing were visually
inspected; movement, arrow rotation and Ctrl+Q were exercised using virtual
physical key codes. Captured relative mouse movement and Escape release were
exercised through a virtual pointer. Native macOS/Metal and
Windows/Direct3D 12 runtime tests remain pending. Software-renderer frame rates
are not evidence of a hardware speedup.

The model gallery was also inspected at close range on Linux/Vulkan llvmpipe.
With the camera stationary, screenshot differences were confined to the lantern
as its frames changed. The [model-client startup report](../benchmarks/rust-native-model-client-2026-10-06/startup-smoke.json)
records the bounded presentation check and the scope of this visual inspection.
