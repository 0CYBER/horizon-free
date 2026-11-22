//! Fixed-clock offscreen export sessions shared by the control socket and client mods.

use super::{EventLoopCtl, Game, KeyCode, MouseButton, Screen, Vec3, native_export};
use horizon_client::{
    Result,
    export::{
        EncoderConfig, EncoderPipe, ExportClock, ExportFolder, ExportManifest, ExportSpec, Format,
        FrameStep, ManifestPublication, ManifestState, Progress, Writer,
    },
    gfx::{ExportFrame, FRAME_EXPORT_STALL_TIMEOUT, pack_uniforms::FrameTiming},
    packet_source::PacketSource as _,
};
use mod_api::{ExportState, ExportStatus};
use serde_json::{Value, json};
use std::{
    collections::HashSet,
    sync::{Arc, mpsc::sync_channel},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

const STALL: Duration = Duration::from_secs(30);
const NS_PER_MS: u64 = 1_000_000;

struct Start {
    spec: ExportSpec,
    hold: bool,
    settle: bool,
}

fn parse_start(args: &[&str]) -> Result<Start> {
    let [name, tail @ ..] = args else {
        return Err("export start needs a name".into());
    };
    let mut tail = tail;
    let mut size = None;
    let mut fps = None;
    let mut frames = None;
    let mut seconds = None;
    let mut format = Format::PngSequence;
    let mut hud = false;
    let mut hold = false;
    let mut settle = true;
    while let [flag, rest @ ..] = tail {
        tail = rest;
        match *flag {
            "--hud" => hud = true,
            "--hold" => hold = true,
            "--no-settle" => settle = false,
            "--size" | "--fps" | "--frames" | "--seconds" | "--format" => {
                let [value, rest @ ..] = tail else {
                    return Err(format!("missing value for {flag}").into());
                };
                tail = rest;
                match *flag {
                    "--size" => {
                        let (w, h) = value.split_once(['x', 'X']).ok_or("size must be WxH")?;
                        size = Some([w.parse()?, h.parse()?]);
                    }
                    "--fps" => {
                        let (num, den) = value.split_once('/').unwrap_or((value, "1"));
                        fps = Some((num.parse()?, den.parse()?));
                    }
                    "--frames" => frames = Some(value.parse::<u32>()?),
                    "--seconds" => seconds = Some(value.parse::<f64>()?),
                    "--format" => {
                        format = match *value {
                            "png" => Format::PngSequence,
                            "encoder" => Format::Encoder,
                            _ => return Err("format must be png or encoder".into()),
                        };
                    }
                    _ => unreachable!(),
                }
            }
            _ => return Err(format!("unknown export option {flag}").into()),
        }
    }
    let fps = fps.ok_or("export start needs --fps")?;
    let frames = match (frames, seconds) {
        (Some(_), Some(_)) => return Err("give either --frames or --seconds".into()),
        (Some(n), None) if n > 0 => n,
        (Some(_), None) => return Err("--frames must be positive".into()),
        (None, Some(s)) if s.is_finite() && s > 0.0 => {
            (s * f64::from(fps.0) / f64::from(fps.1)).ceil() as u32
        }
        (None, Some(_)) => return Err("--seconds must be positive and finite".into()),
        (None, None) => 0,
    };
    let spec = ExportSpec {
        name: (*name).into(),
        size: size.ok_or("export start needs --size")?,
        fps,
        frames,
        format,
        hud,
    };
    spec.validate(8192)?;
    Ok(Start { spec, hold, settle })
}

#[derive(Default)]
struct Requests {
    hold: bool,
    remaining: u32,
}

impl Requests {
    fn held(&self) -> bool {
        self.hold && self.remaining == 0
    }

    fn add(&mut self, count: u32) -> Result<u32> {
        if !self.hold || count == 0 {
            return Err("export step needs a held export and a positive frame count".into());
        }
        self.remaining = self
            .remaining
            .checked_add(count)
            .ok_or("too many export steps queued")?;
        Ok(self.remaining)
    }

    fn captured(&mut self) {
        self.remaining = self.remaining.saturating_sub(1);
    }

    fn check_command(&self, words: &[&str]) -> Result<()> {
        if words.first() == Some(&"pause") || (!self.held() && !allowed_while_running(words)) {
            return Err("an export is running; wait for it or use export stop".into());
        }
        Ok(())
    }
}

fn allowed_while_running(words: &[&str]) -> bool {
    matches!(
        words,
        [] | [
            "export"
                | "quit"
                | "hello"
                | "help"
                | "status"
                | "state"
                | "inventory"
                | "entities"
                | "entity"
                | "looking-at"
                | "find-block"
                | "input-log"
                | "memory"
                | "worlds"
                | "servers"
                | "options-get"
                | "effects"
                | "ui"
                | "block"
                | "getblock"
                | "biome"
                | "chunk"
                | "raycast"
                | "light"
                | "blocks"
                | "screen"
                | "messages"
                | "menu"
                | "narration"
                | "mobs"
                | "simulation"
                | "chat-log"
                | "events"
                | "wait-for"
                | "record-status"
                | "record-list",
            ..
        ] | ["container", _, _, _]
            | ["studio", "replay" | "record", "status"]
            | ["profile", "status"]
            | ["profile", "client" | "server", "status"]
            | ["lan" | "friends" | "pad", "status"]
            | ["profile", "status", "--side", "client" | "server"]
            | ["book" | "emote" | "gpu-passes" | "trace"]
            | ["render-capture" | "parallel-tick" | "gpu-culling"]
            | ["section-draws" | "section-cull" | "section-sides" | "sculk"]
            | ["stats"]
            | ["stats", "raw"]
    )
}

pub(super) struct Session {
    spec: ExportSpec,
    mod_request: Option<u32>,
    stop_requested: bool,
    requests: Requests,
    settle: bool,
    stall_since: Option<Instant>,
    clock: ExportClock,
    real: Duration,
    visual_origin: i64,
    frame_elapsed: Duration,
    replay: Option<ReplayTime>,
    pending: Option<u32>,
    draining: Option<Drain>,
    submitted: u32,
    folder: Arc<ExportFolder>,
    gpu: String,
    previous_hide_gui: bool,
    pub(super) released_mappings: HashSet<std::result::Result<KeyCode, MouseButton>>,
    server_clock: bool,
    writer: Option<JoinHandle<std::io::Result<u64>>>,
    cancel_encoder: Option<Box<dyn FnOnce() + Send>>,
    progress: Arc<Progress>,
}

struct Drain {
    state: &'static str,
    interruption: Option<&'static str>,
    writer_progress: Option<(u32, Instant)>,
    manifest: Option<ManifestJob>,
}

impl Drain {
    fn new(state: &'static str) -> Self {
        Self {
            state,
            interruption: None,
            writer_progress: None,
            manifest: None,
        }
    }
}

struct ManifestJob {
    event: Value,
    started: Instant,
    publication: Arc<ManifestPublication>,
    worker: std::io::Result<JoinHandle<std::io::Result<()>>>,
}

impl ManifestJob {
    fn spawn(
        event: Value,
        publish: impl FnOnce(&ManifestPublication) -> std::io::Result<()> + Send + 'static,
    ) -> Self {
        let publication = Arc::new(ManifestPublication::default());
        let worker_publication = Arc::clone(&publication);
        Self {
            event,
            started: Instant::now(),
            publication,
            worker: thread::Builder::new()
                .name("export-manifest".into())
                .spawn(move || publish(&worker_publication)),
        }
    }

    fn worker_finished(&self) -> bool {
        match &self.worker {
            Ok(worker) => worker.is_finished(),
            Err(_) => true,
        }
    }

    fn is_finished(&self) -> bool {
        self.publication.state() == ManifestState::Published || self.worker_finished()
    }

    /// Once the rename succeeds, staging cleanup can finish independently.
    fn finish(self, pending_error: &str) -> Value {
        if !self.worker_finished() {
            return self.detach(pending_error);
        }
        let outcome = self.worker.and_then(|writer| {
            writer
                .join()
                .map_err(|_| std::io::Error::other("export manifest thread panicked"))?
        });
        let mut event = self.event;
        if self.publication.state() != ManifestState::Published {
            let error = outcome.err().unwrap_or_else(|| {
                std::io::Error::other("manifest worker exited without publishing export.json")
            });
            if event["ok"] == true {
                Self::mark_failed(&mut event, &error.to_string());
            } else {
                log::warn!("could not publish export.json: {error}");
            }
        }
        event
    }

    fn detach(self, error: &str) -> Value {
        self.detach_at_deadline(error, false)
    }

    fn detach_at_deadline(mut self, error: &str, cancel_failed: bool) -> Value {
        // An aborted control export can publish its failure manifest after the
        // control reply. A mod's deadline settles its completion hook.
        let ok = self.event["ok"] == true;
        if ok || cancel_failed {
            match self.publication.cancel() {
                ManifestState::Published => {}
                ManifestState::Publishing => {
                    let message =
                        format!("{error} (export.json rename started, publication unconfirmed)");
                    let message = if let Some(cause) = self.event["error"].as_str() {
                        format!("{cause}. {message}")
                    } else {
                        message
                    };
                    Self::mark_failed(&mut self.event, &message);
                }
                _ if ok => Self::mark_failed(&mut self.event, error),
                _ => {}
            }
        }
        self.event
    }

    fn mark_failed(event: &mut Value, error: &str) {
        event["state"] = json!("failed");
        event["ok"] = json!(false);
        event["error"] = json!(error);
    }
}

struct ReplayTime {
    generation: u64,
    /// Recording positions share the export clock's integer nanoseconds.
    cursor_ns: u64,
    pre_roll: bool,
    initial_tick_phase: bool,
    partial: f32,
}

enum Output {
    Control(Option<EncoderConfig>),
    Mod {
        request: u32,
        pipe: Option<EncoderPipe>,
        stop: bool,
        ready: bool,
    },
}

impl Output {
    fn mod_request(&self) -> Option<u32> {
        match self {
            Self::Control(_) => None,
            Self::Mod { request, .. } => Some(*request),
        }
    }

    fn ready(&self) -> bool {
        !matches!(self, Self::Mod { ready: false, .. })
    }
}

impl Session {
    pub(super) fn is_mod_export(&self) -> bool {
        self.mod_request.is_some()
    }

    pub(super) fn mod_status(&self) -> Option<ExportStatus> {
        Some(native_export::status(
            self.mod_request?,
            &self.spec,
            ExportState::Running,
            self.pending.unwrap_or(self.submitted),
            self.progress.bytes(),
        ))
    }

    fn mod_completion(&self, event: &Value) -> Option<ExportStatus> {
        let request = self.mod_request?;
        let state = if event["ok"] == true {
            ExportState::Finished
        } else {
            ExportState::Failed
        };
        // Both counters are frozen together when the manifest job starts.
        let frames = event["frames_written"].as_u64().unwrap_or_default() as u32;
        let bytes = event["bytes_written"].as_u64().unwrap_or_default();
        Some(native_export::status(
            request, &self.spec, state, frames, bytes,
        ))
    }

    fn note_world_exit(&mut self, reason: &'static str) {
        if self.is_mod_export()
            && let Some(drain) = &mut self.draining
            && drain.manifest.is_none()
        {
            drain.interruption.get_or_insert(reason);
        }
    }

    fn begin_drain(&mut self, state: &'static str, renderer: Option<&mut super::Renderer>) {
        if self.draining.is_none() {
            self.draining = Some(Drain::new(state));
            self.pending = None;
            if let Some(renderer) = renderer {
                renderer.begin_finish_frame_export();
            }
        }
    }

    fn interrupt_for_world_exit(
        &mut self,
        reason: &'static str,
        renderer: Option<&mut super::Renderer>,
    ) {
        // The control socket has always drained to "stopped" on world exit.
        // Mods hear whether the world interrupted their take, after the same drain.
        self.begin_drain("stopped", renderer);
        self.note_world_exit(reason);
    }

    pub(super) fn interrupt_for_replay_close(&mut self, renderer: Option<&mut super::Renderer>) {
        if self.replay.is_some() {
            self.interrupt_for_world_exit("export interrupted because the replay closed", renderer);
        }
    }

    pub(super) fn request_stop(&mut self) {
        self.stop_requested = true;
    }

    fn stop_between_frames(&self) -> bool {
        self.stop_requested
            && self.draining.is_none()
            && self.pending.is_none()
            && self.submitted > 0
    }

    pub(super) fn elapsed(&self) -> Duration {
        self.real
    }

    pub(super) fn frame_elapsed(&self) -> Duration {
        self.frame_elapsed
    }

    fn can_begin_frame(&self) -> bool {
        !self.requests.held() && self.draining.is_none() && self.pending.is_none()
    }

    fn begin_capture(&mut self) {
        let (num, den) = self.spec.fps;
        let next = u128::from(self.submitted) + 1;
        let ns = next * 1_000_000_000 * u128::from(den) / u128::from(num);
        let end = Duration::from_nanos(ns as u64);
        self.frame_elapsed = end - self.real;
        self.real = end;
        self.pending = Some(self.submitted);
    }

    fn capture_submitted(&mut self) {
        self.pending = None;
        self.stall_since = None;
        self.submitted += 1;
        self.requests.captured();
    }

    pub(super) fn visual_millis(&self) -> i64 {
        self.visual_origin + self.elapsed().as_millis() as i64
    }

    pub(super) fn timing(&self) -> FrameTiming {
        let (num, den) = self.spec.fps;
        FrameTiming::nth(
            self.submitted.saturating_add(1),
            1000.0 * f64::from(den) / f64::from(num),
        )
    }

    pub(super) fn replay_alpha(&self) -> Option<f64> {
        self.replay.as_ref().map(|r| f64::from(r.partial))
    }

    fn writer_finished(&self) -> bool {
        self.writer.as_ref().is_none_or(JoinHandle::is_finished)
    }

    fn cancel_output(&mut self) {
        self.progress.cancel();
        if let Some(cancel) = self.cancel_encoder.take() {
            cancel();
        }
    }

    fn begin_manifest(
        &mut self,
        mut state: &'static str,
        mut error: Option<String>,
    ) -> ManifestJob {
        // An abort can leave a sink inside a blocking write. Its worker still
        // owns cleanup, and must not hold up the game or its closing save.
        if let Some(writer) = self.writer.take().filter(JoinHandle::is_finished) {
            match writer.join() {
                Ok(Ok(_)) => {}
                Ok(Err(e)) => {
                    error.get_or_insert_with(|| e.to_string());
                }
                Err(_) => {
                    error.get_or_insert_with(|| "export output thread panicked".into());
                }
            }
        }
        let snapshot = self
            .is_mod_export()
            .then(|| self.progress.freeze_and_snapshot());
        if let Some((_, _, true)) = snapshot {
            error
                .get_or_insert_with(|| "export output interrupted".into())
                .push_str(" (frame publication in progress, output unconfirmed)");
        }
        if error.is_some() {
            state = "failed";
        }
        let mut event = self.status(state);
        let frames_written = if let Some((frames, bytes, _)) = snapshot {
            event["bytes_written"] = json!(bytes);
            frames
        } else {
            // A control export can still finish an in-flight write after an abort.
            self.progress.frames()
        };
        event["frames_written"] = json!(frames_written);
        if let Some(error) = error {
            event["ok"] = json!(false);
            event["error"] = json!(error);
        }
        let folder = Arc::clone(&self.folder);
        let spec = self.spec.clone();
        let gpu = self.gpu.clone();
        ManifestJob::spawn(event, move |publication| {
            folder.write_manifest_tracked(
                &ExportManifest {
                    spec: &spec,
                    state,
                    frames_written,
                    build: horizon_core::release::VERSION,
                    gpu: &gpu,
                },
                publication,
            )
        })
    }

    /// Called once the renderer has handed off all copies. The writer owns
    /// the remaining file or pipe writes and its final flush.
    fn poll_writer_finish(&mut self, now: Instant, budget: Duration) -> Result<()> {
        let finished = self.writer_finished();
        let drain = self.draining.as_mut().unwrap();
        if !finished {
            let frames = self.progress.frames();
            let (previous, deadline) = drain.writer_progress.get_or_insert((frames, now + budget));
            if *previous != frames {
                (*previous, *deadline) = (frames, now + budget);
            }
            if now >= *deadline {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::TimedOut,
                    "export writer stalled while finishing",
                )
                .into());
            }
        } else {
            let (state, interruption) = (drain.state, drain.interruption);
            let manifest = self.begin_manifest(state, interruption.map(str::to_owned));
            self.draining.as_mut().unwrap().manifest = Some(manifest);
        }
        Ok(())
    }

    fn poll_manifest_finish(&mut self, now: Instant, budget: Duration) -> Option<Value> {
        let manifest = self.draining.as_ref()?.manifest.as_ref()?;
        let stalled = "export manifest stalled while publishing export.json";
        if manifest.is_finished() {
            Some(self.take_manifest()?.finish(stalled))
        } else if now.saturating_duration_since(manifest.started) >= budget {
            let cancel_failed = self.is_mod_export();
            Some(
                self.take_manifest()?
                    .detach_at_deadline(stalled, cancel_failed),
            )
        } else {
            None
        }
    }

    fn take_manifest(&mut self) -> Option<ManifestJob> {
        self.draining.as_mut()?.manifest.take()
    }

    fn status(&self, state: &str) -> Value {
        json!({
            "ok": true, "active": matches!(state, "running" | "draining"), "state": state,
            "path": self.folder.path(), "frames_written": self.progress.frames(),
            "frames_submitted": self.submitted, "frames": self.spec.frames,
            "held": self.requests.held(), "queued": self.requests.remaining,
            "size": self.spec.size, "fps": self.spec.fps,
        })
    }
}

impl Game<'_> {
    pub(super) fn export_gate(&mut self, words: &[&str]) -> Result<()> {
        if words.first() == Some(&"quit") {
            self.cancel_export_on_quit();
        }
        match &self.export {
            Some(session) if session.draining.is_some() && !allowed_while_running(words) => {
                Err("export output is draining".into())
            }
            Some(session) => session.requests.check_command(words),
            None => Ok(()),
        }
    }

    pub(super) fn frame_export_command(&mut self, words: &[&str]) -> Result<Value> {
        match words {
            ["start", args @ ..] => self.start_export(parse_start(args)?),
            ["step", n] => {
                let session = self.export.as_mut().ok_or("no active export")?;
                if session.draining.is_some() {
                    return Err("export output is draining".into());
                }
                session.requests.add(n.parse()?)?;
                Ok(session.status("running"))
            }
            ["stop"] => {
                self.begin_export_drain("stopped");
                Ok(self.export_status())
            }
            ["status"] => Ok(self.export_status()),
            _ => Err(
                "usage: export start NAME --size WxH --fps N[/D] | step N | stop | status".into(),
            ),
        }
    }

    fn export_status(&self) -> Value {
        self.export.as_ref().map_or_else(idle, |s| {
            s.status(if s.draining.is_some() {
                "draining"
            } else {
                "running"
            })
        })
    }

    fn start_export(&mut self, start: Start) -> Result<Value> {
        if self.export.is_some()
            || self.native_export.pending.is_some()
            || self.native_export.ending
            || self.screenshot_sequence.is_some()
            || self.pending_screenshot.is_some()
            || self.repro.replaying()
            || self.tick_script.playing()
        {
            return Err("another capture or script is active".into());
        }
        if !self.options.offscreen
            || self.options.paced_by_split
            || self.title.is_some()
            || self.screen != Screen::Playing
            || self.player.is_none()
            || self.pending_save.is_some()
        {
            return Err("export needs a world in play with --offscreen".into());
        }
        let renderer = self.renderer.as_ref().ok_or("no renderer")?;
        if renderer.screenshot.is_some() {
            return Err("a screenshot is already pending".into());
        }
        start
            .spec
            .validate(renderer.device.limits().max_texture_dimension_2d)?;
        let game_dir = Self::game_dir();
        let encoder = match start.spec.format {
            Format::PngSequence => None,
            Format::Encoder => Some(EncoderConfig::load(&game_dir, true)?),
        };
        let clock = ExportClock::new(start.spec.fps)?;
        let folder = Arc::new(ExportFolder::create(&game_dir, &start.spec.name)?);
        self.start_export_session(start, folder, clock, Output::Control(encoder))
    }

    fn start_queued_export(&mut self, pending: native_export::PendingExport) {
        let ready = pending.scope == self.replay_scope()
            && self.native_lifecycle.epoch() == Some(pending.epoch)
            && self.check_mod_export_start().is_ok();
        let start = Start {
            spec: pending.spec,
            hold: false,
            settle: true,
        };
        let output = Output::Mod {
            request: pending.request,
            pipe: pending.encoder,
            stop: pending.stop,
            ready,
        };
        if let Err(error) = self.start_export_session(start, pending.folder, pending.clock, output)
        {
            log::warn!("could not start mod export {}: {error}", pending.request);
        }
    }

    fn start_export_session(
        &mut self,
        start: Start,
        folder: Arc<ExportFolder>,
        clock: ExportClock,
        output: Output,
    ) -> Result<Value> {
        let gpu = self
            .renderer
            .as_ref()
            .map_or_else(String::new, |r| r.export_adapter_name());
        let replay = self.replay.export_driver().map(|driver| ReplayTime {
            generation: self.replay.generation,
            cursor_ns: (driver.position_ms_f64() * NS_PER_MS as f64).round() as u64,
            pre_roll: self.replay.standalone,
            initial_tick_phase: true,
            partial: (self.replay.virtual_tick_ms / 50.0).clamp(0.0, 1.0),
        });
        let server_clock = output.ready()
            && replay.is_none()
            && self
                .integrated
                .as_ref()
                .is_some_and(|s| s.published().is_none());
        let mut session = Session {
            spec: start.spec,
            mod_request: output.mod_request(),
            stop_requested: matches!(&output, Output::Mod { stop: true, .. }),
            requests: Requests {
                hold: start.hold,
                remaining: 0,
            },
            settle: start.settle,
            stall_since: None,
            clock,
            real: Duration::ZERO,
            visual_origin: self.visual_millis(),
            frame_elapsed: Duration::ZERO,
            replay,
            pending: None,
            draining: None,
            submitted: 0,
            folder,
            gpu,
            previous_hide_gui: self.ui.hide_gui,
            released_mappings: HashSet::new(),
            server_clock,
            writer: None,
            cancel_encoder: None,
            progress: Arc::default(),
        };
        let setup = (|| -> Result<()> {
            if !output.ready() {
                return Err("client changed before the mod export could start".into());
            }
            let (out, frames) = sync_channel::<ExportFrame>(3);
            let renderer = self.renderer.as_mut().ok_or("no renderer")?;
            renderer.begin_frame_export(session.spec.size, out)?;
            let recycle = renderer
                .frame_export_recycler()
                .ok_or("no export recycle queue")?;
            let pipe = match output {
                Output::Mod { pipe, .. } => pipe,
                Output::Control(config) => config
                    .map(|config| {
                        EncoderPipe::spawn(
                            &session.folder,
                            &config,
                            session.spec.size,
                            session.spec.fps,
                        )
                    })
                    .transpose()?,
            };
            let writer = match pipe {
                Some(pipe) => {
                    session.cancel_encoder = pipe.cancel_handle();
                    Writer::encoder(pipe)?
                }
                None if session.is_mod_export() => Writer::png_with_budget(
                    Arc::clone(&session.folder),
                    2,
                    &self.native_export.budget,
                )?,
                None => Writer::png(Arc::clone(&session.folder), 2)?,
            };
            session.progress = writer.progress();
            session.writer = Some(thread::Builder::new().name("export-output".into()).spawn(
                move || {
                    let result = frames.into_iter().try_for_each(|frame| {
                        writer.write_recycled(frame.index, frame.size, frame.rgb, &recycle)
                    });
                    let finished = writer.finish();
                    result.and(finished)
                },
            )?);
            Ok(())
        })();
        let response = session.status("running");
        if !session.spec.hud {
            self.ui.hide_gui = true;
        }
        self.export = Some(session);
        if let Err(error) = setup {
            self.abort_export(Some(error.to_string()));
            return Err(error);
        }
        if server_clock && let Some(server) = &self.integrated {
            server.export_clock(true);
            if server.call(|_| ()).is_none() {
                let error = "could not acquire the integrated server's export clock";
                self.abort_export(Some(error.into()));
                return Err(error.into());
            }
        }
        self.notify_window_resize();
        Ok(response)
    }

    pub(super) fn begin_export_drain(&mut self, state: &'static str) {
        if let Some(session) = &mut self.export {
            session.begin_drain(state, self.renderer.as_mut());
        }
    }

    pub(super) fn interrupt_export_for_world_exit(&mut self, reason: &'static str) {
        if let Some(session) = &mut self.export {
            session.interrupt_for_world_exit(reason, self.renderer.as_mut());
        }
    }

    pub(super) fn cancel_export_on_quit(&mut self) {
        self.native_export.pending.take();
        if self.export.is_some() {
            self.abort_export(Some("export cancelled because the game is closing".into()));
        }
    }

    pub(super) fn abort_export(&mut self, mut error: Option<String>) {
        let Some(session) = &mut self.export else {
            return;
        };
        if session.is_mod_export()
            && session
                .draining
                .as_ref()
                .is_some_and(|d| d.manifest.is_some())
        {
            if !thread::panicking()
                && let Some(event) =
                    session.poll_manifest_finish(Instant::now(), FRAME_EXPORT_STALL_TIMEOUT)
            {
                self.finish_export(event);
            }
            return;
        }
        if let Some(manifest) = session.take_manifest() {
            let event = manifest.finish(
                error
                    .as_deref()
                    .unwrap_or("export cancelled while publishing export.json"),
            );
            self.finish_export(event);
            return;
        }
        session.cancel_output();
        if let Some(renderer) = &mut self.renderer
            && let Err(e) = renderer.abort_frame_export()
        {
            error.get_or_insert_with(|| e.to_string());
        }
        let manifest = session.begin_manifest(
            "failed",
            Some(error.unwrap_or_else(|| "export readback stopped".into())),
        );
        if session.is_mod_export() {
            let mut drain = Drain::new("failed");
            drain.manifest = Some(manifest);
            session.pending = None;
            session.draining = Some(drain);
        } else {
            self.finish_export(manifest.finish("export aborted"));
        }
    }

    fn finish_export(&mut self, mut event: Value) {
        let session = self.export.take().expect("active export");
        self.native_export.ending = session.is_mod_export();
        self.ui.hide_gui = session.previous_hide_gui;
        self.reconcile_export_releases(&session.released_mappings);
        if session.server_clock
            && let Some(server) = &self.integrated
        {
            server.export_clock(false);
        }
        self.visual_offset = session.visual_millis() - self.base_visual_millis();
        self.resume_flight_clock();
        self.replay.last_tick = Some(Instant::now());
        self.previous = Instant::now();
        if !thread::panicking() {
            if let Some(size) = self.resize_state.map(|state| state.size)
                && let Some(renderer) = &mut self.renderer
                && !renderer.device_lost()
                && (size.width, size.height) != (renderer.config.width, renderer.config.height)
            {
                renderer.resize(size.width, size.height);
            }
            self.notify_window_resize();
        }
        let status = session.mod_completion(&event);
        event["event"] = json!("export");
        self.events.note(self.ticks, event);
        if let Some(status) = status {
            self.finish_mod_export(status);
        }
    }

    pub(super) fn export_held(&self) -> bool {
        self.export
            .as_ref()
            .is_some_and(|s| s.requests.held() || s.draining.is_some())
    }

    pub(super) fn poll_export(&mut self) {
        if let Some(pending) = self.native_export.pending.take() {
            self.start_queued_export(pending);
        }
        if self
            .export
            .as_ref()
            .is_some_and(Session::stop_between_frames)
        {
            self.begin_export_drain("stopped");
        }
        let Some(session) = &mut self.export else {
            return;
        };
        if session
            .draining
            .as_ref()
            .is_some_and(|d| d.manifest.is_some())
        {
            if let Some(event) =
                session.poll_manifest_finish(Instant::now(), FRAME_EXPORT_STALL_TIMEOUT)
            {
                self.finish_export(event);
            }
            return;
        }
        if let Some(error) = session.progress.error() {
            self.abort_export(Some(error.to_string()));
        } else if session.draining.is_some() {
            let drained = self
                .renderer
                .as_mut()
                .map_or(Ok(true), |r| r.poll_finish_frame_export());
            let result = match drained {
                Ok(true) => session.poll_writer_finish(Instant::now(), FRAME_EXPORT_STALL_TIMEOUT),
                Ok(false) => Ok(()),
                Err(error) => Err(error),
            };
            if let Err(error) = result {
                self.abort_export(Some(error.to_string()));
            }
        } else if self
            .renderer
            .as_ref()
            .is_some_and(|r| r.frame_export_failed())
        {
            self.abort_export(None);
        }
    }

    fn export_ready(&mut self) -> Result<bool> {
        let Some(session) = self.export.as_ref() else {
            return Ok(false);
        };
        if !session.settle {
            return Ok(true);
        }
        // The player's section can become ready without a client tick while
        // the export waits for uploads and the loading screen's close delay.
        self.tick_client_load();
        let ready = terrain_ready(
            self.player.as_ref().map(|p| p.position),
            self.streaming.center,
            self.scene_loading(super::screenshot_sequence::CaptureMode::Export),
        );
        let session = self.export.as_mut().unwrap();
        if !ready
            && session
                .stall_since
                .get_or_insert_with(Instant::now)
                .elapsed()
                >= STALL
        {
            return Err("export stalled waiting for terrain and uploads for 30 seconds".into());
        }
        Ok(ready)
    }

    fn export_tick_ns(&self) -> u64 {
        (f64::from(self.tick_target_millis()) * NS_PER_MS as f64)
            .round()
            .max(1.0) as u64
    }

    pub(super) fn export_begin_frame(&mut self) -> Result<Option<FrameStep>> {
        if !self.export.as_ref().is_some_and(Session::can_begin_frame) {
            return Ok(None);
        }

        if let Some(replay) = self.export.as_mut().unwrap().replay.as_mut()
            && std::mem::take(&mut replay.pre_roll)
        {
            let pos = replay.cursor_ns / NS_PER_MS;
            replay.cursor_ns = pos * NS_PER_MS;
            replay.initial_tick_phase = false;
            let tick_ns = self.export_tick_ns();
            self.pre_roll_export(pos as u32)?;
            if self.export_held() {
                return Ok(None);
            }
            self.export
                .as_mut()
                .ok_or("the replay closed")?
                .clock
                .set_tick_phase(pos * NS_PER_MS % tick_ns);
        }
        let ready = self.export_ready()?;
        capture_when_ready(
            ready,
            self,
            |game| {
                let index = game.export.as_ref().ok_or("no frame export")?.submitted;
                index.checked_add(1).ok_or("export frame index overflow")?;
                game.renderer
                    .as_mut()
                    .ok_or("no renderer")?
                    .try_arm_export_frame(index)
            },
            |game| {
                let step = game.next_export_step()?;
                game.export.as_mut().unwrap().begin_capture();
                Ok(step)
            },
        )
    }

    fn next_export_step(&mut self) -> Result<FrameStep> {
        let session = self.export.as_ref().ok_or("no frame export")?;
        let (speed, remaining_ns) = match &session.replay {
            Some(replay) if replay.generation == self.replay.generation => {
                let driver = self.replay.export_driver().ok_or("the replay closed")?;
                if (driver.position_ms_f64() * NS_PER_MS as f64).round() as u64 != replay.cursor_ns
                {
                    return Err("the replay position changed outside the export clock".into());
                }
                let duration = u64::from(driver.duration_ms()) * NS_PER_MS;
                (driver.speed(), duration.saturating_sub(replay.cursor_ns))
            }
            Some(_) => return Err("the replay changed during export".into()),
            None if self.replay.playing() => return Err("a replay opened during export".into()),
            None => (1.0, u64::MAX),
        };
        let tick_ns = self.export_tick_ns();
        let session = self.export.as_mut().unwrap();
        if let Some(replay) = &mut session.replay
            && std::mem::take(&mut replay.initial_tick_phase)
        {
            session
                .clock
                .set_tick_phase((f64::from(replay.partial) * tick_ns as f64) as u64);
        }
        Ok(session
            .clock
            .step_until((speed * 1_000_000.0).round() as u32, tick_ns, remaining_ns)?)
    }

    pub(super) fn export_tick_frame(
        &mut self,
        step: &FrameStep,
        event_loop: &dyn EventLoopCtl,
    ) -> Result<()> {
        let steps = self.clock.advance_set(step.sim, step.ticks, step.partial);
        let Some(replay) = self.export.as_ref().and_then(|s| s.replay.as_ref()) else {
            if self.export.as_ref().is_some_and(|s| s.server_clock) {
                for _ in 0..steps {
                    self.integrated
                        .as_ref()
                        .ok_or("the integrated server stopped")?
                        .export_tick()?;
                    self.poll_remote()?;
                    self.client_tick_steps(1, Some(event_loop))?;
                }
            } else {
                self.client_tick_steps(steps, Some(event_loop))?;
            }
            return Ok(());
        };
        self.apply_export_replay_step(step, replay.cursor_ns, Some(event_loop))
    }

    fn apply_export_replay_step(
        &mut self,
        step: &FrameStep,
        start: u64,
        event_loop: Option<&dyn EventLoopCtl>,
    ) -> Result<()> {
        let cursor = self.step_export_replay(step, start, event_loop)?;
        if let Some(replay) = self.export.as_mut().and_then(|s| s.replay.as_mut()) {
            replay.cursor_ns = cursor;
            replay.partial = step.partial;
        }
        Ok(())
    }

    pub(super) fn step_export_replay(
        &mut self,
        step: &FrameStep,
        start: u64,
        event_loop: Option<&dyn EventLoopCtl>,
    ) -> Result<u64> {
        let end = u64::from(
            self.replay
                .driver
                .as_ref()
                .ok_or("the replay closed")?
                .duration_ms(),
        ) * NS_PER_MS;
        let finish = drive_replay_frame(step, start, end, |time| {
            if self.export.as_ref().is_some_and(|s| s.draining.is_some()) {
                return Ok(());
            }
            match time {
                ReplayFrameTime::Tick(ns) => self.export_replay_tick(ns, event_loop),
                ReplayFrameTime::End(ns) => self.export_replay_to(recording_ms(ns)),
            }
        })?;
        self.report_export_replay(step.partial);
        Ok(finish)
    }

    fn export_replay_tick(
        &mut self,
        time_ns: u64,
        event_loop: Option<&dyn EventLoopCtl>,
    ) -> Result<()> {
        self.export_replay_to(recording_ms(time_ns))?;
        if self.replay.playing() {
            self.tick_replay_player_model();
            self.client_tick_steps(1, event_loop)?;
        }
        Ok(())
    }

    pub(super) fn pre_roll_export(&mut self, pos: u32) -> Result<()> {
        let (start, ticks) = pre_roll_times(pos);
        self.seek_studio_replay(start)?;
        self.replay.seek_report = false;
        for time in ticks {
            self.export_replay_tick(time, None)?;
        }
        self.export_replay_to(f64::from(pos))?;
        self.report_export_replay((pos % 50) as f32 / 50.0);
        Ok(())
    }

    fn settle_export_replay(&mut self) -> Result<()> {
        let Some(replay) = self.export.as_ref().and_then(|s| s.replay.as_ref()) else {
            return Ok(());
        };
        let has_player_column = self.player.as_ref().is_some_and(|player| {
            let at = player.position;
            self.world
                .chunks
                .contains_key(&((at.x.floor() as i32) >> 4, (at.z.floor() as i32) >> 4))
        });
        if !self
            .remote
            .as_ref()
            .is_some_and(|r| r.level_load.needs_load_packets(has_player_column))
        {
            return Ok(());
        }

        // A portal sends Respawn before it knows where the player will land.
        // Advance through the recorded loading interval without consuming a capture.
        let cursor = replay.cursor_ns;
        if self.replay.driver.as_ref().is_some_and(|r| r.is_finished()) {
            self.begin_export_drain("finished");
        } else {
            let step = self.next_export_step()?;
            self.clock.advance_set(step.sim, step.ticks, step.partial);
            self.apply_export_replay_step(&step, cursor, None)?;
        }
        Ok(())
    }

    pub(super) fn export_draw(&mut self) -> bool {
        if self.export.as_ref().is_some_and(|s| s.draining.is_some()) {
            return false;
        }
        let draw = (|| -> Result<bool> {
            let pending = self.export.as_ref().and_then(|s| s.pending);
            let renderer = self.renderer.as_mut().ok_or("no renderer")?;
            renderer.flush_frame_export_uploads();
            if !self.export_ready()? {
                self.settle_export_replay()?;
                return Ok(false);
            }
            let Some(index) = pending else {
                return Ok(false);
            };
            let renderer = self.renderer.as_mut().ok_or("no renderer")?;
            if !renderer.try_arm_export_frame(index)? {
                return Ok(false);
            }
            let session = self.export.as_ref().ok_or("no frame export")?;
            let elapsed = session.frame_elapsed;
            if !session.spec.hud {
                self.ui.hide_gui = true;
            }
            self.extras_frame(elapsed);
            let drawn = self.render_and_flip(elapsed)?;
            let copy_submitted = self
                .renderer
                .as_mut()
                .is_some_and(|renderer| !renderer.disarm_export_frame());
            if drawn && copy_submitted {
                let session = self.export.as_mut().ok_or("export ended before capture")?;
                session.capture_submitted();
                let limit = session.spec.frames != 0 && session.submitted >= session.spec.frames;
                let replay_ended = session.replay.is_some()
                    && self.replay.driver.as_ref().is_some_and(|r| r.is_finished());
                if session.stop_requested {
                    self.begin_export_drain("stopped");
                } else if limit || replay_ended {
                    self.begin_export_drain("finished");
                }
            } else {
                return Err("renderer refused an armed export frame".into());
            }
            Ok(drawn)
        })();
        match draw {
            Ok(drawn) => drawn,
            Err(error) => {
                self.abort_export(Some(error.to_string()));
                false
            }
        }
    }
}

impl Drop for Game<'_> {
    fn drop(&mut self) {
        if self.export.is_some() {
            self.abort_export(Some("game dropped during export".into()));
        }
    }
}

fn terrain_ready(player: Option<Vec3>, center: (i32, i32), loading: bool) -> bool {
    // Simulation or a flight can move after update_world tracked the view.
    !loading && player.is_some_and(|at| super::player_chunk(at) == center)
}

fn capture_when_ready<T>(
    ready: bool,
    state: &mut T,
    reserve: impl FnOnce(&mut T) -> Result<bool>,
    advance: impl FnOnce(&mut T) -> Result<FrameStep>,
) -> Result<Option<FrameStep>> {
    if ready && reserve(state)? {
        return advance(state).map(Some);
    }
    Ok(None)
}

fn recording_ms(ns: u64) -> f64 {
    ns as f64 / NS_PER_MS as f64
}

enum ReplayFrameTime {
    Tick(u64),
    End(u64),
}

fn drive_replay_frame(
    step: &FrameStep,
    start: u64,
    end: u64,
    mut advance: impl FnMut(ReplayFrameTime) -> Result<()>,
) -> Result<u64> {
    let finish = (start + step.sim.as_nanos() as u64).min(end);
    for time in step
        .simulation_tick_offsets_ns()
        .map(|offset| start + offset)
        .take_while(|&time| time <= finish)
    {
        advance(ReplayFrameTime::Tick(time))?;
    }
    advance(ReplayFrameTime::End(finish))?;
    Ok(finish)
}

fn pre_roll_times(pos: u32) -> (u32, impl Iterator<Item = u64>) {
    let start = pos.saturating_sub(1000);
    // Align to the recording's 20 Hz boundaries. The snapshot supplies time zero.
    let first = (u64::from(start) / 50 + 1) * 50;
    let ticks = (first..=u64::from(pos))
        .step_by(50)
        .map(|time| time * NS_PER_MS);
    (start, ticks)
}

fn idle() -> Value {
    json!({ "ok": true, "active": false, "state": "idle" })
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::{
        packet_source::{PacketSource, TimestampedPacket},
        studio::{McprWriter, Metadata, ReplayDriver},
    };
    use horizon_protocol::{buf::Reader, packets::play};

    #[test]
    fn held_export_accepts_exactly_the_requested_frames_and_automatic_export_gates_commands() {
        let parsed = parse_start(&[
            "take",
            "--size",
            "1080x1920",
            "--fps",
            "30000/1001",
            "--seconds",
            "2",
            "--hold",
            "--no-settle",
        ])
        .unwrap();
        assert_eq!(parsed.spec.frames, 60);
        assert!(parsed.hold && !parsed.settle);
        let mut requests = Requests {
            hold: parsed.hold,
            remaining: 0,
        };
        assert!(requests.held());
        assert_eq!(requests.add(2).unwrap(), 2);
        assert!(requests.check_command(&["look", "30", "0"]).is_err());
        assert!(requests.check_command(&["export", "stop"]).is_ok());
        assert!(requests.check_command(&["quit"]).is_ok());
        assert!(requests.check_command(&["state"]).is_ok());
        requests.captured();
        assert!(!requests.held());
        assert!(requests.check_command(&["tp", "0", "64", "0"]).is_err());
        requests.captured();
        assert!(requests.held());
        assert!(requests.add(0).is_err());
        assert!(Requests::default().add(2).is_err());

        for words in [
            &["tp", "0", "64", "0"][..],
            &["look", "0", "0"],
            &["stats", "reset"],
            &["studio", "replay", "seek", "0"],
        ] {
            assert!(!allowed_while_running(words));
        }
        for words in [
            &["export", "status"][..],
            &["export", "stop"],
            &["events"],
            &["looking-at"],
            &["find-block", "stone"],
            &["wait-for", "event", "export"],
            &["quit"],
        ] {
            assert!(allowed_while_running(words));
        }
        assert_eq!(idle()["active"], false);

        assert!(requests.check_command(&["look", "30", "0"]).is_ok());
        assert!(requests.check_command(&["pause"]).is_err());
    }

    #[test]
    fn automatic_exports_accept_queries_without_enabling_their_mutating_variants() {
        let running = Requests::default();
        for query in [
            &["options-get", "renderDistance"][..],
            &["servers"],
            &["effects"],
            &["book"],
            &["emote"],
            &["lan", "status"],
            &["friends", "status"],
            &["pad", "status"],
            &["profile", "server", "status"],
            &["profile", "status", "--side", "client"],
            &["trace"],
            &["gpu-passes"],
            &["render-capture"],
            &["parallel-tick"],
            &["gpu-culling"],
            &["section-draws"],
            &["section-cull"],
            &["section-sides"],
            &["sculk"],
        ] {
            assert!(running.check_command(query).is_ok(), "{query:?}");
        }
        for mutation in [
            &["options-set", "renderDistance", "8"][..],
            &["connect", "server"],
            &["effect", "clear"],
            &["book", "next"],
            &["emote", "stop"],
            &["lan", "close"],
            &["friends", "stop"],
            &["pad", "release"],
            &["profile", "server", "start"],
            &["trace", "out.hzperf"],
            &["gpu-passes", "on"],
            &["render-capture", "on"],
            &["render-capture", "start"],
            &["parallel-tick", "on"],
            &["gpu-culling", "off"],
            &["section-draws", "flat"],
            &["section-cull", "off"],
            &["section-sides", "on"],
            &["sculk", "warn", "4"],
            &["hurtlog"],
        ] {
            assert!(running.check_command(mutation).is_err(), "{mutation:?}");
        }
    }

    #[test]
    fn replay_settling_reaches_load_start_and_chunk_packets_after_respawn() {
        use horizon_client::level_loading::{LevelLoadTracker, PlayerCheck, Reason};

        // These packets arrive in three separate 30 fps steps. Respawn
        // arrives after the first frame's last tick, as at a portal.
        let mut packets = [20, 60, 85].into_iter().enumerate().peekable();
        let mut clock = ExportClock::new((30, 1)).unwrap();
        let mut load = LevelLoadTracker::new(100);
        let mut ticks = Vec::new();
        let mut has_player_column = false;
        let mut advance = |start, load: &mut LevelLoadTracker, has_player_column: &mut bool| {
            let step = clock.step(1_000_000, 50_000_000).unwrap();
            drive_replay_frame(&step, start, 1_000_000_000, |time| {
                let cursor = match time {
                    ReplayFrameTime::Tick(ns) | ReplayFrameTime::End(ns) => ns,
                };
                while let Some((packet, _)) = packets.next_if(|&(_, ms)| ms * NS_PER_MS <= cursor) {
                    match packet {
                        0 => load.start_client_load(1_000, Reason::NetherPortal),
                        1 => load.loading_packets_received(),
                        2 => *has_player_column = true,
                        _ => unreachable!(),
                    }
                }
                if let ReplayFrameTime::Tick(ns) = time {
                    ticks.push(ns);
                }
                Ok(())
            })
            .unwrap()
        };

        let mut cursor = advance(0, &mut load, &mut has_player_column);
        assert_eq!(cursor, 33_333_333);
        assert!(load.is_active());
        assert!(load.needs_load_packets(true));
        while load.needs_load_packets(has_player_column) {
            cursor = advance(cursor, &mut load, &mut has_player_column);
            assert!(cursor <= 100_000_000);
        }
        assert_eq!(cursor, 100_000_000);
        assert_eq!(ticks, [50_000_000, 100_000_000]);
        assert!(has_player_column);
        assert!(load.is_active());

        let player = PlayerCheck {
            outside_build_height: false,
            spectator: false,
            alive: true,
        };
        assert!(!load.poll_client_load(1_010, false, player));
        assert!(!load.poll_client_load(1_020, true, player));
        assert!(!load.needs_load_packets(false));
        assert!(load.poll_client_load(1_120, true, player));
        assert!(!load.is_active());
    }

    #[test]
    fn the_tick_target_is_sampled_only_after_settle_and_slot_reservation() {
        let mut state = (ExportClock::new((24, 1)).unwrap(), 0);
        for (ready, slot, expected_calls) in [
            (false, false, 0),
            (false, true, 0),
            (true, false, 0),
            (true, true, 1),
            (true, false, 1),
            (true, true, 2),
        ] {
            let step = capture_when_ready(
                ready,
                &mut state,
                |_| {
                    assert!(ready, "reserving a slot before settling");
                    Ok(slot)
                },
                |(clock, samples)| {
                    *samples += 1;
                    Ok(clock.step(1_000_000, 50_000_000)?)
                },
            )
            .unwrap();
            assert_eq!(state.1, expected_calls);
            if ready && slot {
                assert_eq!(step.unwrap().index, expected_calls - 1);
            } else {
                assert!(step.is_none());
            }
        }
    }

    fn test_session(root: &std::path::Path) -> Session {
        let folder = Arc::new(ExportFolder::create(root, "take").unwrap());
        let spec = parse_start(&["take", "--size", "16x16", "--fps", "30"])
            .unwrap()
            .spec;
        Session {
            clock: ExportClock::new(spec.fps).unwrap(),
            mod_request: None,
            stop_requested: false,
            spec,
            requests: Requests::default(),
            settle: true,
            stall_since: None,
            real: Duration::ZERO,
            visual_origin: 0,
            frame_elapsed: Duration::ZERO,
            replay: None,
            pending: None,
            draining: None,
            submitted: 0,
            folder,
            gpu: "fixture".into(),
            previous_hide_gui: false,
            released_mappings: HashSet::new(),
            server_clock: false,
            writer: None,
            cancel_encoder: None,
            progress: Arc::default(),
        }
    }

    #[test]
    fn mod_status_tracks_the_current_capture_until_it_is_submitted() {
        let root = tempfile::tempdir().unwrap();
        let mut session = test_session(root.path());
        let mut api = native_export::NativeExport::default();
        assert!(session.mod_status().is_none());
        assert_eq!(
            api.request_stop(None),
            Err(mod_api::ExportError::NotRunning)
        );
        assert_eq!(
            api.request_stop(Some(&mut session)),
            Err(mod_api::ExportError::NotRunning)
        );
        session.mod_request = Some(31);
        for frame in 0..3 {
            session.begin_capture();
            let status = session.mod_status().unwrap();
            assert_eq!(
                (status.request, status.state, status.frame),
                (31, ExportState::Running, frame)
            );
            assert!(!session.can_begin_frame());
            if frame == 2 {
                api.request_stop(Some(&mut session)).unwrap();
                assert!(!session.stop_between_frames());
            }
            session.capture_submitted();
        }
        assert!(session.stop_between_frames());
        assert_eq!(session.mod_status().unwrap().frame, 3);
    }

    #[test]
    fn mod_completion_follows_drained_output_and_the_manifest_rename() {
        let root = tempfile::tempdir().unwrap();
        let mut session = test_session(root.path());
        session.mod_request = Some(8);
        session.draining = Some(Drain::new("stopped"));
        let writer = Writer::png(Arc::clone(&session.folder), 1).unwrap();
        session.progress = writer.progress();
        for index in 0..2 {
            writer.write(index, [16, 16], vec![0; 16 * 16 * 3]).unwrap();
        }
        session.writer = Some(thread::spawn(move || writer.finish()));
        let writer = session.writer.take().unwrap();
        let completed_bytes = writer.join().unwrap().unwrap();
        let now = Instant::now();
        session.poll_writer_finish(now, STALL).unwrap();
        let job = session
            .draining
            .as_mut()
            .unwrap()
            .manifest
            .as_mut()
            .unwrap();
        let worker = std::mem::replace(
            &mut job.worker,
            Err(std::io::Error::other("joined by test")),
        );
        worker.unwrap().join().unwrap().unwrap();
        let event = session.poll_manifest_finish(now, STALL).unwrap();
        let status = session.mod_completion(&event).unwrap();
        assert_eq!(
            (status.request, status.state, status.frame, status.bytes),
            (8, ExportState::Finished, 2, completed_bytes)
        );
        let manifest: Value = serde_json::from_slice(
            &std::fs::read(session.folder.path().join("export.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["state"], "stopped");
        assert_eq!(manifest["frames_written"], 2);
        assert!(session.folder.path().join("frame_000001.png").is_file());
        assert!(session.poll_manifest_finish(now, STALL).is_none());
        session.mod_request = None;
        assert!(session.mod_completion(&event).is_none());
    }

    #[test]
    fn a_failed_mod_export_settles_with_a_failed_manifest() {
        let root = tempfile::tempdir().unwrap();
        let mut session = test_session(root.path());
        session.mod_request = Some(9);
        let mut job = session.begin_manifest("failed", Some("test readback failure".into()));
        let worker = std::mem::replace(
            &mut job.worker,
            Err(std::io::Error::other("joined by test")),
        );
        worker.unwrap().join().unwrap().unwrap();
        session.draining = Some(Drain {
            manifest: Some(job),
            ..Drain::new("failed")
        });
        let event = session.poll_manifest_finish(Instant::now(), STALL).unwrap();
        let status = session.mod_completion(&event).unwrap();
        assert_eq!(
            (status.request, status.state, status.frame),
            (9, ExportState::Failed, 0)
        );
        let manifest: Value = serde_json::from_slice(
            &std::fs::read(session.folder.path().join("export.json")).unwrap(),
        )
        .unwrap();
        assert_eq!(manifest["state"], "failed");
    }

    #[test]
    fn world_exit_and_disconnection_report_failed_mod_completions() {
        for (request, reason) in [
            (Some(10), "export interrupted because the world closed"),
            (
                Some(11),
                "export interrupted because the client disconnected",
            ),
            (None, "export interrupted because the world closed"),
        ] {
            let root = tempfile::tempdir().unwrap();
            let mut session = test_session(root.path());
            session.mod_request = request;
            let writer = Writer::png(Arc::clone(&session.folder), 1).unwrap();
            session.progress = writer.progress();
            writer
                .write(0, session.spec.size, vec![0; 16 * 16 * 3])
                .unwrap();
            writer.finish().unwrap();

            session.draining = Some(Drain::new("stopped"));
            session.note_world_exit(reason);
            session.note_world_exit("another close path reached the same session");
            session.poll_writer_finish(Instant::now(), STALL).unwrap();
            let mut job = session.take_manifest().unwrap();
            let worker = std::mem::replace(
                &mut job.worker,
                Err(std::io::Error::other("joined by test")),
            );
            worker.unwrap().join().unwrap().unwrap();
            session.draining.as_mut().unwrap().manifest = Some(job);
            let event = session.poll_manifest_finish(Instant::now(), STALL).unwrap();
            let manifest: Value = serde_json::from_slice(
                &std::fs::read(session.folder.path().join("export.json")).unwrap(),
            )
            .unwrap();
            assert_eq!(event["frames_written"], 1);
            assert_eq!(event["state"], manifest["state"]);
            if let Some(id) = request {
                assert_eq!(event["error"], reason);
                let status = session.mod_completion(&event).unwrap();
                assert_eq!(
                    (status.request, status.state, status.frame),
                    (id, ExportState::Failed, 1)
                );
                assert_eq!(status.bytes, event["bytes_written"].as_u64().unwrap());
            } else {
                assert_eq!(event["state"], "stopped");
                assert!(session.mod_completion(&event).is_none());
            }
            assert!(
                session
                    .poll_manifest_finish(Instant::now(), STALL)
                    .is_none()
            );
        }
    }

    #[test]
    fn replacing_a_replay_drains_its_capture_to_a_failed_manifest() {
        use crate::game::{native_replay_tests::write_recording, replay::Replay};
        use horizon_client::studio::ReplayDriver;

        let root = tempfile::tempdir().unwrap();
        let path = root.path().join("outgoing.mcpr");
        write_recording(&path, 1000, &[]);
        let mut replay = Replay::default();
        replay.driver = Some(Box::new(ReplayDriver::open(&path).unwrap()));
        replay.recording = "outgoing".into();
        let mut session = test_session(root.path());
        assert_eq!(
            replay
                .take_replaced_recording(Some(&mut session), None)
                .as_deref(),
            Some("outgoing")
        );
        assert!(
            session.draining.is_none(),
            "a live capture is independent of the replay"
        );

        replay.driver = Some(Box::new(ReplayDriver::open(&path).unwrap()));
        replay.recording = "outgoing".into();
        session.mod_request = Some(1);
        session.replay = Some(ReplayTime {
            generation: replay.generation,
            cursor_ns: 0,
            pre_roll: false,
            initial_tick_phase: false,
            partial: 0.0,
        });

        assert_eq!(
            replay
                .take_replaced_recording(Some(&mut session), None)
                .as_deref(),
            Some("outgoing")
        );
        assert!(replay.driver.is_none());
        session.poll_writer_finish(Instant::now(), STALL).unwrap();
        let job = session.take_manifest().unwrap();
        job.worker.unwrap().join().unwrap().unwrap();
        let status = session.mod_completion(&job.event).unwrap();
        assert_eq!((status.request, status.state), (1, ExportState::Failed));
        assert_eq!(
            job.event["error"],
            "export interrupted because the replay closed"
        );
        let bytes = std::fs::read(session.folder.path().join("export.json")).unwrap();
        let manifest: Value = serde_json::from_slice(&bytes).unwrap();
        assert_eq!(manifest["state"], "failed");
    }

    #[test]
    fn flight_across_a_chunk_boundary_waits_to_capture_the_same_frame() {
        let root = tempfile::tempdir().unwrap();
        let mut session = test_session(root.path());
        let settled = (0, 0);
        assert!(terrain_ready(
            Some(Vec3::new(15.5, 64.0, 0.0)),
            settled,
            false
        ));
        session.begin_capture();
        let elapsed = session.elapsed();
        let moved = Some(Vec3::new(16.5, 64.0, 0.0));

        assert!(!terrain_ready(moved, settled, false));
        assert!(!session.can_begin_frame());
        assert_eq!(session.pending, Some(0));
        assert!(!terrain_ready(moved, (1, 0), true));

        assert!(terrain_ready(moved, (1, 0), false));
        assert_eq!(session.elapsed(), elapsed);
        assert_eq!(session.pending, Some(0));
    }

    #[test]
    fn cancelling_a_drain_does_not_wait_for_a_blocked_writer() {
        let root = tempfile::tempdir().unwrap();
        let mut session = test_session(root.path());
        session.draining = Some(Drain::new("stopped"));
        let (release, hold) = sync_channel(0);
        session.writer = Some(thread::spawn(move || {
            hold.recv().unwrap();
            Ok(0)
        }));
        let (cancel, cancelled) = sync_channel(1);
        session.cancel_encoder = Some(Box::new(move || cancel.send(()).unwrap()));

        let (reply, result) = sync_channel(1);
        let control = thread::spawn(move || {
            assert!(!session.writer_finished());
            session.cancel_output();
            let manifest = session.begin_manifest("failed", Some("export cancelled".into()));
            reply.send(manifest.finish("export aborted")).unwrap();
        });
        let value = result.recv_timeout(Duration::from_secs(2));
        release.send(()).unwrap();
        control.join().unwrap();

        assert_eq!(value.unwrap()["state"], "failed");
        cancelled.try_recv().unwrap();
    }

    #[test]
    fn a_stalled_writer_fails_at_the_finish_deadline() {
        let root = tempfile::tempdir().unwrap();
        let mut session = test_session(root.path());
        session.draining = Some(Drain::new("stopped"));
        let (release, hold) = sync_channel(1);
        session.writer = Some(thread::spawn(move || {
            hold.recv_timeout(Duration::from_secs(5)).unwrap();
            Ok(0)
        }));

        let start = Instant::now();
        let budget = Duration::from_millis(50);
        session.poll_writer_finish(start, budget).unwrap();
        session
            .poll_writer_finish(start + budget - Duration::from_millis(1), budget)
            .unwrap();
        assert!(session.take_manifest().is_none());
        let err = session
            .poll_writer_finish(start + budget, budget)
            .unwrap_err();
        assert_eq!(
            err.downcast_ref::<std::io::Error>().unwrap().kind(),
            std::io::ErrorKind::TimedOut
        );
        session.cancel_output();
        let manifest = session.begin_manifest("failed", Some(err.to_string()));
        assert_eq!(manifest.finish("export aborted")["state"], "failed");
        release.send(()).unwrap();
    }

    #[test]
    fn a_stalled_manifest_fails_at_the_publication_deadline() {
        let root = tempfile::tempdir().unwrap();
        let mut session = test_session(root.path());
        let (release, hold) = sync_channel(1);
        let manifest = ManifestJob::spawn(session.status("finished"), move |_| {
            hold.recv_timeout(Duration::from_secs(5)).unwrap();
            Ok(())
        });
        let started = manifest.started;
        let publication = Arc::clone(&manifest.publication);
        session.draining = Some(Drain {
            manifest: Some(manifest),
            ..Drain::new("finished")
        });

        let budget = Duration::from_millis(50);
        assert!(session.poll_manifest_finish(started, budget).is_none());
        assert!(
            session
                .poll_manifest_finish(started + budget - Duration::from_millis(1), budget)
                .is_none()
        );
        let failed = session
            .poll_manifest_finish(started + budget, budget)
            .unwrap();
        release.send(()).unwrap();
        assert_eq!(failed["state"], "failed");
        assert_eq!(failed["ok"], false);
        assert_eq!(
            failed["error"],
            "export manifest stalled while publishing export.json"
        );
        assert_eq!(publication.state(), ManifestState::Cancelled);
        assert!(session.take_manifest().is_none());
    }

    #[test]
    fn a_mod_finishes_failed_at_the_deadline_even_inside_manifest_rename() {
        let root = tempfile::tempdir().unwrap();
        let mut session = test_session(root.path());
        session.mod_request = Some(13);
        let (entered, publishing) = sync_channel(0);
        let (release, stalled) = sync_channel(0);
        let manifest = ManifestJob::spawn(session.status("finished"), move |publication| {
            publication.publish(|| {
                entered.send(()).unwrap();
                stalled.recv_timeout(Duration::from_secs(5)).unwrap();
                Ok(())
            })
        });
        publishing.recv_timeout(Duration::from_secs(2)).unwrap();
        let start = manifest.started;
        let publication = Arc::clone(&manifest.publication);
        session.draining = Some(Drain {
            manifest: Some(manifest),
            ..Drain::new("finished")
        });
        let budget = Duration::from_millis(50);
        assert!(
            session
                .poll_manifest_finish(start + budget / 2, budget)
                .is_none()
        );
        let event = session.poll_manifest_finish(start + budget, budget);
        assert_eq!(publication.state(), ManifestState::Publishing);
        release.send(()).unwrap();

        let event = event.expect("do not hold the export open for a stalled rename");
        assert_eq!(event["active"], false);
        assert!(
            event["error"]
                .as_str()
                .unwrap()
                .contains("publication unconfirmed")
        );
        let status = session.mod_completion(&event).unwrap();
        assert_eq!(
            (status.request, status.state, status.frame),
            (13, ExportState::Failed, 0)
        );
        assert!(
            session
                .poll_manifest_finish(start + budget, budget)
                .is_none()
        );
    }

    #[test]
    fn an_aborted_control_export_can_publish_its_failure_after_the_reply() {
        let root = tempfile::tempdir().unwrap();
        let session = test_session(root.path());
        let (release, hold) = sync_channel(0);
        let folder = Arc::clone(&session.folder);
        let spec = session.spec.clone();
        let (published, done) = sync_channel(0);
        let mut event = session.status("failed");
        ManifestJob::mark_failed(&mut event, "readback failed");
        let job = ManifestJob::spawn(event, move |publication| {
            hold.recv_timeout(Duration::from_secs(5)).unwrap();
            let result = folder.write_manifest_tracked(
                &ExportManifest {
                    spec: &spec,
                    state: "failed",
                    frames_written: 0,
                    build: "test",
                    gpu: "fixture",
                },
                publication,
            );
            published.send(result).unwrap();
            Ok(())
        });

        let event = job.finish("export aborted");
        release.send(()).unwrap();
        done.recv_timeout(Duration::from_secs(2)).unwrap().unwrap();
        assert_eq!(event["error"], "readback failed");
        assert!(session.folder.path().join("export.json").is_file());
    }

    #[test]
    fn quitting_during_manifest_publication_reports_failure() {
        let root = tempfile::tempdir().unwrap();
        let mut session = test_session(root.path());
        let (release, hold) = sync_channel(1);
        let (started, publishing) = sync_channel(1);
        let manifest = ManifestJob::spawn(session.status("stopped"), move |_| {
            started.send(()).unwrap();
            hold.recv_timeout(Duration::from_secs(5)).unwrap();
            Ok(())
        });
        let publication = Arc::clone(&manifest.publication);
        publishing.recv_timeout(Duration::from_secs(2)).unwrap();
        session.draining = Some(Drain {
            manifest: Some(manifest),
            ..Drain::new("stopped")
        });
        let (reply, result) = sync_channel(1);
        let control = thread::spawn(move || {
            let job = session.take_manifest().unwrap();
            assert!(!job.is_finished());
            reply
                .send(job.finish("export cancelled because the game is closing"))
                .unwrap();
        });
        let quit = result.recv_timeout(Duration::from_secs(2));
        release.send(()).unwrap();
        control.join().unwrap();
        let quit = quit.unwrap();
        assert_eq!(quit["state"], "failed");
        assert_eq!(quit["ok"], false);
        assert_eq!(
            quit["error"],
            "export cancelled because the game is closing"
        );
        assert_eq!(publication.state(), ManifestState::Cancelled);
    }

    #[test]
    fn quitting_after_manifest_rename_keeps_the_published_state() {
        let root = tempfile::tempdir().unwrap();
        let session = test_session(root.path());
        let folder = Arc::clone(&session.folder);
        let (renamed, published) = sync_channel(1);
        let (release, cleanup) = sync_channel(1);
        let spec = session.spec.clone();
        let manifest = ManifestJob::spawn(session.status("stopped"), move |publication| {
            folder.write_manifest_tracked(
                &ExportManifest {
                    spec: &spec,
                    state: "stopped",
                    frames_written: 0,
                    build: "test",
                    gpu: "fixture",
                },
                publication,
            )?;
            renamed.send(()).unwrap();
            // Keep the worker busy after a real rename, as during slow staging cleanup.
            cleanup.recv_timeout(Duration::from_secs(5)).unwrap();
            Ok(())
        });
        published.recv_timeout(Duration::from_secs(2)).unwrap();
        assert!(!manifest.worker_finished());

        let (reply, result) = sync_channel(1);
        let control = thread::spawn(move || {
            reply
                .send(manifest.finish("export cancelled because the game is closing"))
                .unwrap();
        });
        let event = result.recv_timeout(Duration::from_secs(2));
        release.send(()).unwrap();
        control.join().unwrap();

        let event = event.unwrap();
        let file = std::fs::File::open(session.folder.path().join("export.json")).unwrap();
        let manifest: Value = serde_json::from_reader(file).unwrap();
        assert_eq!(event["ok"], true);
        assert_eq!(event["state"], "stopped");
        assert_eq!(event["state"], manifest["state"]);
    }

    #[test]
    fn stopping_flushes_queued_frames_and_writes_the_final_status() {
        let root = tempfile::tempdir().unwrap();
        let mut session = test_session(root.path());
        let writer = Writer::png(Arc::clone(&session.folder), 1).unwrap();
        session.progress = writer.progress();
        writer
            .write(0, session.spec.size, vec![19; 16 * 16 * 3])
            .unwrap();
        session.submitted = 1;
        session.draining = Some(Drain::new("stopped"));
        session.writer = Some(thread::spawn(move || writer.finish()));
        let deadline = Instant::now() + Duration::from_secs(5);
        while !session.writer_finished() {
            assert!(Instant::now() < deadline);
            thread::yield_now();
        }
        assert_eq!(session.status("draining")["active"], true);
        session
            .poll_writer_finish(Instant::now(), Duration::from_secs(1))
            .unwrap();
        let manifest = session.take_manifest().unwrap();
        while !manifest.is_finished() {
            assert!(Instant::now() < deadline);
            thread::yield_now();
        }
        let stopped = manifest.finish("export manifest is pending");
        assert_eq!(stopped["active"], false);
        assert_eq!(stopped["state"], "stopped");
        assert_eq!(stopped["frames_written"], 1);
        let dir = session.folder.path();
        let manifest: Value =
            serde_json::from_slice(&std::fs::read(dir.join("export.json")).unwrap()).unwrap();
        assert_eq!(manifest["state"], "stopped");
        assert_eq!(manifest["frames_written"], 1);
        let file =
            std::io::BufReader::new(std::fs::File::open(dir.join("frame_000000.png")).unwrap());
        let mut reader = png::Decoder::new(file).read_info().unwrap();
        let mut pixels = vec![0; 16 * 16 * 3];
        reader.next_frame(&mut pixels).unwrap();
        assert!(pixels.iter().all(|&p| p == 19));
    }

    /// A replay packet schedules a three-tick entity move. At a clock boundary
    /// the packet must reach the entity before the corresponding client tick.
    #[derive(Default)]
    struct Entity {
        x: f64,
        target: f64,
        ticks: u32,
    }

    impl Entity {
        fn packets(&mut self, packets: Vec<TimestampedPacket>) {
            for pkt in packets {
                if let Ok(play::Clientbound::EntityPositionSync(sync)) =
                    play::Clientbound::decode(pkt.id, &pkt.payload)
                    && let play::PositionPath::Linear([x, ..]) = sync.path
                {
                    self.target = x;
                    self.ticks = 3;
                }
            }
        }

        fn tick(&mut self) {
            if self.ticks != 0 {
                self.x += (self.target - self.x) / f64::from(self.ticks);
                self.ticks -= 1;
            }
        }
    }

    #[test]
    fn replay_frame_rates_and_pre_roll_deliver_moves_at_the_same_recording_ticks() {
        let tmp = tempfile::tempdir().unwrap();
        let path = tmp.path().join("moves.mcpr");
        let mut writer = McprWriter::new(
            path.clone(),
            Metadata {
                duration: 1500,
                ..Default::default()
            },
        )
        .unwrap();
        let mut store = |ms, packet: play::Clientbound| {
            let raw = packet.encode();
            let mut input = Reader::new(&raw);
            let id = input.varint().unwrap();
            writer.write_packet(ms, id, input.rest()).unwrap();
        };
        store(
            0,
            play::Clientbound::Login(play::Login {
                player_id: 1,
                hardcore: false,
                levels: vec!["minecraft:overworld".into()],
                max_players: 1,
                chunk_radius: 2,
                simulation_distance: 2,
                reduced_debug_info: false,
                show_death_screen: true,
                limited_crafting: false,
                spawn: play::SpawnInfo {
                    dimension_type: 0,
                    dimension: "minecraft:overworld".into(),
                    seed: 0,
                    game_type: 0,
                    previous_game_type: None,
                    debug: false,
                    flat: false,
                    last_death: None,
                    portal_cooldown: 0,
                    sea_level: 63,
                },
                online_mode: false,
                enforces_secure_chat: false,
            }),
        );
        for (time, x) in [
            (100, 6.0),
            (200, 15.0),
            (800, 18.0),
            (950, 27.0),
            (1000, 37.0),
        ] {
            store(
                time,
                play::Clientbound::EntityPositionSync(play::EntityPositionSync {
                    id: 2,
                    path: play::PositionPath::Linear([x, 64.0, 0.0]),
                    yaw: 0.0,
                    pitch: 0.0,
                    on_ground: true,
                }),
            );
        }
        writer.finish(1500).unwrap();
        let open = || ReplayDriver::open(&path).unwrap();
        let mut driver = open();
        let mut normal = Entity::default();
        for _ in 0..20 {
            normal.packets(driver.advance(Duration::from_millis(50)).unwrap());
            normal.tick();
        }
        assert_eq!(driver.position_ms(), 1000);
        // The move at 950 got one tick (18 -> 21) before the move at 1000 replaced it.
        assert_eq!(normal.x, 21.0 + 16.0 / 3.0);

        for (fps, speed) in [(24, 1), (30, 1), (60, 1), (24, 2), (30, 3), (60, 3)] {
            let mut clock = ExportClock::new((fps, 1)).unwrap();
            let mut driver = open();
            let mut actual = Entity::default();
            let mut cursor = 0;
            while cursor < 1_000_000_000 {
                let step = clock.step(speed * 1_000_000, 50_000_000).unwrap();
                cursor = drive_replay_frame(&step, cursor, 1_000_000_000, |time| {
                    let ns = match time {
                        ReplayFrameTime::Tick(ns) | ReplayFrameTime::End(ns) => ns,
                    };
                    actual.packets(driver.advance_to(recording_ms(ns))?);
                    if matches!(time, ReplayFrameTime::Tick(_)) {
                        actual.tick();
                    }
                    Ok(())
                })
                .unwrap();
            }
            assert_eq!(driver.position_ms(), 1000);
            assert_eq!(actual.x, normal.x, "{fps} fps at {speed}x");
        }

        // Reconstruct a standalone replay sought past the target. Pre-roll
        // supplies all the entity ticks that a direct seek skipped.
        driver.seek_and_collect(1200).unwrap();
        let (start, ticks) = pre_roll_times(1000);
        let mut rebuilt = Entity::default();
        rebuilt.packets(driver.seek_and_collect(start).unwrap());
        let mut pre_rolled = 0;
        for time in ticks {
            pre_rolled += 1;
            rebuilt.packets(driver.advance_to(recording_ms(time)).unwrap());
            rebuilt.tick();
        }
        assert_eq!(pre_rolled, 20);
        assert_eq!(rebuilt.x, normal.x);
        assert_eq!(rebuilt.ticks, normal.ticks);
    }
}
