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
