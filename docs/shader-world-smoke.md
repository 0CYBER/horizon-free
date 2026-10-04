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
Recorded outcomes are added after real probes finish.
