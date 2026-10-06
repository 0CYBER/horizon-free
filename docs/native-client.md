# Horizon native development client

`horizon-client` opens a native Rust window and renders a controlled block scene
stored in Horizon's resident chunks. It loads six opaque cube textures and the
block registry from the pinned Java Edition 26.3 resources. A padded chunk mesh
culls hidden faces across chunk boundaries and merges adjacent matching faces;
the GPU draws the rectangles as instances. The camera is a free inspection tool.

This checkpoint has no player simulation, vanilla world generation, block
interaction, inventory, sound, networking or world save/load. It does not meet
the [playable-build acceptance criteria](rust-parity.md#completion-criteria-and-playable-builds).
Full block models, weighted variants, texture orientation, transparency, tinting,
light, resource packs, chunk streaming and visual parity remain unfinished.

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
JAR plus **2,337 synthetic cases**: element bounds, optional geometry, nullable
children, face UVs and quarter turns, single-axis/Euler rotations, texture
references and translucency materials, light emission, display transforms and
inheritance markers. Floating-point fields compare by their exact bits. The
comparison also covers primitive coercion, integer truncation/wraparound and
selected strict-reader numeric/syntax limits.

```sh
python3 tools/test_cuboid_model_parity.py
```

The [archived decoder result](../benchmarks/rust-cuboid-model-decoding-2026-10-06/scoped-parity.json)
records source, fixture and reference hashes. The oracle uses actual client
classes; those rendering APIs are absent from the server. This decoder is not
yet connected to the development renderer. Rotation matrices, quad baking,
blockstate selection and rendered appearance need their own checks.

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
nine-chunk scene, requires 30 presented frames and a clean exit, and writes GPU,
platform, binary and source information to ignored
`.local/horizon-client/startup-smoke.json`. It has an external timeout as well as
the application's frame deadline. These checks do not establish gameplay or
pixel parity with Java.

Initial local validation uses Linux x86_64, a headless Wayland compositor and
Vulkan llvmpipe software rendering. Textured geometry and resizing were visually
inspected; movement, arrow rotation and Ctrl+Q were exercised using virtual
physical key codes. Captured relative mouse movement and Escape release were
exercised through a virtual pointer. Native macOS/Metal and
Windows/Direct3D 12 runtime tests remain pending. Software-renderer frame rates
are not evidence of a hardware speedup.
