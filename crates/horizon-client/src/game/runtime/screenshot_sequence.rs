//! `screenshot-sequence FRAMES STEP_MS PATH`: the running shader pack starts over, draws its
//! first FRAMES frames on a set clock, and the last is written as `screenshot PATH` writes it
//! (docs/agent-control.md, Repeatable screenshots). The reply comes once the PNG is written.

use super::{Game, Screen};
use horizon_client::{
    Result,
    gfx::{ScreenshotRequest, pack_uniforms::FrameTiming},
};
use serde_json::{Value, json};
use std::{path::PathBuf, sync::mpsc::Sender};

const MAX_FRAMES: u32 = 10_000;

/// The commands a running sequence lets through: they change nothing it draws.
const READ_ONLY: [&str; 5] = ["screenshot-sequence", "hello", "help", "status", "state"];

pub(super) struct Sequence {
    frames: u32,
    step_ms: f64,
    /// The frame drawn next, from 1.
    next: u32,
    path: PathBuf,
    reply: Sender<String>,
    /// What must stay as it was at the start (`Game::sequence_scene`).
    scene: String,
    pack: Value,
}

impl Sequence {
    pub(super) fn timing(&self) -> FrameTiming {
        FrameTiming::nth(self.next, self.step_ms)
    }
}

/// `FRAMES STEP_MS PATH`; the path is the rest of the line and may hold spaces.
fn parse(args: &str) -> Result<(u32, f64, PathBuf)> {
    let usage = "screenshot-sequence FRAMES STEP_MS PATH, or screenshot-sequence cancel";
    let (frames, rest) = args.trim().split_once(char::is_whitespace).ok_or(usage)?;
    let (step_ms, path) = rest
        .trim_start()
        .split_once(char::is_whitespace)
        .ok_or(usage)?;
    let frames: u32 = frames.parse().map_err(|_| usage)?;
    let step_ms: f64 = step_ms.parse().map_err(|_| usage)?;
    if !(1..=MAX_FRAMES).contains(&frames) {
        return Err(format!("FRAMES must be 1 to {MAX_FRAMES}").into());
    }
    if !step_ms.is_finite() || step_ms <= 0.0 {
        return Err("STEP_MS must be a number of milliseconds more than 0".into());
    }
    Ok((frames, step_ms, path.trim().into()))
}

fn capture_terrain_loading(
    streaming_busy: bool,
    replaying: bool,
    view: &horizon_client::streaming::ViewArrival,
    has_column: impl Fn((i32, i32)) -> bool,
) -> bool {
    // Replay columns can only arrive as the recording advances.
    streaming_busy || (!replaying && view.arrival_incomplete(has_column))
}

pub(super) enum CaptureMode {
    Sequence,
    Export,
}

impl CaptureMode {
    fn blocked(self, screen: Screen, terrain_loading: impl FnOnce() -> bool) -> bool {
        (matches!(self, Self::Sequence) && screen != Screen::Playing) || terrain_loading()
    }
}

impl Game<'_> {
    pub(super) fn scene_loading(&self, mode: CaptureMode) -> bool {
        mode.blocked(self.screen, || self.capture_terrain_loading())
    }

    /// The drawn world still has terrain or resources to put in place.
    fn capture_terrain_loading(&self) -> bool {
        self.title.is_some()
            || self.player.is_none()
            || self.integrated_starting.is_some()
            || self.pending_entry.is_some()
            || self.pack_reload.is_some()
            || self.pending_save.is_some()
            || self
                .remote
                .as_ref()
                .is_some_and(|r| r.level_load.is_active())
            || self.world.chunks.is_empty()
            || capture_terrain_loading(
                self.streaming.busy(),
                self.replay.playing(),
                &self.view_arrival,
                |column| self.world.chunks.contains_key(&column),
            )
            || !self.pending_uploads.is_empty()
            || self.remote_backlog() != 0
            || !self.world.dirty_light.is_empty()
            || !self.world.dirty_sections.is_empty()
    }

    /// Why a sequence can't start (or go on) now, if it can't.
    fn sequence_refusal(&self) -> Option<&'static str> {
        let g = &self.settings.horizon;
        if !self.clock.still() {
            Some("screenshot-sequence needs HORIZON_STILL=1")
        } else if !self.clock.frozen() || self.clock.stepping() || self.clock.sprinting() {
            Some("screenshot-sequence needs the world frozen (/tick freeze), not stepping")
        } else if self.flight.is_some() || self.scene_loading(CaptureMode::Sequence) {
            Some("screenshot-sequence needs a loaded world in play, with nothing left to load")
        } else if self.lod.is_some() || self.far_terrain_choice().0 > 0 {
            // Far tiles load and fade in on the wall clock.
            Some("screenshot-sequence needs far terrain off (LOD Distance 0)")
        } else if self.settings.chunk_fade != 0.0 || g.shadow_cache {
            Some("screenshot-sequence needs chunk fade 0 and the shadow cache off")
        } else if g.taa || g.frame_generation != horizon_client::graphics::FrameGeneration::Off {
            // Both build on earlier frames the pack's reset doesn't reach.
            Some("screenshot-sequence needs TAA and frame generation off")
        } else {
            None
        }
    }

    /// What a sequence's frames must share: the world, the camera, the settings, the pack and
    /// the window's size. Before the first frame the roll is the last drawn frame's. For a frame
    /// about to be drawn (`sequence_frame_asks`: whether mods are asked for its roll, else
    /// level) it is that frame's, kept for it in `sequence_frame_roll`, and asked after the pose:
    /// a mod hears `camera_pose` before `camera_roll`, as on every frame.
    fn sequence_scene(&mut self) -> Result<String> {
        let frame = self.sequence_frame_asks.take();
        let view = self.camera_view(1.0).ok_or("no camera")?;
        let (position, yaw, pitch) = self.native_camera_pose_now(1.0, view);
        let roll = match frame {
            None => self.posed_roll,
            Some(asks) => {
                let roll = if asks {
                    self.native_camera_roll_now(self.clock.camera_fraction() as f32)
                } else {
                    0.0
                };
                self.sequence_frame_roll = Some(roll);
                roll
            }
        };
        let renderer = self.renderer.as_ref().ok_or("no renderer")?;
        Ok(format!(
            "{:?}",
            (
                (self.dim.id(), self.world.game_time, self.world.day_time),
                (self.world.revision, self.world.chunks.len()),
                self.world.chunks.membership_revision(),
                (
                    position.to_array().map(f64::to_bits),
                    yaw.to_bits(),
                    pitch.to_bits(),
                    roll.to_bits()
                ),
                format!("{:?}", self.camera_type),
                format!("{:?}", self.settings),
                (&self.native_shaders.picked, renderer.shader_pack_kind()),
                [renderer.config.width, renderer.config.height],
            )
        ))
    }

    /// `screenshot-sequence ...`; `None` while the reply waits for the PNG.
    pub(super) fn screenshot_sequence_command(
        &mut self,
        line: &str,
        reply: &Sender<String>,
    ) -> Result<Option<Value>> {
        let args = line
            .trim_start()
            .strip_prefix("screenshot-sequence")
            .unwrap_or("");
        if args.trim() == "cancel" {
            let running = self.screenshot_sequence.is_some();
            self.end_screenshot_sequence(Err("cancelled".into()));
            return Ok(Some(json!({ "ok": true, "cancelled": running })));
        }
        let (frames, step_ms, path) = parse(args)?;
        if self.screenshot_sequence.is_some() || self.export.is_some() {
            return Err("a capture is already running".into());
        }
        let renderer = self.renderer.as_ref().ok_or("no renderer yet")?;
        if self.pending_screenshot.is_some() || renderer.screenshot.is_some() {
            return Err("a screenshot is already pending".into());
        }
        if let Some(refusal) = self.sequence_refusal() {
            return Err(refusal.into());
        }
        let picked = self.native_shaders.picked.clone();
        let kind = renderer.shader_pack_kind();
        if !picked.is_empty() && kind.is_none() {
            return Err("the shader pack isn't running (still loading, or it failed)".into());
        }
        let scene = self.sequence_scene()?;
        let renderer = self.renderer.as_mut().ok_or("no renderer yet")?;
        renderer.reset_shader_pack_history();
        self.screenshot_sequence = Some(Sequence {
            frames,
            step_ms,
            next: 1,
            path,
            reply: reply.clone(),
            scene,
            pack: json!({
                "kind": kind.unwrap_or("none"),
                "name": (!picked.is_empty()).then_some(picked),
            }),
        });
        Ok(None)
    }

    /// While a sequence runs, the control commands that would change what it draws are
    /// refused; `quit` ends it first.
    pub(super) fn screenshot_sequence_gate(&mut self, command: Option<&str>) -> Result<()> {
        if self.screenshot_sequence.is_none() || command.is_none_or(|c| READ_ONLY.contains(&c)) {
            return Ok(());
        }
        if command == Some("quit") {
            self.end_screenshot_sequence(Err("cancelled: the game is quitting".into()));
            return Ok(());
        }
        Err(
            "a screenshot sequence is running: wait for its reply, or screenshot-sequence cancel"
                .into(),
        )
    }

    /// Ends the running sequence, if any, and sends its reply.
    pub(super) fn end_screenshot_sequence(&mut self, outcome: Result<Value>) {
        let Some(sequence) = self.screenshot_sequence.take() else {
            return;
        };
        if let Some(renderer) = &mut self.renderer
            && renderer
                .screenshot
                .as_ref()
                .is_some_and(|r| r.path == sequence.path)
        {
            renderer.screenshot = None;
        }
        let value = outcome.unwrap_or_else(
            |e| json!({ "error": format!("screenshot-sequence: {e}"), "frame": sequence.next }),
        );
        let _ = sequence.reply.send(value.to_string());
    }

    /// One frame of the running sequence: the next one, on its clock. A frame the surface
    /// refused is drawn again; the last is saved, and the reply sent.
    pub(super) fn render_sequence_frame(&mut self) -> Result<bool> {
        let sequence = self
            .screenshot_sequence
            .as_ref()
            .expect("a running sequence");
        let (last, path, scene) = (
            sequence.next == sequence.frames,
            sequence.path.clone(),
            sequence.scene.clone(),
        );
        let refusal = self.sequence_refusal();
        // Read once: the check and the frame both use this roll.
        self.sequence_frame_asks =
            Some(refusal.is_none() && !self.shows_panorama() && !self.studio.flying());
        let now = self.sequence_scene();
        // A mod's replacement of `sequence_scene` doesn't take it.
        self.sequence_frame_asks = None;
        match (refusal, now) {
            (None, Ok(now)) if now == scene => {}
            (refusal, _) => {
                self.sequence_frame_roll = None;
                let reason = refusal.unwrap_or(
                    "the camera, world, settings, shader pack or window size changed while it ran",
                );
                self.end_screenshot_sequence(Err(reason.into()));
                return self.render_frame();
            }
        }
        if last && let Some(renderer) = &mut self.renderer {
            renderer.screenshot = Some(ScreenshotRequest {
                path,
                size: None,
                scaled: false,
                background: false,
            });
            renderer.screenshot_result = None;
        }
        let drawn = self.render_frame();
        self.sequence_frame_roll = None;
        let renderer = self.renderer.as_mut().ok_or("no renderer")?;
        // Still queued when the frame wasn't drawn, or when the PNG is to be taken
        // again smaller (out of memory): not this sequence's frame.
        let queued = last && renderer.screenshot.take().is_some();
        let failed = drawn.is_err();
        let done = match drawn {
            Ok(false) => return Ok(false),
            Err(e) => Err(e.to_string()),
            Ok(true) => renderer.finish_sequence_frame().map_err(|e| e.to_string()),
        };
        if let Err(e) = done {
            self.end_screenshot_sequence(Err(e.into()));
            return Ok(!failed);
        }
        let sequence = self
            .screenshot_sequence
            .as_mut()
            .expect("a running sequence");
        if !last {
            sequence.next += 1;
            return Ok(true);
        }
        let outcome = match (queued, renderer.screenshot_result.take()) {
            (false, Some(Ok(pending))) => {
                let saved = pending.save_with_details().map_err(|e| e.to_string())?;
                let [w, h] = saved.size;
                let timing = sequence.timing();
                let mut value = json!({
                    "ok": true,
                    "path": sequence.path,
                    "frames": sequence.frames,
                    "step_ms": sequence.step_ms,
                    "frame": {
                        "counter": timing.counter,
                        "elapsed": timing.elapsed,
                        "delta": timing.delta,
                    },
                    "size": [w, h],
                    "pack": sequence.pack.clone(),
                });

                if let Some(hdr) = saved.hdr_path {
                    value["hdr_path"] = json!(hdr);
                }
                Ok(value)
            }
            (_, Some(Err(e))) => Err(e.into()),
            _ => Err("the last frame's PNG wasn't written".into()),
        };
        self.end_screenshot_sequence(outcome);
        Ok(true)
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_recorded_inventory_does_not_block_export_after_terrain_loads() {
        use super::{CaptureMode, Screen};

        assert!(CaptureMode::Export.blocked(Screen::Inventory, || true));
        assert!(!CaptureMode::Export.blocked(Screen::Inventory, || false));
        assert!(CaptureMode::Sequence.blocked(Screen::Inventory, || false));
    }

    use super::{capture_terrain_loading, parse};
    use std::path::PathBuf;

    #[test]
    fn capture_waits_for_streaming_work_and_an_arriving_live_view() {
        use horizon_client::streaming::ViewArrival;
        let mut arrival = ViewArrival::default();
        let now = std::time::Instant::now();
        let view = ((0, 0), 2);

        let missing = |_| false;
        let all = |_| true;
        assert!(arrival.arriving(view, now, missing));
        assert!(capture_terrain_loading(false, false, &arrival, missing));
        assert!(!capture_terrain_loading(false, true, &arrival, missing));

        let expired = now + std::time::Duration::from_secs(31);
        assert!(!arrival.arriving(view, expired, missing));
        assert!(capture_terrain_loading(false, false, &arrival, missing));
        assert!(capture_terrain_loading(true, false, &arrival, all));
        assert!(capture_terrain_loading(true, true, &arrival, all));
        assert!(!capture_terrain_loading(false, false, &arrival, all));
    }

    #[test]
    fn capture_checks_the_replacement_columns_for_the_same_view() {
        use horizon_client::streaming::ViewArrival;
        let mut arrival = ViewArrival::default();
        let now = std::time::Instant::now();
        let view = ((0, 0), 2);
        let mut columns: std::collections::HashSet<_> = (-2..=2)
            .flat_map(|x| (-2..=2).map(move |z| (x, z)))
            .collect();
        assert!(!arrival.arriving(view, now, |column| columns.contains(&column)));
        assert!(!capture_terrain_loading(false, false, &arrival, |column| {
            columns.contains(&column)
        }));

        columns.clear();
        columns.insert((0, 0));
        assert!(!arrival.arriving(view, now, |column| columns.contains(&column)));
        assert!(capture_terrain_loading(false, false, &arrival, |column| {
            columns.contains(&column)
        }));
    }

    #[test]
    fn the_path_is_the_rest_of_the_line_and_the_counts_are_bounded() {
        assert_eq!(
            parse(" 64  50 /tmp/a b.png ").unwrap(),
            (64, 50.0, PathBuf::from("/tmp/a b.png"))
        );
        assert_eq!(parse("10000 1 x.png").unwrap().0, 10_000);
        assert_eq!(parse("64 16.666667 x.png").unwrap().1, 16.666667);
        for bad in [
            "0 50 x.png",
            "10001 50 x.png",
            "4 0 x.png",
            "4 -1 x.png",
            "4 0.0 x.png",
            "4 NaN x.png",
            "4 inf x.png",
            "4 50",
            "x 50 a",
        ] {
            assert!(parse(bad).is_err(), "{bad}");
        }
    }
}
