//! The anticheat on a dedicated server's hub (docs/anticheat.md): the checks run where the hub handles
//! what players send, and a flag is carried out here (send back, alert, kick). The rules and the
//! violation levels are `horizon_anticheat`'s. The hub holds none of this unless `server.toml` turns it
//! on, so a server without it does exactly what vanilla's does.
use super::hub::{HostEvent, Hub};
use horizon_anticheat::{Check, Config};
use horizon_world::nbt::{JavaString, Tag};
use std::{io::Write, time::Instant};

pub struct Anticheat {
    pub config: Config,
    /// `log`, opened at the first flag.
    log: Option<std::fs::File>,
    /// When the previous server tick started, for the timer's balance.
    last_tick: Option<Instant>,
    tick_rate: f32,
}

impl Anticheat {
    pub fn new(config: Config) -> Self {
        Self {
            config,
            log: None,
            last_tick: None,
            tick_rate: 20.0,
        }
    }

    fn write_log(&mut self, line: &str) {
        if self.config.log.is_empty() {
            return;
        }
        if self.log.is_none() {
            let path = std::path::Path::new(&self.config.log);
            if let Some(dir) = path.parent() {
                let _ = std::fs::create_dir_all(dir);
            }
            self.log = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .ok();
        }
        if let Some(file) = &mut self.log {
            let time = chrono::Local::now().format("%Y-%m-%d %H:%M:%S");
            let _ = writeln!(file, "[{time}] {line}");
        }
    }
}

fn text(s: &str, color: &str) -> Tag {
    Tag::compound([
        ("text", Tag::String(JavaString::from(s))),
        ("color", Tag::String(JavaString::from(color))),
    ])
}

impl Hub {
    /// Turn the anticheat on with these settings, or off (`config.enabled` false).
    pub fn set_anticheat(&mut self, config: Config) {
        self.anticheat = config.enabled.then(|| Box::new(Anticheat::new(config)));
    }

    /// The start of a server tick: the time since the previous one fills each player's timer balance.
    pub(super) fn anticheat_tick(&mut self, tick_rate: f32) {
        let Some(ac) = self.anticheat.as_deref_mut() else {
            return;
        };
        let now = Instant::now();
        let elapsed = ac.last_tick.replace(now).map(|t| now - t);
        if !ac.config.runs(Check::Timer) {
            return;
        }
        // A new tick rate reaches the client a little later: both start again.
        let restart = ac.tick_rate != tick_rate;
        ac.tick_rate = tick_rate;
        let ms = elapsed.map_or(0.0, |e| e.as_secs_f64() * 1000.0);
        for g in &mut self.guests {
            if restart {
                g.anticheat.timer.reset();
            }
            g.anticheat.timer.elapse(ms, &ac.config.timer);
        }
    }

    /// ServerboundClientTickEndPacket: one client tick spent.
    pub(super) fn anticheat_tick_end(&mut self, i: usize, events: &mut Vec<HostEvent>) {
        let Some(ac) = self.anticheat.as_deref() else {
            return;
        };
        if !ac.config.runs(Check::Timer) {
            return;
        }
        let (rate, config) = (ac.tick_rate, &ac.config.timer);
        if let Some(ahead) = self.guests[i].anticheat.timer.tick_end(rate, config) {
            self.flag(
                i,
                Check::Timer,
                1.0,
                &format!("{ahead:.0} ms ahead"),
                events,
            );
        }
    }

    /// `check` failed for guest `i`: raise its level, log it, and act on it. Returns whether the move or
    /// action is cancelled (the player is sent back, or kicked).
    pub(super) fn flag(
        &mut self,
        i: usize,
        check: Check,
        weight: f32,
        detail: &str,
        events: &mut Vec<HostEvent>,
    ) -> bool {
        let ticks = self.ticks;
        let Some(ac) = self.anticheat.as_deref_mut() else {
            return false;
        };
        let g = &mut self.guests[i];
        let v = g.anticheat.flag(&ac.config, check, weight, ticks);
        let action = if v.kick {
            ", kicked"
        } else if v.setback {
            ", sent back"
        } else {
            ""
        };
        let line = format!(
            "{} failed {} (VL {:.1}): {detail}{action}",
            g.name,
            check.name(),
            v.vl
        );
        ac.write_log(&line);
        if v.alert {
            events.push(HostEvent::Warn(format!("[Anticheat] {line}")));
            if ac.config.alerts {
                let level = ac.config.alert_level;
                let content = Tag::compound([
                    ("text", Tag::String(JavaString::from("[Anticheat] "))),
                    ("color", Tag::String(JavaString::from("red"))),
                    ("extra", Tag::List(vec![text(&line, "gray")])),
                ]);
                let packet = horizon_protocol::packets::play::Clientbound::SystemChat(
                    horizon_protocol::packets::play::SystemChat {
                        content,
                        overlay: false,
                    },
                );
                for op in self
                    .guests
                    .iter()
                    .filter(|o| o.placed && o.permission >= level)
                {
                    op.send(&packet);
                }
            }
        }
        if v.kick {
            let conn = self.guests[i].conn;
            self.kick(conn, text("Kicked by the server's anticheat", "white"));
        } else if v.setback && self.guests[i].awaiting_teleport.is_none() {
            self.resend_teleport(i, None);
        }
        v.setback || v.kick
    }
}
