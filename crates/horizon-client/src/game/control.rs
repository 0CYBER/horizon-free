//! Commands from the control socket (`horizon-ctl`), run on the game thread.
use super::{Game, Screen};
use horizon_client::{Result, items::ItemStack};
use horizon_core::geometry::Vec3;
use serde_json::{Value, json};
use std::{
    sync::mpsc::Sender,
    time::{Duration, Instant},
};
use winit::{event::MouseButton, event_loop::ActiveEventLoop, keyboard::KeyCode};

const HELP: &[&str] = &[
    "status                         game, player, streaming and renderer state",
    "key KEY down|up|tap            press or release a key (w a s d space shift ctrl e q esc f3 1-9 arrows ...)",
    "release                        release every key and mouse button",
    "mouse left|right|middle down|up|click",
    "look YAW PITCH                 set the view direction in degrees",
    "turn DYAW DPITCH               rotate the view",
    "tp X Y Z                       teleport the player",
    "hotbar 1-9                     select a hotbar slot",
    "give ITEM [COUNT]              add items to the inventory",
    "render-distance N              chunks, 2..64",
    "time TICKS                     set the time of day",
    "resume                         leave the pause menu",
    "screenshot PATH                write the next frame as PNG",
    "stats [reset]                  frame-time summary since the last reset",
    "bench SECONDS                  reset stats, wait, then reply with the summary",
    "open WORLD [SEED] [creative]   open or create a world from the title screen",
    "save                           save the world",
    "quit                           save and exit",
];

impl Game<'_> {
    pub(super) fn poll_control(&mut self, event_loop: &ActiveEventLoop) {
        if let Some((deadline, reply)) = self.pending_bench.take() {
            if Instant::now() >= deadline {
                let _ = reply.send(self.frame_stats.summary().to_string());
            } else {
                self.pending_bench = Some((deadline, reply));
            }
        }
        while let Some(request) = self.control.as_ref().and_then(|c| c.try_recv()) {
            let reply = match self.control_command(&request.line, &request.reply, event_loop) {
                Ok(Some(value)) => value,
                // Deferred: the command keeps its own reply channel.
                Ok(None) => continue,
                Err(e) => json!({ "error": e.to_string() }),
            };
            let _ = request.reply.send(reply.to_string());
        }
    }

    fn control_command(
        &mut self,
        line: &str,
        reply: &Sender<String>,
        event_loop: &ActiveEventLoop,
    ) -> Result<Option<Value>> {
        let words: Vec<&str> = line.split_whitespace().collect();
        let arg = |i: usize| {
            words
                .get(i)
                .copied()
                .ok_or_else(|| format!("missing argument {i} for {}", words[0]))
        };
        let ok = || Ok(Some(json!({ "ok": true })));
        match words[0] {
            "help" => Ok(Some(json!({ "commands": HELP }))),
            "status" => Ok(Some(self.control_status())),
            "key" => {
                let key = key_code(arg(1)?).ok_or_else(|| format!("unknown key {}", words[1]))?;
                self.require_world()?;
                match arg(2)? {
                    "down" => self.control_key_down(key)?,
                    "up" => {
                        self.keys.remove(&key);
                    }
                    "tap" => {
                        self.control_key_down(key)?;
                        self.keys.remove(&key);
                    }
                    other => {
                        return Err(
                            format!("key action must be down, up or tap, not {other}").into()
                        );
                    }
                }
                ok()
            }
            "release" => {
                self.keys.clear();
                self.left = false;
                self.right = false;
                ok()
            }
            "mouse" => {
                let button = match arg(1)? {
                    "left" => MouseButton::Left,
                    "right" => MouseButton::Right,
                    "middle" => MouseButton::Middle,
                    other => return Err(format!("unknown mouse button {other}").into()),
                };
                self.require_world()?;
                if self.screen != Screen::Playing {
                    return Err(format!(
                        "mouse needs the playing screen, not {}",
                        self.screen_name()
                    )
                    .into());
                }
                match arg(2)? {
                    "down" => self.play_mouse(button, true)?,
                    "up" => self.play_mouse(button, false)?,
                    "click" => {
                        self.play_mouse(button, true)?;
                        self.play_mouse(button, false)?;
                    }
                    other => {
                        return Err(
                            format!("mouse action must be down, up or click, not {other}").into(),
                        );
                    }
                }
                ok()
            }
            "look" => {
                self.yaw = arg(1)?.parse()?;
                self.pitch = arg(2)?.parse::<f32>()?.clamp(-90.0, 90.0);
                ok()
            }
            "turn" => {
                self.yaw += arg(1)?.parse::<f32>()?;
                self.pitch = (self.pitch + arg(2)?.parse::<f32>()?).clamp(-90.0, 90.0);
                ok()
            }
            "tp" => {
                let to = Vec3::new(arg(1)?.parse()?, arg(2)?.parse()?, arg(3)?.parse()?);
                let player = self.player.as_mut().ok_or("no player yet")?;
                player.position = to;
                player.velocity = Vec3::new(0.0, 0.0, 0.0);
                player.fall_distance = 0.0;
                self.previous_position = to;
                ok()
            }
            "hotbar" => {
                let slot: usize = arg(1)?.parse()?;
                if !(1..=9).contains(&slot) {
                    return Err("hotbar slot must be 1..9".into());
                }
                self.selected = slot - 1;
                ok()
            }
            "give" => {
                self.require_world()?;
                let name = arg(1)?;
                let item = if name.contains(':') {
                    name.to_owned()
                } else {
                    format!("minecraft:{name}")
                };
                let info = self
                    .res
                    .items
                    .get(&item)
                    .ok_or_else(|| format!("unknown item {item}"))?;
                let count: u32 = words.get(2).map_or(Ok(1), |c| c.parse())?;
                let max = info.max_stack;
                let mut given = 0;
                // Stop once the inventory is full.
                while given < count {
                    let n = (count - given).min(max);
                    let rest = self
                        .give(ItemStack {
                            item: item.as_str().into(),
                            count: n,
                            damage: 0,
                        })
                        .map_or(0, |r| r.count);
                    given += n - rest;
                    if rest > 0 {
                        break;
                    }
                }
                Ok(Some(json!({ "ok": true, "given": given })))
            }
            "render-distance" => {
                let n: i32 = arg(1)?.parse()?;
                if !(2..=64).contains(&n) {
                    return Err("render distance must be 2..64".into());
                }
                self.settings.render_distance = n;
                self.apply_settings();
                ok()
            }
            "time" => {
                self.world.day_time = arg(1)?.parse()?;
                ok()
            }
            "resume" => {
                if self.screen == Screen::Paused {
                    self.screen = Screen::Playing;
                }
                ok()
            }
            "screenshot" => {
                let renderer = self.renderer.as_mut().ok_or("no renderer yet")?;
                if self.pending_screenshot.is_some() {
                    return Err("a screenshot is already pending".into());
                }
                renderer.screenshot = Some(arg(1)?.into());
                self.pending_screenshot = Some(reply.clone());
                Ok(None)
            }
            "stats" => {
                let summary = self.frame_stats.summary();
                if words.get(1) == Some(&"reset") {
                    self.frame_stats.reset();
                }
                Ok(Some(summary))
            }
            "bench" => {
                let seconds: f64 = arg(1)?.parse()?;
                if self.pending_bench.is_some() {
                    return Err("a benchmark is already running".into());
                }
                self.frame_stats.reset();
                self.pending_bench = Some((
                    Instant::now() + Duration::from_secs_f64(seconds),
                    reply.clone(),
                ));
                Ok(None)
            }
            "open" => {
                if self.title.is_none() {
                    return Err("a world is already open".into());
                }
                let folder = arg(1)?;
                let seed = words.get(2).map(|s| s.parse::<i64>()).transpose()?;
                let creative = words.get(3) == Some(&"creative");
                self.open_world(folder, Some(folder), seed, creative)?;
                ok()
            }
            "save" => {
                self.require_world()?;
                self.save_game()?;
                ok()
            }
            "quit" => {
                if self.title.is_none() {
                    self.save_game()?;
                }
                event_loop.exit();
                ok()
            }
            other => Err(format!("unknown command {other}; try help").into()),
        }
    }

    fn require_world(&self) -> Result<()> {
        if self.title.is_some() {
            return Err("no world open; use: open WORLD [SEED] [creative]".into());
        }
        Ok(())
    }

    fn control_key_down(&mut self, key: KeyCode) -> Result<()> {
        if self.keys.insert(key) {
            self.key_pressed(key)?;
        }
        Ok(())
    }

    fn screen_name(&self) -> &'static str {
        if self.title.is_some() {
            return "title";
        }
        match self.screen {
            Screen::Playing => "playing",
            Screen::Paused => "paused",
            Screen::Options => "options",
            Screen::Inventory => "inventory",
            Screen::Creative => "creative",
            Screen::Dead => "dead",
        }
    }

    fn control_status(&self) -> Value {
        let player = self.player.as_ref().map(|p| {
            json!({
                "position": [p.position.x, p.position.y, p.position.z],
                "velocity": [p.velocity.x, p.velocity.y, p.velocity.z],
                "on_ground": p.ground.on_ground,
                "health": p.health.health(),
                "food": p.health.food.food,
            })
        });
        let target = self.pick().map(|hit| {
            let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
            json!({ "pos": hit.pos, "block": &*self.res.facts.get(state).name })
        });
        let renderer = self.renderer.as_ref().map(|r| {
            json!({
                "gpu": r.info.name,
                "backend": format!("{:?}", r.info.backend),
                "size": [r.config.width, r.config.height],
                "present_mode": format!("{:?}", r.config.present_mode),
                "drawn_sections": r.drawn_sections,
                "uploaded_bytes": r.uploaded_bytes,
            })
        });
        json!({
            "screen": self.screen_name(),
            "dimension": self.dim.id(),
            "creative": self.creative,
            "player": player,
            "yaw": self.yaw,
            "pitch": self.pitch,
            "target": target,
            "selected": self.selected + 1,
            "held": self.held().map(|s| json!({ "item": &*s.item, "count": s.count })),
            "keys": self.keys.iter().map(|k| format!("{k:?}")).collect::<Vec<_>>(),
            "fps": self.fps,
            "frames": self.frames,
            "ticks": self.ticks,
            "day_time": self.world.day_time,
            "render_distance": self.streaming.radius,
            "chunks": self.world.chunks.len(),
            "pending_chunks": self.streaming.pending(),
            "generated_chunks": self.streaming.generated_total,
            "meshed_sections": self.streaming.meshed_total,
            "renderer": renderer,
            "uptime_seconds": self.started.elapsed().as_secs_f64(),
        })
    }
}

fn key_code(name: &str) -> Option<KeyCode> {
    let name = name.to_ascii_lowercase();
    let letters = [
        KeyCode::KeyA,
        KeyCode::KeyB,
        KeyCode::KeyC,
        KeyCode::KeyD,
        KeyCode::KeyE,
        KeyCode::KeyF,
        KeyCode::KeyG,
        KeyCode::KeyH,
        KeyCode::KeyI,
        KeyCode::KeyJ,
        KeyCode::KeyK,
        KeyCode::KeyL,
        KeyCode::KeyM,
        KeyCode::KeyN,
        KeyCode::KeyO,
        KeyCode::KeyP,
        KeyCode::KeyQ,
        KeyCode::KeyR,
        KeyCode::KeyS,
        KeyCode::KeyT,
        KeyCode::KeyU,
        KeyCode::KeyV,
        KeyCode::KeyW,
        KeyCode::KeyX,
        KeyCode::KeyY,
        KeyCode::KeyZ,
    ];
    let digits = [
        KeyCode::Digit0,
        KeyCode::Digit1,
        KeyCode::Digit2,
        KeyCode::Digit3,
        KeyCode::Digit4,
        KeyCode::Digit5,
        KeyCode::Digit6,
        KeyCode::Digit7,
        KeyCode::Digit8,
        KeyCode::Digit9,
    ];
    if let [c] = name.as_bytes() {
        return match c {
            b'a'..=b'z' => Some(letters[usize::from(c - b'a')]),
            b'0'..=b'9' => Some(digits[usize::from(c - b'0')]),
            _ => None,
        };
    }
    Some(match name.as_str() {
        "space" => KeyCode::Space,
        "shift" | "lshift" => KeyCode::ShiftLeft,
        "ctrl" | "control" | "lctrl" => KeyCode::ControlLeft,
        "esc" | "escape" => KeyCode::Escape,
        "tab" => KeyCode::Tab,
        "enter" | "return" => KeyCode::Enter,
        "f3" => KeyCode::F3,
        "left" => KeyCode::ArrowLeft,
        "right" => KeyCode::ArrowRight,
        "up" => KeyCode::ArrowUp,
        "down" => KeyCode::ArrowDown,
        _ => return None,
    })
}
