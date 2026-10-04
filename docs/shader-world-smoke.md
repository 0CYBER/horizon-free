# Optional shader world probe

The cinematic pack starts with shaders disabled. A world rendering successfully
with that profile does not establish that its optional shader pack works.
The separate manual **macOS shader research** workflow explicitly enables the
pinned Complementary Reimagined pack in private cinematic instances and probes
OpenGL and Vulkan independently.

The probe uses Iris's runtime API through the recorder's existing read-only
inspection. Success requires available shader status, an active shader pack,
the exact pinned pack identity, focused world gameplay without an overlay,
nonuniform framebuffer readback, saved world data and a clean exit. A disabled,
unavailable or different pack fails even if ordinary world pixels look valid.
Arbitrary runtime pack names and paths are not exported.

```sh
python3 tools/build_mod.py
python3 tools/test_client_graphics.py --native-display --profile cinematic --backend opengl --world --shaders
python3 tools/test_client_graphics.py --native-display --profile cinematic --backend vulkan --world --shaders
```

Shader instances use separate runtime folders and a 6 GiB maximum heap. Other
graphics probes keep their 3 GiB budget. The fixture reports the actual maximum
heap and observed shader status. Playable packs and their shader defaults are
unchanged. Full logs, PNGs, world files, assets and shader ZIPs stay private.

The research workflow runs both cases and retains selected failed outcomes.
It fails if either case fails and remains separate from required OpenGL CI.
This is a short initialization/readback/save check, not a shader FPS, physical
M2 GPU, exhaustive visual quality or distant-terrain completion benchmark.

## Observed results

The [selected shader observations](../benchmarks/shader-worlds-2026-10-04/live-observations.json)
retain successful and failed cases. On Linux llvmpipe, the OpenGL cinematic
world probe passes with the pinned shader pack actually active, matching identity,
nonuniform readback and clean save/exit. Its private image was visually inspected.
This is a software-renderer startup result, not a shader FPS or physical Mac claim.

The Linux Vulkan cinematic case fails before readback, with a Distant Horizons
lightmap handler in selected stack locations. A fresh private isolation instance
with the DH JAR removed reaches ordinary Vulkan world readback but Iris reports
shaders inactive. The strict activation assertion rejects it; ordinary world
pixels alone would have produced a misleading shader success.

Hosted Mac run 37198566508 times out in both OpenGL and Vulkan cinematic cases,
with no completed probe report. The previous timeout wrapper retained only
`completed: false`; initialization/readback/waiting state is unknown. This does
not establish shader support or incompatibility on physical Apple GPUs.

Later timeout diagnostics preserve selected initialization/readback stages,
actual child exit status after bounded cleanup, and the last private waiting-state
snapshot. Waiting state contains only six booleans and a tick count; arbitrary
screen names, messages and paths are excluded. An unsuccessful research run
remains unsuccessful. Playable shader/backend defaults remain unchanged.
