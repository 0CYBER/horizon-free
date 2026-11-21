//! Recording a session's packets as `.mcpr` and playing them back in a `Game`: the `/studio record`
//! and `/studio replay` commands (chat and `horizon-ctl`), the replay's clock and its world. This is
//! platform (docs/layers.md, "Replays"); Studio's editor (`studio.rs`) builds on it.

use super::{Game, PendingWorldEntry, Screen};
use horizon_client::{
    Result,
    packet_source::PacketSource,
    studio::{Metadata, ReplayDriver},
    text::Text,
};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

/// The packet replay a `Game` plays, if any, and its playback clock.
#[derive(Default)]
pub(super) struct Replay {
    /// The packet replay playing, if any.
    pub(super) driver: Option<Box<dyn PacketSource>>,
    /// Camera perspective to restore after leaving replay playback.
    pub(super) previous_camera_type: Option<horizon_client::camera::CameraType>,
    /// Replay opened from the title screen owns the synthetic remote session.
    pub(super) standalone: bool,
    pub(super) last_tick: Option<std::time::Instant>,
    pub(super) virtual_tick_ms: f32,
    recording_error_reported: bool,
    /// The playing recording's id: its file name without `.mcpr`.
    pub(super) recording: String,
    /// Counts the replays opened, so a mod's close or seek reaches only the replay it was asked
    /// of.
    pub(super) generation: u64,
    /// The position mods were last told of (`replay_advanced`); none until `replay_opened`.
    pub(super) reported_ms: Option<u32>,
    /// A mod's close or seek, run at the end of the frame (`native_replay_requests`).
    pub(super) request: Option<super::native_replay::ReplayRequest>,
    /// The one thread reading a mod's recording (`replay_open`).
    pub(super) opening: Option<super::native_replay::ReplayOpening>,
    /// A mod's accepted open waiting for that thread; a later one takes its place.
    pub(super) queued: Option<super::native_replay::QueuedOpen>,
    /// Mods' accepted opens that ended without a replay, told at the frame's end.
    pub(super) open_failures: Vec<(super::native_replay::ReplayOrigin, mod_api::ReplayError)>,
    /// The connections this window has made: an open asked for before one was made (and maybe
    /// left) is no longer for the session it was asked in.
    pub(super) connections: u64,
    /// `connections` when the replay playing took up its session: a replay whose session has
    /// gone since is no longer playing on the one there now.
    pub(super) connection: u64,
    /// The archive a mod's replay was opened from (`replay_open`): the editor saves its markers
    /// and camera path only into that very file, never into one put in its place since.
    pub(super) archive: Option<super::native_replay::FileIdentity>,
    /// The archive file the replay playing read (any route), refreshed from the file each
    /// editor save writes: what a mod's reopen of it is compared with, without opening it.
    pub(super) identity: Option<super::native_replay::FileIdentity>,
    /// A seek was applied: the next frame reports its position (`replay_advanced`) and holds the
    /// clock there, so the frame drawn shows exactly where the seek went.
    pub(super) seek_report: bool,
    /// The folder mods open recordings from by id; the game folder's `recordings/` when none.
    pub(super) recordings_dir: Option<PathBuf>,
}

impl Replay {
    fn recording_error(
        &mut self,
        status: Option<&horizon_client::net::RecorderStatus>,
    ) -> Option<String> {
        let Some(status) = status else {
            self.recording_error_reported = false;
            return None;
        };
        if status.active {
            self.recording_error_reported = false;
            return None;
        }
        let error = status.error.as_ref()?;
        if self.recording_error_reported {
            return None;
        }
        self.recording_error_reported = true;
        Some(error.clone())
    }

    /// A replay plays: the world shown is the recording's.
    pub(super) fn playing(&self) -> bool {
        self.driver.is_some()
    }
    /// How far the replay's player is between its last two ticks, while a replay plays: it
    /// moves on the replay's clock, not the game's.
    pub(super) fn player_alpha(&self) -> Option<f64> {
        self.playing()
            .then(|| (f64::from(self.virtual_tick_ms) / 50.0).clamp(0.0, 1.0))
    }
}

impl Game<'_> {
    /// How far the window's own player model is between two ticks: on the replay's clock while a
    /// replay plays, else the game's.
    pub(super) fn own_player_alpha(&self) -> f64 {
        self.replay
            .player_alpha()
            .unwrap_or_else(|| self.clock.camera_fraction())
    }

    /// In-game chat command handler: `/studio <record|replay> ...`.
    /// Returns true if the command was recognized and handled.
    pub(super) fn studio_command(&mut self, command: &str) -> bool {
        let Some(rest) = command.strip_prefix("studio") else {
            return false;
        };
        if !rest.is_empty() && !rest.starts_with(char::is_whitespace) {
            return false;
        }
        let words: Vec<&str> = rest.split_whitespace().collect();
        if words.is_empty() {
            self.system_message(Text::translate("horizon.studio.replay.usage", &[]));
            return true;
        }

        match words[0] {
            "freecam" | "keyframe" | "path" if super::studio::EDITOR => {
                return self.studio_editor_command(&words);
            }
            "record" => match words.get(1).copied() {
                Some("start") => match self.start_studio_recording(words.get(2).copied()) {
                    Ok(v) if v["ok"].as_bool() == Some(true) => {
                        let path = v["path"].as_str().unwrap_or_default();
                        self.system_message(Text::tr("horizon.studio.record.started", &[&path]));
                    }
                    Ok(v) => {
                        let error = v["error"].as_str().unwrap_or_default();
                        self.system_message(Text::tr("horizon.studio.record.error", &[&error]));
                    }
                    Err(error) => {
                        self.system_message(Text::tr("horizon.studio.record.error", &[&error]));
                    }
                },
                Some("stop") => match self.stop_studio_recording() {
                    Ok(v) if v["ok"].as_bool() == Some(true) => {
                        let path = v["path"].as_str().unwrap_or_default();
                        self.system_message(Text::tr("horizon.studio.record.saved", &[&path]));
                    }
                    Ok(v) => {
                        let error = v["error"].as_str().unwrap_or_default();
                        self.system_message(Text::tr("horizon.studio.record.error", &[&error]));
                    }
                    Err(error) => {
                        self.system_message(Text::tr("horizon.studio.record.error", &[&error]));
                    }
                },
                Some("status") => {
                    let status = self.studio_recording_status();
                    if status["active"].as_bool() == Some(true) {
                        let elapsed =
                            (status["elapsed_ms"].as_u64().unwrap_or(0) / 1000).to_string();
                        let packets = status["packet_count"].as_u64().unwrap_or(0).to_string();
                        let size =
                            (status["bytes_written"].as_u64().unwrap_or(0) / 1024).to_string();
                        let path = status["path"].as_str().unwrap_or_default();
                        self.system_message(Text::tr(
                            "horizon.studio.record.status.active",
                            &[&elapsed, &packets, &size, &path],
                        ));
                    } else if let Some(error) = status["error"].as_str() {
                        self.system_message(Text::tr("horizon.studio.record.error", &[&error]));
                    } else {
                        self.system_message(Text::translate(
                            "horizon.studio.record.status.inactive",
                            &[],
                        ));
                    }
                }
                _ => self.system_message(Text::translate("horizon.studio.record.usage", &[])),
            },
            "replay" => match words.get(1).copied() {
                Some("open") => {
                    let Some(path) = words.get(2) else {
                        self.system_message(Text::translate(
                            "horizon.studio.replay.open.usage",
                            &[],
                        ));
                        return true;
                    };
                    match self.open_studio_replay(path) {
                        // The replay opens once the world is left: nothing to report yet.
                        Ok(v) if v["pending"].as_bool() == Some(true) => {}
                        Ok(v) if v["ok"].as_bool() == Some(true) => {
                            let duration_s = v["duration_ms"].as_f64().unwrap_or(0.0) / 1000.0;
                            let packets = v["packet_count"].as_u64().unwrap_or(0);
                            let duration = format!("{duration_s:.1}");
                            let packets = packets.to_string();
                            self.system_message(Text::tr(
                                "horizon.studio.replay.opened",
                                &[path, &duration, &packets],
                            ));
                        }
                        Ok(v) => {
                            let err = v["error"].as_str().unwrap_or_default();
                            self.system_message(Text::tr(
                                "horizon.studio.replay.open_error",
                                &[&err],
                            ));
                        }
                        Err(e) => {
                            let error = e.to_string();
                            self.system_message(Text::tr(
                                "horizon.studio.replay.open_error",
                                &[&error],
                            ));
                        }
                    }
                }
                Some("pause") => {
                    if let Some(r) = self.replay.driver.as_mut() {
                        r.set_paused(true);
                        self.system_message(Text::translate("horizon.studio.replay.paused", &[]));
                    } else {
                        self.system_message(Text::translate("horizon.studio.replay.inactive", &[]));
                    }
                }
                Some("resume") => {
                    if let Some(r) = self.replay.driver.as_mut() {
                        r.set_paused(false);
                        self.system_message(Text::translate("horizon.studio.replay.resumed", &[]));
                    } else {
                        self.system_message(Text::translate("horizon.studio.replay.inactive", &[]));
                    }
                }
                Some("speed") => {
                    let Some(spd_str) = words.get(2) else {
                        self.system_message(Text::translate(
                            "horizon.studio.replay.speed.usage",
                            &[],
                        ));
                        return true;
                    };
                    if let Ok(spd) = spd_str.parse::<f64>() {
                        let new_speed = if let Some(r) = self.replay.driver.as_mut() {
                            r.set_speed(spd);
                            Some(r.speed())
                        } else {
                            None
                        };
                        if let Some(speed) = new_speed {
                            let speed = format!("{speed:.2}");
                            self.system_message(Text::tr(
                                "horizon.studio.replay.speed.set",
                                &[&speed],
                            ));
                        } else {
                            self.system_message(Text::translate(
                                "horizon.studio.replay.inactive",
                                &[],
                            ));
                        }
                    } else {
                        self.system_message(Text::translate(
                            "horizon.studio.replay.speed.invalid",
                            &[],
                        ));
                    }
                }
                Some("seek") => {
                    let Some(ms_str) = words.get(2) else {
                        self.system_message(Text::translate(
                            "horizon.studio.replay.seek.usage",
                            &[],
                        ));
                        return true;
                    };
                    if let Ok(ms) = ms_str.parse::<u32>() {
                        if self.replay.driver.is_some() {
                            match self.seek_studio_replay(ms) {
                                Ok(()) => {
                                    let actual_ms = self
                                        .replay
                                        .driver
                                        .as_ref()
                                        .map_or(ms, |replay| ms.min(replay.duration_ms()));
                                    let ms_text = actual_ms.to_string();
                                    let seconds = format!("{:.1}", actual_ms as f32 / 1000.0);
                                    self.system_message(Text::tr(
                                        "horizon.studio.replay.seeked",
                                        &[&ms_text, &seconds],
                                    ));
                                }
                                Err(e) => {
                                    if e.kind() == std::io::ErrorKind::Unsupported {
                                        self.system_message(Text::translate(
                                            "horizon.studio.replay.seek.backward_requires_standalone",
                                            &[],
                                        ));
                                    } else {
                                        let error = e.to_string();
                                        self.system_message(Text::tr(
                                            "horizon.studio.replay.seek.error",
                                            &[&error],
                                        ));
                                    }
                                }
                            }
                        } else {
                            self.system_message(Text::translate(
                                "horizon.studio.replay.inactive",
                                &[],
                            ));
                        }
                    } else {
                        self.system_message(Text::translate(
                            "horizon.studio.replay.seek.invalid",
                            &[],
                        ));
                    }
                }
                Some("status") => {
                    if let Some(r) = self.replay.driver.as_ref() {
                        let position = r.position_ms().to_string();
                        let duration = r.duration_ms().to_string();
                        let progress = format!("{:.1}", r.progress() * 100.0);
                        let speed = format!("{:.2}", r.speed());
                        let paused = r.is_paused().to_string();
                        let finished = r.is_finished().to_string();
                        self.system_message(Text::tr(
                            "horizon.studio.replay.status",
                            &[&position, &duration, &progress, &speed, &paused, &finished],
                        ));
                    } else {
                        self.system_message(Text::translate("horizon.studio.replay.inactive", &[]));
                    }
                }
                Some("close") => {
                    if self.close_studio_replay() {
                        self.system_message(Text::translate("horizon.studio.replay.closed", &[]));
                    } else {
                        self.system_message(Text::translate("horizon.studio.replay.inactive", &[]));
                    }
                }
                _ => {
                    self.system_message(Text::translate("horizon.studio.replay.usage", &[]));
                }
            },
            _ => {
                self.system_message(Text::translate("horizon.studio.replay.usage", &[]));
            }
        }
        true
    }

    /// Reset playback tick accumulator and align positions on seek.
    pub(super) fn reset_studio_playback_ticks(&mut self) {
        self.replay.virtual_tick_ms = 0.0;
        if let Some(player) = &self.player {
            self.previous_position = player.position;
            if let Some(m) = self.player_model.as_mut() {
                m.previous = player.position;
                m.position = player.position;
                m.limb_speed = 0.0;
                m.previous_limb_speed = 0.0;
            }
        }
    }

    /// Reconstruct replay state at a target timestamp. Standalone replays can
    /// safely reset their synthetic remote; an in-world replay can only seek
    /// forward because its pre-replay world must remain intact.
    pub(super) fn seek_studio_replay(&mut self, target_ms: u32) -> std::io::Result<()> {
        let current_ms = self
            .replay
            .driver
            .as_ref()
            .map_or(0, |driver| driver.position_ms());
        let packets = if self.replay.standalone {
            let packets = self
                .replay
                .driver
                .as_mut()
                .expect("replay checked by caller")
                .seek_and_collect(target_ms)?;
            self.init_remote_for_replay()
                .map_err(|error| std::io::Error::other(error.to_string()))?;
            self.reset_replay_world_extras();
            packets
        } else {
            if target_ms < current_ms {
                return Err(std::io::Error::new(
                    std::io::ErrorKind::Unsupported,
                    "backward seek requires a standalone replay",
                ));
            }
            self.replay
                .driver
                .as_mut()
                .expect("replay checked by caller")
                .seek_forward(target_ms)?
        };

        self.dispatch_studio_replay_packets(packets);
        if !self.replay.playing() {
            return Ok(());
        }
        self.reset_studio_playback_ticks();
        self.replay.last_tick = Some(std::time::Instant::now());
        self.replay.seek_report = true;
        Ok(())
    }

    /// What the recording's packets set on the game outside the replay's session (which
    /// `init_remote_for_replay` makes anew), back as a fresh open has it, so the world rebuilt
    /// from the start shows only what the packets replayed set.
    fn reset_replay_world_extras(&mut self) {
        self.border = Default::default();
        self.tab_list = Default::default();
        self.score.board = Default::default();
        self.titles.clear();
        self.titles.reset_times();
        self.titles.overlay_time = 0;
        self.clock.set_rate(20.0);
        self.clock.set_frozen(false);
        self.clock.set_frozen_ticks_to_run(0);
        self.adv = Default::default();
        self.stats = Default::default();
        self.post_effects = Vec::new();
        self.cooldowns = Default::default();
        self.sleeping = None;
        self.sleep_timer = 0;
    }

    fn dispatch_studio_replay_packets(
        &mut self,
        packets: Vec<horizon_client::studio::mcpr::TimestampedPacket>,
    ) {
        let generation = self.replay.generation;
        for pkt in packets {
            // A packet whose handler left the session (a chat it can't place) ended the
            // replay: the rest of the batch has no session to go to.
            if !self.replay.playing() || self.replay.generation != generation {
                break;
            }
            match horizon_protocol::packets::play::Clientbound::decode(pkt.id, &pkt.payload) {
                Ok(packet) => {
                    let _ = self.handle_packet(packet);
                }
                Err(e) => {
                    let name = horizon_protocol::ids::name(
                        horizon_protocol::State::Play,
                        horizon_protocol::Direction::Clientbound,
                        pkt.id,
                    )
                    .unwrap_or("?");
                    log::debug!(
                        "Studio replay skipped packet {name} (id {:#x}): {e}",
                        pkt.id
                    );
                }
            }
        }
    }

    /// Advance active replay playback by elapsed frame time and dispatch ready packets.
    pub(super) fn advance_studio_replay(&mut self) -> Result<()> {
        self.advance_studio_replay_at(std::time::Instant::now())
    }

    /// [`Self::advance_studio_replay`] for a frame drawn at `now`.
    pub(super) fn advance_studio_replay_at(&mut self, now: std::time::Instant) -> Result<()> {
        let status = self
            .remote
            .as_ref()
            .and_then(|r| r.session.recorder_status());
        if let Some(error) = self.replay.recording_error(status.as_ref()) {
            log::warn!("Studio recording stopped: {error}");
            self.system_message(Text::tr("horizon.studio.record.error", &[&error]));
        }
        let Some(driver) = self.replay.driver.as_mut() else {
            self.replay.last_tick = None;
            return Ok(());
        };

        if std::mem::take(&mut self.replay.seek_report) {
            // The frame after a seek shows the seek's position and says so; the clock runs on
            // from the next one.
            let position = driver.position_ms();
            self.replay.last_tick = Some(now);
            self.studio_replay_advanced();
            if self.replay.reported_ms.is_some() {
                self.replay.reported_ms = Some(position);
                self.native_replay_advanced(position);
            }
            return Ok(());
        }
        let delta = match self.replay.last_tick {
            Some(last) => now
                .saturating_duration_since(last)
                .min(Duration::from_millis(250)),
            None => Duration::from_millis(16),
        };
        self.replay.last_tick = Some(now);

        let ready_packets = match driver.advance(delta) {
            Ok(pkts) => pkts,
            Err(e) => {
                log::warn!("Studio replay advance error: {e}");
                return Ok(());
            }
        };

        self.dispatch_studio_replay_packets(ready_packets);
        if !self.replay.playing() {
            return Ok(());
        }
        let paused = self
            .replay
            .driver
            .as_ref()
            .is_some_and(|replay| replay.is_paused());
        if self.player_model.is_none() {
            self.tick_player_model();
        }
        if !paused {
            let speed = self
                .replay
                .driver
                .as_ref()
                .map_or(1.0, |r| r.speed() as f32)
                .max(0.1);
            self.replay.virtual_tick_ms += delta.as_secs_f32() * 1000.0 * speed;
            while self.replay.virtual_tick_ms >= 50.0 {
                self.replay.virtual_tick_ms -= 50.0;
                if let Some(player) = &self.player {
                    let current_pos = player.position;
                    let (dx, dz) = (
                        current_pos.x - self.previous_position.x,
                        current_pos.z - self.previous_position.z,
                    );
                    let walked = (dx * dx + dz * dz).sqrt() as f32;
                    if let Some(m) = self.player_model.as_mut() {
                        m.previous = self.previous_position;
                        m.position = current_pos;
                        m.previous_limb_speed = m.limb_speed;
                        let target_speed = (walked * 4.0).min(1.0);
                        m.limb_speed += (target_speed - m.limb_speed) * 0.4;
                        if m.limb_speed < 0.001 {
                            m.limb_speed = 0.0;
                        }
                        m.limb_position += m.limb_speed;
                        m.previous_yaw = m.yaw;
                        m.previous_head_yaw = m.head_yaw;
                        m.previous_head_pitch = m.head_pitch;
                        m.previous_body_yaw = m.body_yaw;
                        m.yaw = self.yaw;
                        m.head_yaw = self.yaw;
                        m.head_pitch = self.pitch;
                        let mut target = m.body_yaw;
                        if dx * dx + dz * dz > 0.0025 {
                            let walk = (dz.atan2(dx) as f32).to_degrees() - 90.0;
                            let facing =
                                ((self.yaw - walk + 180.0).rem_euclid(360.0) - 180.0).abs();
                            target = if 95.0 < facing && facing < 265.0 {
                                walk - 180.0
                            } else {
                                walk
                            };
                        }
                        let diff = (target - m.body_yaw + 180.0).rem_euclid(360.0) - 180.0;
                        m.body_yaw += diff * 0.3;
                        let body_diff = (self.yaw - m.body_yaw + 180.0).rem_euclid(360.0) - 180.0;
                        if body_diff.abs() > 50.0 {
                            m.body_yaw += body_diff - body_diff.signum() * 50.0;
                        }
                        m.age = m.age.wrapping_add(1);
                    }
                    self.previous_position = current_pos;
                }
            }
        } else if let Some(m) = self.player_model.as_mut() {
            m.previous_limb_speed = m.limb_speed;
        }
        self.studio_replay_advanced();
        self.announce_replay_advance();
        Ok(())
    }

    /// Control socket command handler for `studio`.
    pub(super) fn studio_control(&mut self, words: &[&str]) -> Result<Option<Value>> {
        if words.first().copied() != Some("studio") {
            return Ok(None);
        }
        match words.get(1).copied() {
            Some("record") => match words.get(2).copied() {
                Some("start") => {
                    let name = words.get(3).copied();
                    let res = self.start_studio_recording(name)?;
                    Ok(Some(res))
                }
                Some("stop") => {
                    let res = self.stop_studio_recording()?;
                    Ok(Some(res))
                }
                Some("status") => {
                    let res = self.studio_recording_status();
                    Ok(Some(res))
                }
                _ => Ok(Some(json!({ "error": "unknown studio record subcommand" }))),
            },
            Some("replay") => match words.get(2).copied() {
                Some("open") => {
                    let path = words.get(3).ok_or("missing path for studio replay open")?;
                    let res = self.open_studio_replay(path)?;
                    Ok(Some(res))
                }
                Some("pause") => {
                    if let Some(r) = self.replay.driver.as_mut() {
                        r.set_paused(true);
                        Ok(Some(json!({ "ok": true, "paused": true })))
                    } else {
                        Ok(Some(json!({ "error": "no active replay" })))
                    }
                }
                Some("resume") => {
                    if let Some(r) = self.replay.driver.as_mut() {
                        r.set_paused(false);
                        Ok(Some(json!({ "ok": true, "paused": false })))
                    } else {
                        Ok(Some(json!({ "error": "no active replay" })))
                    }
                }
                Some("speed") => {
                    let spd_str = words.get(3).ok_or("missing speed value")?;
                    let spd: f64 = spd_str.parse().map_err(|_| "invalid speed")?;
                    if let Some(r) = self.replay.driver.as_mut() {
                        r.set_speed(spd);
                        Ok(Some(json!({ "ok": true, "speed": r.speed() })))
                    } else {
                        Ok(Some(json!({ "error": "no active replay" })))
                    }
                }
                Some("seek") => {
                    let ms_str = words.get(3).ok_or("missing seek timestamp")?;
                    let ms: u32 = ms_str.parse().map_err(|_| "invalid timestamp")?;
                    if self.replay.driver.is_none() {
                        return Ok(Some(json!({ "error": "no active replay" })));
                    }
                    // As chat, Studio and mods seek: the world rebuilt at the position, which the
                    // next frame reports exactly.
                    if let Err(error) = self.seek_studio_replay(ms) {
                        return Ok(Some(json!({ "error": error.to_string() })));
                    }
                    let pos = self.replay.driver.as_ref().map(|r| r.position_ms());
                    Ok(Some(json!({ "ok": true, "position_ms": pos })))
                }
                Some("status") => {
                    if let Some(r) = self.replay.driver.as_ref() {
                        Ok(Some(json!({
                            "ok": true,
                            "active": true,
                            "position_ms": r.position_ms(),
                            "duration_ms": r.duration_ms(),
                            "progress": r.progress(),
                            "speed": r.speed(),
                            "paused": r.is_paused(),
                            "finished": r.is_finished(),
                        })))
                    } else {
                        Ok(Some(json!({ "ok": true, "active": false })))
                    }
                }
                Some("close") => {
                    let closed = self.close_studio_replay();
                    Ok(Some(json!({ "ok": true, "closed": closed })))
                }
                _ => Ok(Some(json!({ "error": "unknown studio replay subcommand" }))),
            },
            Some("screen") if super::studio::EDITOR => self.studio_screen_control(words),
            _ => Ok(Some(json!({ "error": "unknown studio command" }))),
        }
    }

    pub(super) fn start_studio_recording(&mut self, name: Option<&str>) -> Result<Value> {
        let Some(r) = self.remote.as_ref() else {
            return Ok(json!({ "ok": false, "error": "no active world session" }));
        };
        if r.session.is_recording() {
            return Ok(json!({ "ok": false, "error": "recording already active" }));
        }
        let _ = std::fs::create_dir_all("recordings");
        let ts = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();
        let filename = match name {
            Some(n) if !n.is_empty() => format!("{}_{}.mcpr", n, ts),
            _ => format!("recording_{}.mcpr", ts),
        };
        let path = PathBuf::from("recordings").join(filename);
        let server_address = if r.session.address.is_empty() {
            "local".to_owned()
        } else {
            r.session.address.clone()
        };
        let is_singleplayer = server_address == "local";
        let meta = Metadata::new(
            server_address,
            is_singleplayer,
            horizon_core::release::VERSION.into(),
        );
        match horizon_client::studio::StudioRecorder::start(path.clone(), meta) {
            Ok(recorder) => {
                self.replay.recording_error_reported = false;
                r.session.start_recording(recorder, |recorder| {
                    self.dump_world_snapshot_to_recorder(recorder);
                });
                Ok(json!({ "ok": true, "path": path.display().to_string() }))
            }
            Err(e) => Ok(json!({ "ok": false, "error": e.to_string() })),
        }
    }

    pub(super) fn stop_studio_recording(&mut self) -> Result<Value> {
        let Some(r) = self.remote.as_ref() else {
            return Ok(json!({ "ok": false, "error": "no active world session" }));
        };
        match r.session.stop_recording() {
            Some(Ok(path)) => Ok(json!({ "ok": true, "path": path.display().to_string() })),
            Some(Err(e)) => Ok(json!({ "ok": false, "error": e.to_string() })),
            None => Ok(json!({ "ok": false, "error": "no active recording" })),
        }
    }

    pub(super) fn studio_recording_status(&self) -> Value {
        if let Some(r) = self.remote.as_ref()
            && let Some(status) = r.session.recorder_status()
        {
            json!({
                "ok": true,
                "active": status.active,
                "path": status.path.display().to_string(),
                "elapsed_ms": status.elapsed.as_millis(),
                "packet_count": status.packet_count,
                "bytes_written": status.bytes_written,
                "error": status.error,
            })
        } else {
            json!({ "ok": true, "active": false })
        }
    }

    pub(super) fn open_studio_replay(&mut self, path: &str) -> Result<Value> {
        self.drop_stale_replay();
        if let Err(error) = self.save_studio_camera_track() {
            return Ok(json!({ "ok": false, "error": error.to_string() }));
        }
        // A replay is never watched on top of a live session: the server's time, weather, blocks
        // and entities would mix with the recording's. Leave the session as Disconnect or Save
        // and Quit does, then open the replay standalone once the title is back.
        if self.replay.driver.is_none() && (self.remote.is_some() || self.integrated.is_some()) {
            // A recording that cannot be opened must leave the live session untouched.
            if let Err(error) = ReplayDriver::open(Path::new(path)) {
                log::error!("Failed to open studio replay '{path}': {error}");
                return Ok(json!({ "ok": false, "error": error.to_string() }));
            }
            self.displace_pending_replay(mod_api::ReplayError::WorldBusy);
            self.pending_entry = Some(PendingWorldEntry::StudioReplay(PathBuf::from(path)));
            if let Err(error) = self.close_world() {
                self.pending_entry = None;
                return Err(error);
            }
            return Ok(json!({ "ok": true, "pending": true }));
        }
        // The recordings browser can be opened while the title screen is
        // available but the slower world tables are still loading. Replay
        // setup builds the normal client registries, which require `rest`.
        if self.remote.is_none() && self.res.rest.get().is_none() {
            let pending = PendingWorldEntry::StudioReplay(PathBuf::from(path));
            if self.defer_world_entry(pending)? {
                return Ok(json!({ "ok": true, "pending": true }));
            }
            self.join_world_rest()?;
        }

        // Validate and index the archive before replacing the title menu or active world. A
        // temp-file, metadata, or protocol error must leave the current screen usable.
        // The archive is read through one handle, which also says which file it is.
        let opened = std::fs::File::open(path).and_then(|file| {
            let identity = super::native_replay::FileIdentity::of(&file).ok();
            Ok((ReplayDriver::from_file(file, Path::new(path))?, identity))
        });
        let (driver, identity) = match opened {
            Ok(opened) => opened,
            Err(error) => {
                log::error!("Failed to open studio replay '{path}': {error}");
                return Ok(json!({ "ok": false, "error": error.to_string() }));
            }
        };

        let origin = super::native_replay::ReplayOrigin {
            recording: self.recording_id(path),
            request: 0,
            archive: None,
            identity,
            replacing: None,
        };
        self.start_studio_replay(driver, origin, path)
    }

    /// Plays `driver`, a recording already read, from where the game is (the title, or a replay
    /// it replaces): its session, the first frame, then `replay_opened`. `origin` says who asked
    /// for it, `path` only names it in the log.
    pub(super) fn start_studio_replay(
        &mut self,
        driver: ReplayDriver,
        origin: super::native_replay::ReplayOrigin,
        path: &str,
    ) -> Result<Value> {
        let super::native_replay::ReplayOrigin {
            recording,
            request,
            archive,
            identity,
            ..
        } = origin;
        let mut driver = driver;
        // The first frame is set up from the opening snapshot before mods hear `replay_opened`,
        // whatever time the recording's Login was stamped with.
        let opening = match driver.take_opening() {
            Ok(opening) => opening,
            Err(error) => {
                log::error!("Failed to read studio replay '{path}': {error}");
                return Ok(json!({ "ok": false, "error": error.to_string() }));
            }
        };
        self.drop_stale_replay();
        let standalone = self.replay.driver.is_none() && self.remote.is_none();
        if self.remote.is_none()
            && let Err(error) = self.init_remote_for_replay()
        {
            log::error!("Failed to initialize studio replay session: {error}");
            return Ok(json!({ "ok": false, "error": error.to_string() }));
        }
        if self.replay.driver.is_none() {
            self.replay.connection = self.replay.connections;
        }

        let duration_ms = driver.duration_ms();
        let packets = driver.packet_count();
        self.studio_replay_opened(&driver);
        if self.replay.driver.is_none() {
            self.replay.previous_camera_type = Some(self.camera_type);
            self.replay.standalone = standalone;
        } else {
            // A replay opened over another: mods hear the old one close before the new one opens.
            self.replay.driver = None;
            self.replay.reported_ms = None;
            // The kept synthetic session isn't made anew, so what the old recording set outside
            // it goes here.
            self.ui.chat.clear();
            self.reset_replay_world_extras();
            let previous = std::mem::take(&mut self.replay.recording);
            self.native_replay_closed(&previous);
        }
        self.replay.recording = recording.clone();
        self.replay.archive = archive;
        self.replay.identity = identity;
        self.replay.generation += 1;
        self.replay.reported_ms = None;
        self.replay.seek_report = false;
        // The wheel's part step belongs to the session the replay leaves.
        self.mouse.scroll = Default::default();
        self.replay.driver = Some(Box::new(driver));
        self.camera_type = horizon_client::camera::CameraType::FirstPerson;
        self.title = None;
        self.screen = Screen::Playing;
        self.capture(false);
        self.dragging = false;
        // After the resets: a screen the snapshot opens (a workstation's) stays open.
        self.dispatch_studio_replay_packets(opening);
        // Apply timestamp-zero login, player pose, and chunk packets before the first rendered
        // frame so playback opens at the recorded player's viewpoint.
        let _ = self.advance_studio_replay();
        if !self.replay.playing() {
            // The recording's own packets ended its session (a disconnect) before it was set up.
            log::error!("Studio replay '{path}' ended while it opened");
            return Ok(json!({ "ok": false, "error": "the replay ended while it opened" }));
        }
        self.reset_studio_playback_ticks();
        log::info!("Studio: Opened replay '{path}' ({duration_ms} ms, {packets} packets)");
        // Mods hear of the clock's moves from here on, starting where this first frame left it.
        self.replay.reported_ms = self.replay.driver.as_ref().map(|r| r.position_ms());
        self.native_replay_opened(&recording, duration_ms, self.replay.standalone, request);
        Ok(json!({
            "ok": true,
            "duration_ms": duration_ms,
            "packet_count": packets,
        }))
    }
    pub(super) fn close_studio_replay(&mut self) -> bool {
        // A standalone replay owns a synthetic Remote connection. Treat that
        // connection as replay state even if the saved origin flag was lost;
        // sending it through leave_server() opens the Multiplayer screen.
        let standalone = self.replay.standalone
            || self
                .remote
                .as_ref()
                .is_some_and(|remote| remote.brand == "horizon-studio");
        if !self.end_studio_replay() {
            return false;
        }
        let closed = true;
        self.capture(false);
        if standalone {
            if let Some(mut remote) = self.remote.take() {
                remote.session.disconnect();
            }
            self.using = None;
            self.end_mod_use();
            if let Err(error) = self.finish_close_world() {
                log::error!("Failed to close standalone Studio replay cleanly: {error}");
            }
            self.reset_studio_hand_state();
            // A title-screen replay has no previous world perspective to
            // restore. Start the next world in first person with visible hands.
            self.camera_type = horizon_client::camera::CameraType::FirstPerson;
            if let Some(title) = self.title.as_mut() {
                title.mode = horizon_client::title::Mode::Main;
            }
        } else if self.integrated.is_none() && self.lan.is_none() {
            let _ = self.close_world();
            if let Some(title) = self.title.as_mut() {
                title.mode = horizon_client::title::Mode::Main;
            }
        }
        closed
    }
    /// The replay itself ends: its edits saved, its driver dropped, mods told `replay_closed`.
    /// The session it played on is left as it is. False when no replay was installed.
    pub(super) fn end_studio_replay(&mut self) -> bool {
        self.studio_replay_closing();
        if self.replay.driver.take().is_none() {
            return false;
        }
        // Mods heard of it only once `replay_opened` was told (a session the recording's own
        // packets end while it opens fails the open instead).
        let heard = self.replay.reported_ms.is_some();
        self.replay.standalone = false;
        if let Some(camera_type) = self.replay.previous_camera_type.take() {
            self.camera_type = camera_type;
        }
        self.replay.last_tick = None;
        self.studio_replay_closed();
        self.replay.reported_ms = None;
        self.replay.seek_report = false;
        // A part step of the wheel in the replay doesn't carry into the next session.
        self.mouse.scroll = Default::default();
        self.replay.archive = None;
        self.replay.identity = None;
        let recording = std::mem::take(&mut self.replay.recording);
        if heard {
            self.native_replay_closed(&recording);
        }
        self.replay.virtual_tick_ms = 0.0;
        true
    }

    /// A replay installed on a connection that has gone since (a different one is current, or
    /// none) is ended without touching the session there now.
    pub(super) fn drop_stale_replay(&mut self) {
        if self.replay.driver.is_some() && self.replay.connection != self.replay.connections {
            self.end_studio_replay();
        }
    }

    /// Dump current player pose, dimension login, center chunk, and resident loaded chunks
    /// into the recording stream at timestamp 0ms so replays accurately reconstruct the world.
    pub(super) fn dump_world_snapshot_to_recorder(
        &self,
        recorder: &horizon_client::studio::StudioRecorder,
    ) {
        let Some(r) = self.remote.as_ref() else {
            return;
        };
        let record_cb = |pkt: horizon_protocol::packets::play::Clientbound| {
            let encoded = pkt.encode();
            let mut reader = horizon_protocol::buf::Reader::new(&encoded);
            if let Ok(id) = reader.varint() {
                let payload = reader.rest();
                recorder.record_packet_at(0, id, payload);
            }
        };

        // 1. C::Login packet baseline
        let login = horizon_protocol::packets::play::Clientbound::Login(
            horizon_protocol::packets::play::Login {
                player_id: r.entity_id,
                hardcore: self.level.hardcore,
                levels: vec!["minecraft:overworld".into()],
                max_players: 20,
                chunk_radius: self.settings.render_distance,
                simulation_distance: self.settings.simulation_distance,
                reduced_debug_info: false,
                show_death_screen: true,
                limited_crafting: false,
                spawn: horizon_protocol::packets::play::SpawnInfo {
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
            },
        );
        record_cb(login);

        // 2. Center chunk and player position
        let (pos, yaw, pitch) = if let Some(p) = &self.player {
            (
                glam::DVec3::new(p.position.x, p.position.y, p.position.z),
                self.yaw,
                self.pitch,
            )
        } else {
            (glam::DVec3::ZERO, self.yaw, self.pitch)
        };
        let chunk_x = (pos.x.floor() as i32) >> 4;
        let chunk_z = (pos.z.floor() as i32) >> 4;

        let center_chunk = horizon_protocol::packets::play::Clientbound::SetChunkCacheCenter(
            horizon_protocol::packets::play::SetChunkCacheCenter {
                x: chunk_x,
                z: chunk_z,
            },
        );
        record_cb(center_chunk);

        let player_pos = horizon_protocol::packets::play::Clientbound::PlayerPosition(
            horizon_protocol::packets::play::PlayerPosition {
                teleport: 0,
                position: [pos.x, pos.y, pos.z],
                delta: [0.0, 0.0, 0.0],
                yaw,
                pitch,
                relatives: 0,
            },
        );
        record_cb(player_pos);

        // 3. Encode resident loaded chunks into LevelChunkWithLight packets
        let biome_names: Vec<String> = world_biomes!(self)
            .list
            .iter()
            .map(|b| b.name.clone())
            .collect();
        let registry: Vec<String> = r
            .registries
            .registries
            .get("minecraft:worldgen/biome")
            .map(|v| v.iter().map(|(n, _)| n.clone()).collect())
            .unwrap_or_else(|| biome_names.clone());

        let encoder = crate::lan::chunks::ChunkEncoder::with_signs(
            self.res.facts().states.iter().map(|s| {
                (
                    s.name.clone(),
                    s.fluid_level > 0 || s.waterlogged,
                    s.sign_form(),
                )
            }),
            &r.builtin,
            &biome_names,
            &registry,
        );

        let mut count = 0;
        for chunk in self.world.chunks.values() {
            let level_chunk = encoder.encode(chunk, &self.world.block_entities);
            record_cb(
                horizon_protocol::packets::play::Clientbound::LevelChunkWithLight(level_chunk),
            );
            count += 1;
        }
        log::info!(
            "Studio Recorder: Dumped initial world snapshot ({count} chunks) at timestamp 0ms"
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::net::RecorderStatus;

    #[test]
    fn recording_failure_is_notified_once_and_rearms_for_a_new_session() {
        let mut replay = Replay::default();
        let mut status = RecorderStatus {
            active: true,
            path: PathBuf::from("test.mcpr"),
            elapsed: Duration::ZERO,
            packet_count: 0,
            bytes_written: 0,
            error: None,
        };
        assert_eq!(replay.recording_error(Some(&status)), None);
        status.active = false;
        status.error = Some("recorder queue is full; capture stopped".into());
        assert_eq!(replay.recording_error(Some(&status)), status.error);
        assert_eq!(replay.recording_error(Some(&status)), None);
        assert_eq!(replay.recording_error(None), None);
        assert_eq!(replay.recording_error(Some(&status)), status.error);
        status.active = true;
        status.error = None;
        assert_eq!(replay.recording_error(Some(&status)), None);
        status.active = false;
        status.error = Some("recorder worker stopped unexpectedly".into());
        assert_eq!(replay.recording_error(Some(&status)), status.error);
        assert_eq!(replay.recording_error(Some(&status)), None);
    }
}
