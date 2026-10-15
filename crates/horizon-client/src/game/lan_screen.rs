//! WorldOptionsScreen: the in-world options with the multiplayer section
//! that opens the world to the local network (the 26.3 replacement of the
//! old "Open to LAN" screen): default and personal game mode, cheats,
//! difficulty, LAN on/off with its port, command access for guests and
//! forcing the game mode.
use super::{Game, Screen, lan::LanSettings};
use horizon_client::{
    hud::{HudBatch, rgba},
    text::Text,
    ui::{self, EditBox},
};
use winit::{event::KeyEvent, keyboard::PhysicalKey};

pub(super) struct WorldOptions {
    default_mode: i32,
    personal_mode: i32,
    cheats: bool,
    difficulty: u8,
    lan: bool,
    port: EditBox,
    /// The free port suggested while the box is empty.
    hint: u16,
    guest_commands: bool,
    force: bool,
    initial: Snapshot,
}
#[derive(Clone, PartialEq)]
struct Snapshot {
    default_mode: i32,
    personal_mode: i32,
    cheats: bool,
    difficulty: u8,
    lan: bool,
    port: u16,
    guest_commands: bool,
    force: bool,
}

const MODES: [&str; 4] = ["survival", "creative", "adventure", "spectator"];
const DIFFICULTIES: [&str; 4] = ["peaceful", "easy", "normal", "hard"];

#[derive(Clone, Copy, PartialEq)]
enum Button {
    DefaultMode,
    PersonalMode,
    Cheats,
    Difficulty,
    Lan,
    CommandAccess,
    Force,
    Apply,
    Cancel,
}

fn on_off(on: bool) -> Text {
    Text::translate(if on { "options.on" } else { "options.off" }, &[])
}
fn labelled(key: &str, value: Text) -> Text {
    Text::translate("options.generic_value", &[Text::translate(key, &[]), value])
}

impl WorldOptions {
    fn parsed_port(&self) -> Option<u16> {
        if self.port.value.is_empty() {
            return Some(self.hint);
        }
        self.port.value.parse::<u16>().ok().filter(|p| *p >= 1024)
    }
    fn snapshot(&self) -> Snapshot {
        Snapshot {
            default_mode: self.default_mode,
            personal_mode: self.personal_mode,
            cheats: self.cheats,
            difficulty: self.difficulty,
            lan: self.lan,
            port: if self.lan {
                self.parsed_port().unwrap_or(0)
            } else {
                0
            },
            guest_commands: self.guest_commands && self.lan && self.cheats,
            force: self.force,
        }
    }
    fn changed(&self) -> bool {
        let valid = !self.lan || self.parsed_port().is_some();
        valid && self.snapshot() != self.initial
    }
    fn buttons(w: f32) -> Vec<(Button, f32, f32, f32)> {
        let x = (w / 2.0 - 154.0).floor();
        let right = x + 158.0;
        vec![
            (Button::DefaultMode, x, 38.0, 308.0),
            (Button::PersonalMode, x, 62.0, 308.0),
            (Button::Cheats, x, 86.0, 150.0),
            (Button::Difficulty, right, 86.0, 150.0),
            (Button::Lan, x, 126.0, 150.0),
            (Button::CommandAccess, x, 150.0, 150.0),
            (Button::Force, right, 150.0, 150.0),
        ]
    }
    fn footer(w: f32, h: f32) -> [(Button, f32, f32, f32); 2] {
        let x = (w / 2.0 - 154.0).floor();
        [
            (Button::Apply, x, h - 28.0, 150.0),
            (Button::Cancel, x + 158.0, h - 28.0, 150.0),
        ]
    }
    fn label(&self, b: Button) -> Text {
        let mode = |m: i32| {
            Text::translate(
                &format!("selectWorld.gameMode.{}", MODES[m as usize & 3]),
                &[],
            )
        };
        match b {
            Button::DefaultMode => {
                labelled("options.worldOptions.game_mode", mode(self.default_mode))
            }
            Button::PersonalMode => labelled(
                "options.worldOptions.personal_game_mode",
                mode(self.personal_mode),
            ),
            Button::Cheats => labelled("selectWorld.allowCommands", on_off(self.cheats)),
            Button::Difficulty => labelled(
                "options.difficulty",
                Text::translate(
                    &format!(
                        "options.difficulty.{}",
                        DIFFICULTIES[usize::from(self.difficulty) & 3]
                    ),
                    &[],
                ),
            ),
            Button::Lan => labelled("menu.multiplayerOptions.lan", on_off(self.lan)),
            Button::CommandAccess => labelled(
                "options.worldOptions.guest.command_access",
                on_off(self.guest_commands && self.lan && self.cheats),
            ),
            Button::Force => labelled(
                "options.worldOptions.guest.force_game_mode",
                on_off(self.force || !self.lan),
            ),
            Button::Apply => Text::translate("menu.multiplayerOptions.applyChanges", &[]),
            Button::Cancel => Text::translate("gui.cancel", &[]),
        }
    }
    fn active(&self, b: Button) -> bool {
        match b {
            Button::CommandAccess => self.lan && self.cheats,
            Button::Force => self.lan && !(self.guest_commands && self.cheats),
            Button::Apply => self.changed(),
            _ => true,
        }
    }
}

impl Game<'_> {
    pub(super) fn open_world_options(&mut self) {
        let hint = horizon_client::lan::available_port();
        let lan = self.lan.is_some();
        let mut port = EditBox::new(0.0, 0.0, 150.0, 20.0);
        port.max_length = 5;
        let (guest_commands, force) = self.lan.as_ref().map_or((false, true), |h| {
            (h.settings.guest_commands, h.settings.force_game_mode)
        });
        if let Some(p) = self.lan_port() {
            port.set_value(&p.to_string());
        }
        let mut options = WorldOptions {
            default_mode: self.level.mode(),
            personal_mode: self.mode_id(),
            cheats: self.cheats,
            difficulty: self.ui.difficulty,
            lan,
            port,
            hint,
            guest_commands,
            force,
            initial: Snapshot {
                default_mode: 0,
                personal_mode: 0,
                cheats: false,
                difficulty: 0,
                lan: false,
                port: 0,
                guest_commands: false,
                force: false,
            },
        };
        options.initial = options.snapshot();
        self.world_options = Some(options);
        self.screen = Screen::WorldOptions;
    }

    pub(super) fn close_world_options(&mut self) {
        self.world_options = None;
        self.screen = Screen::Paused;
    }

    pub(super) fn draw_world_options(&mut self, batch: &mut HudBatch) {
        let Some(mut o) = self.world_options.take() else {
            return;
        };
        let atlas = &self.res.atlas;
        let (w, h) = (batch.width, batch.height);
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, true);
        let centered = |batch: &mut HudBatch, key: &str, y: f32| {
            let t = Text::translate(key, &[]);
            let tw = atlas.font.width_text(&t);
            batch.text_styled(atlas, &t, ((w - tw) / 2.0).floor(), y, u32::MAX, true);
        };
        centered(batch, "options.worldOptions.title", 12.0);
        centered(batch, "options.worldOptions.general.title", 26.0);
        centered(batch, "options.worldOptions.multiplayer.title", 114.0);
        for (b, x, y, bw) in WorldOptions::buttons(w)
            .into_iter()
            .chain(WorldOptions::footer(w, h))
        {
            let active = o.active(b);
            let hover = active && ui::hovered(self.cursor, x, y, bw, 20.0);
            ui::button(batch, atlas, x, y, bw, 20.0, &o.label(b), active, hover);
        }
        // The port box sits beside the LAN button.
        let x = (w / 2.0 - 154.0).floor() + 158.0;
        o.port.x = x;
        o.port.y = 126.0;
        o.port.focused = o.lan && o.port.focused;
        if o.lan {
            o.port.draw(batch, atlas, None);
            if o.port.value.is_empty() {
                let hint = Text::literal(o.hint.to_string());
                batch.text_styled(
                    atlas,
                    &hint,
                    x + 4.0,
                    132.0,
                    rgba(160, 160, 160, 255),
                    false,
                );
            }
            if o.parsed_port().is_none() {
                let t = Text::translate(
                    "lanServer.port.invalid",
                    &[Text::literal("1024"), Text::literal("65535")],
                );
                let first = t.clone();
                batch.text_styled(atlas, &first, x, 148.0, rgba(255, 85, 85, 255), true);
            }
        } else {
            ui::button(
                batch,
                atlas,
                x,
                126.0,
                150.0,
                20.0,
                &Text::translate("lanServer.port", &[]),
                false,
                false,
            );
        }
        self.world_options = Some(o);
    }

    pub(super) fn world_options_click(&mut self) {
        let Some(mut o) = self.world_options.take() else {
            return;
        };
        let size = self
            .renderer
            .as_ref()
            .map_or([1, 1], |r| [r.config.width, r.config.height]);
        let b = HudBatch::new(size);
        let (w, h) = (b.width, b.height);
        let port_x = (w / 2.0 - 154.0).floor() + 158.0;
        o.port.focused = o.lan && ui::hovered(self.cursor, port_x, 126.0, 150.0, 20.0);
        let hit = WorldOptions::buttons(w)
            .into_iter()
            .chain(WorldOptions::footer(w, h))
            .find(|(button, x, y, bw)| {
                o.active(*button) && ui::hovered(self.cursor, *x, *y, *bw, 20.0)
            })
            .map(|b| b.0);
        if hit.is_some() {
            self.audio.play_button_click();
        }
        match hit {
            Some(Button::DefaultMode) => o.default_mode = (o.default_mode + 1) % 4,
            Some(Button::PersonalMode) => o.personal_mode = (o.personal_mode + 1) % 4,
            Some(Button::Cheats) => o.cheats = !o.cheats,
            Some(Button::Difficulty) => o.difficulty = (o.difficulty + 1) % 4,
            Some(Button::Lan) => {
                o.lan = !o.lan;
                o.port.focused = false;
            }
            Some(Button::CommandAccess) => o.guest_commands = !o.guest_commands,
            Some(Button::Force) => o.force = !o.force,
            Some(Button::Cancel) => {
                self.world_options = Some(o);
                self.close_world_options();
                return;
            }
            Some(Button::Apply) => {
                let wanted = o.snapshot();
                let before = o.initial.clone();
                self.world_options = Some(o);
                self.apply_world_options(&before, &wanted);
                self.close_world_options();
                return;
            }
            None => {}
        }
        self.world_options = Some(o);
    }

    pub(super) fn world_options_key(&mut self, event: &KeyEvent) {
        if event.state != winit::event::ElementState::Pressed {
            return;
        }
        let PhysicalKey::Code(key) = event.physical_key else {
            return;
        };
        let ctrl = self.keys.contains(&winit::keyboard::KeyCode::ControlLeft);
        let shortcut = self.edit_shortcut(event);
        let Some(o) = self.world_options.as_mut() else {
            return;
        };
        if !o.port.focused {
            return;
        }
        if let Some(shortcut) = shortcut {
            o.port.shortcut(shortcut, &mut self.clipboard);
            return;
        }
        if !o.port.key(key, ctrl, false) && !ctrl {
            self.world_options_text(event.text.as_deref());
        }
    }
    /// Digits typed into the port field (the keyboard, or a controller's
    /// on-screen keyboard), and its backspace.
    pub(super) fn world_options_text(&mut self, text: Option<&str>) {
        let Some(o) = self.world_options.as_mut().filter(|o| o.port.focused) else {
            return;
        };
        match text {
            Some(text) => {
                let digits: String = text.chars().filter(char::is_ascii_digit).collect();
                if !digits.is_empty() {
                    o.port.insert_text(&digits);
                }
            }
            None => {
                o.port
                    .key(winit::keyboard::KeyCode::Backspace, false, false);
            }
        }
    }

    fn apply_world_options(&mut self, before: &Snapshot, wanted: &Snapshot) {
        if wanted.difficulty != before.difficulty {
            let name = DIFFICULTIES[usize::from(wanted.difficulty) & 3];
            self.run_command(&format!("difficulty {name}"));
        }
        if wanted.default_mode != before.default_mode {
            self.set_default_game_mode(wanted.default_mode);
        }
        if wanted.personal_mode != before.personal_mode {
            self.set_mode_id(wanted.personal_mode);
        }
        self.cheats = wanted.cheats;
        let settings_changed =
            wanted.guest_commands != before.guest_commands || wanted.force != before.force;
        if wanted.lan != before.lan || (wanted.lan && wanted.port != before.port) {
            self.lan_close(before.lan);
            if wanted.lan {
                let opened = self.lan_open(LanSettings {
                    port: wanted.port,
                    guest_commands: wanted.guest_commands,
                    force_game_mode: wanted.force,
                });
                if opened.is_err() {
                    self.system_message(Text::translate("commands.publish.failed", &[]));
                }
            }
        } else if settings_changed {
            self.lan_apply_settings(wanted.guest_commands, wanted.force);
        }
    }
}
