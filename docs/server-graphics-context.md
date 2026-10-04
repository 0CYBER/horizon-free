# Integrated-server graphics context

Client rendering competes with the integrated server for CPU time. Version 0.21
copies GPU, backend and driver into the tick report when F8 or `/horizon bench`
starts a single-player recording. The values come from the client/render thread
and are copied into immutable metadata; server ticks never access GPU APIs.

Reports identify `server_runtime_kind` as integrated, dedicated or unknown.
`client_graphics_capture_status` is `captured_at_recording_start` only when all
three graphics fields are supplied for an integrated server. The server-only
command does not inspect client graphics. Old callers retain their compatible
four-argument recording entrypoint and report unknown runtime context.

The offline viewer and CLI flag differing runtime kinds, graphics backends and
drivers. Legacy reports remain readable, with unknown-context warnings. A
matched start snapshot does not establish unchanged graphics throughout the
recording, equivalent client quality, world work or scheduling. Rendering changes
still require paired frame reports and an explicitly controlled experiment.
This metadata change makes no FPS, TPS or RAM improvement claim.

```sh
python3 tools/build_mod.py
python3 tools/test_server_graphics.py --native-display --backend opengl
# Separate fresh private instance; Linux software renderer research only:
python3 tools/test_server_graphics.py --native-display --backend vulkan
```

The private fixture starts a normal 10-second frame/tick recording through the
client recorder, checks both reports against the actual device, saves the demo
world and exits. It does not change simulation rate or gameplay rules. Windows
compiles the fixture; Linux and hosted macOS run the OpenGL check. Full logs,
worlds and fixture JARs remain private and outside playable downloads.
