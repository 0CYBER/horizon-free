# Export a video

A client mod running in a world or replay on an `--offscreen` client can
capture a PNG sequence or send frames to the player's configured encoder.
The engine advances the simulation or replay on a fixed clock and writes
the output under `exports/<name>/` in the game directory.

Request a capture from a mutable client hook. Set the dimensions and rational
frame rate independently of the window size. This example requests 300 portrait
frames at 30 FPS and hides the HUD.

```rust,ignore
let request = ExportRequest::new("portrait-take", [1080, 1920], (30, 1), 300);
let id = host.start_export(&request)?;
```

Use a new directory name for every take. Use even dimensions between 16 and
the renderer's texture limit (at most 8192), a frame rate from 1 through 240 FPS,
and a frame limit up to 216,000. A zero limit runs until stopped or the replay
ends. Set `request.hud = true` to keep the player's HUD setting. For video instead
of PNGs, set `request.format = ExportFormat::Encoder` and configure an encoder
with the seekable `fd:` output protocol in the player's `horizon-export.txt`.

The call reserves the directory and returns the request ID. The renderer starts
the capture between callbacks if the same world is ready. Read the zero-based
capture index in `camera_pose` or another frame hook to drive a camera path.

```rust
fn capture_seconds(host: &dyn mod_api::ClientHost) -> Option<f64> {
    let status = host.export_status()?;
    (status.state == mod_api::ExportState::Running).then(|| {
        f64::from(status.frame) * f64::from(status.fps_den) / f64::from(status.fps_num)
    })
}
```

At 30 FPS, frame 15 is half a second into the path regardless of rendering speed.
Between frames the field counts submissions, and in the terminal status it
counts completed output frames.

Call `host.stop_export()` from a mutable client hook to finish the current
frame and drain the output. A stop requested before the first frame still
captures that frame. The completion hook `export_ended` reports the request ID
and the number of written frames after `export.json` has been published or
publication has failed. It reports `Finished` for a completed or gracefully
stopped capture and `Failed` for setup or output failure, including an export
interrupted by leaving a world or losing the server connection. If the
manifest rename exceeds its deadline, the client resumes and reports `Failed`
even if the rename may still complete. If a frame rename is already in progress
when a capture fails, the final counts include the frames confirmed at teardown.
An in-flight rename can still add a PNG later. The terminal status stays available
until another mod capture is accepted. Start the next take from a later client hook.

Mod exports share a 64 GiB budget per game session. Each accepted directory
costs 1 MiB against it. Frames cost their uncompressed RGB size, or their
actual PNG size if larger. Encoder frames count the RGB bytes the engine
sends to the player's encoder. A PNG that fails before publication releases
its reservation, and an interrupted encoder write is charged only for the
bytes accepted by the pipe. Oversized requests are refused up front, and
exports that run out of budget stop with `Failed`. The budget excludes
control-socket captures. A control-socket capture blocks another start, while
the mod status, stop and completion hook cover mod-started captures only.
