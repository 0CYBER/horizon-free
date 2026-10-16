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
    hardcore: bool,
    initial: Snapshot,
    /// ScrollableLayout's scroll amount.
    scroll: f32,
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

/// HeaderAndFooterLayout's header and footer heights.
const HEADER: f32 = 33.0;
const FOOTER: f32 = 33.0;
/// The scrolled content: both grids, each padded by 8 and 8 apart.
const CONTENT: f32 = 218.0;
/// The category headings' tops (FocusableTextWidget, padding 4).
const GENERAL_TOP: f32 = 8.0;
const MULTIPLAYER_TOP: f32 = 145.0;
/// The port box's top beside the LAN button.
const PORT_TOP: f32 = 166.0;

#[derive(Clone, Copy, Debug, PartialEq)]
enum Button {
    DefaultMode,
    PersonalMode,
    Cheats,
    Difficulty,
    /// LockIconButton: Horizon can't lock a world's difficulty yet.
    Lock,
    /// InWorldGameRulesScreen and RestrictionsScreen are not in Horizon yet.
    GameRules,
    Restrictions,
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
    /// The scrolled content's top: under the header, less the scroll.
    fn top(&self) -> f32 {
        HEADER - self.scroll.trunc()
    }
    /// The grids' left edge: the content (308 wide, padded by 8) centred
    /// in a ScrollableLayout reserving 10 pixels for the scrollbar each side.
    fn left(w: f32) -> f32 {
        (w / 2.0).floor() - 154.0
    }
    /// The scrolled widgets: each grid's rows 4 apart, columns 8 apart.
    fn buttons(w: f32, top: f32) -> Vec<(Button, f32, f32, f32)> {
        let x = Self::left(w);
        let right = x + 158.0;
        vec![
            (Button::DefaultMode, x, top + 29.0, 308.0),
            (Button::PersonalMode, x, top + 53.0, 308.0),
            (Button::Cheats, x, top + 77.0, 150.0),
            // DifficultyButtons: the cycle button gives the lock its 20 pixels.
            (Button::Difficulty, right, top + 77.0, 130.0),
            (Button::Lock, right + 130.0, top + 77.0, 20.0),
            (Button::GameRules, x, top + 101.0, 150.0),
            (Button::Restrictions, right, top + 101.0, 150.0),
            (Button::Lan, x, top + 166.0, 150.0),
            (Button::CommandAccess, x, top + 190.0, 150.0),
            (Button::Force, right, top + 190.0, 150.0),
        ]
    }
    /// The footer's two buttons, centred in its 33 pixels (FrameLayout
    /// truncates the 6.5 pixel offset).
    fn footer(w: f32, h: f32) -> [(Button, f32, f32, f32); 2] {
        let x = Self::left(w);
        [
            (Button::Apply, x, h - FOOTER + 6.0, 150.0),
            (Button::Cancel, x + 158.0, h - FOOTER + 6.0, 150.0),
        ]
    }
    /// The widgets under the pointer's row: scrolled ones only inside the
    /// scroll area.
    fn hit(&self, (x, y): (f32, f32), (w, h): (f32, f32)) -> Option<Button> {
        let inside = y >= HEADER && y < h - FOOTER;
        Self::buttons(w, self.top())
            .into_iter()
            .filter(|_| inside)
            .chain(Self::footer(w, h))
            .find(|(b, bx, by, bw)| self.active(*b) && ui::hovered((x, y), *bx, *by, *bw, 20.0))
            .map(|b| b.0)
    }
    fn max_scroll(h: f32) -> f32 {
        (CONTENT - (h - HEADER - FOOTER)).max(0.0)
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
            Button::Lock => Text::translate("narrator.button.difficulty_lock", &[]),
            Button::GameRules => Text::translate("editGamerule.inGame.button", &[]),
            Button::Restrictions => Text::translate("restrictions_screen.button", &[]),
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
                on_off(self.force || !self.lan || self.hardcore),
            ),
            Button::Apply => Text::translate("menu.multiplayerOptions.applyChanges", &[]),
            Button::Cancel => Text::translate("gui.cancel", &[]),
        }
    }
    /// WorldOptionsScreen.updateButton and the update*Button methods.
    fn active(&self, b: Button) -> bool {
        match b {
            Button::DefaultMode | Button::PersonalMode => self.cheats && !self.hardcore,
            // An owner's commands come with Allow Commands.
            Button::Cheats => !self.hardcore || self.initial.cheats,
            Button::Difficulty => !self.hardcore,
            Button::Lock | Button::GameRules | Button::Restrictions => false,
            Button::CommandAccess => self.lan && self.cheats,
            Button::Force => self.lan && !(self.guest_commands && self.cheats) && !self.hardcore,
            Button::Apply => self.changed(),
            Button::Lan | Button::Cancel => true,
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
            cheats: self.allow_commands(),
            difficulty: self.ui.difficulty,
            lan,
            port,
            hint,
            guest_commands,
            force,
            hardcore: self.level.hardcore,
            scroll: 0.0,
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
        o.scroll = o.scroll.clamp(0.0, WorldOptions::max_scroll(h));
        // extractMenuBackground: the in-world background, the list
        // background under the content, then the separators over it.
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, true);
        ui::list_background(batch, atlas, HEADER, h - FOOTER, true);
        let title = Text::translate("options.worldOptions.title", &[]);
        let tw = atlas.font.width_text(&title);
        batch.text_styled(
            atlas,
            &title,
            ((w - tw) / 2.0).floor(),
            12.0,
            u32::MAX,
            true,
        );
        let top = o.top();
        let first = batch.vertices.len();
        let x = WorldOptions::left(w);
        for (key, y) in [
            ("options.worldOptions.general.title", GENERAL_TOP),
            ("options.worldOptions.multiplayer.title", MULTIPLAYER_TOP),
        ] {
            // A FocusableTextWidget padded by 4, centred over both columns.
            let t = Text::translate(key, &[]).underlined().bold();
            let tw = atlas.font.width_text(&t);
            let tx = x + ((308.0 - (tw + 8.0)) / 2.0).trunc() + 4.0;
            batch.text_styled(atlas, &t, tx, top + y + 4.0, u32::MAX, true);
        }
        let inside = self.cursor.1 >= HEADER && self.cursor.1 < h - FOOTER;
        for (b, bx, by, bw) in WorldOptions::buttons(w, top) {
            let active = o.active(b);
            let hover = active && inside && ui::hovered(self.cursor, bx, by, bw, 20.0);
            if b == Button::Lock {
                let lock = if o.hardcore { "locked" } else { "unlocked" };
                let state = match (active, hover) {
                    (false, _) => "_disabled",
                    (true, true) => "_highlighted",
                    (true, false) => "",
                };
                let sprite = format!("gui/sprites/widget/{lock}_button{state}");
                batch.sprite_scaled(atlas, &sprite, bx, by, 20.0, 20.0, u32::MAX);
                continue;
            }
            ui::button(batch, atlas, bx, by, bw, 20.0, &o.label(b), active, hover);
        }
        // The port box beside the LAN button: editable only with LAN on,
        // its hint the port to use (setPortError colours the text).
        o.port.x = x + 158.0;
        o.port.y = top + PORT_TOP;
        o.port.focused = o.lan && o.port.focused;
        o.port.text_color = if !o.lan {
            rgba(0x70, 0x70, 0x70, 255)
        } else if o.parsed_port().is_some() {
            rgba(0xE0, 0xE0, 0xE0, 255)
        } else {
            rgba(0xDF, 0x50, 0x50, 255)
        };
        o.port.draw(batch, atlas, None);
        if o.port.value.is_empty() && !o.port.focused {
            let hint = if o.lan {
                Text::literal(o.hint.to_string())
            } else {
                Text::translate("lanServer.port", &[])
            };
            // EditBox.DEFAULT_HINT_STYLE: dark gray.
            batch.text_styled(
                atlas,
                &hint,
                o.port.text_x(),
                o.port.text_y(),
                rgba(0x55, 0x55, 0x55, 255),
                true,
            );
        }
        batch.clip_since(first, 0.0, HEADER, w, h - FOOTER);
        ui::scrollbar(
            batch,
            atlas,
            (w / 2.0).floor() + 166.0,
            HEADER,
            h - FOOTER,
            CONTENT,
            o.scroll.trunc(),
        );
        ui::separators(batch, atlas, HEADER, h - FOOTER, true);
        for (b, bx, by, bw) in WorldOptions::footer(w, h) {
            let active = o.active(b);
            let hover = active && ui::hovered(self.cursor, bx, by, bw, 20.0);
            ui::button(batch, atlas, bx, by, bw, 20.0, &o.label(b), active, hover);
        }
        self.world_options = Some(o);
    }

    /// The mouse wheel over the screen (ScrollableLayout, 10 pixels a step).
    pub(super) fn world_options_scroll(&mut self, delta: f32) {
        let h = self.gui_size().1;
        if let Some(o) = self.world_options.as_mut() {
            o.scroll = (o.scroll - delta * 10.0).clamp(0.0, WorldOptions::max_scroll(h));
        }
    }

    pub(super) fn world_options_click(&mut self) {
        let Some(mut o) = self.world_options.take() else {
            return;
        };
        let (w, h) = self.gui_size();
        let inside = self.cursor.1 >= HEADER && self.cursor.1 < h - FOOTER;
        o.port.focused = o.lan
            && inside
            && ui::hovered(
                self.cursor,
                WorldOptions::left(w) + 158.0,
                o.top() + PORT_TOP,
                150.0,
                20.0,
            );
        let hit = o.hit(self.cursor, (w, h));
        if hit.is_some() {
            self.audio.play_button_click();
        }
        match hit {
            Some(Button::DefaultMode) => o.default_mode = (o.default_mode + 1) % 4,
            Some(Button::PersonalMode) => o.personal_mode = (o.personal_mode + 1) % 4,
            Some(Button::Cheats) => {
                o.cheats = !o.cheats;
                // updatePermissionDependentButtons: without commands both
                // game modes return to the world's default.
                if !o.cheats {
                    o.default_mode = o.initial.default_mode;
                    o.personal_mode = o.default_mode;
                }
            }
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
            Some(Button::Lock | Button::GameRules | Button::Restrictions) | None => {}
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
            self.as_console(|g| g.run_command(&format!("difficulty {name}")));
        }
        if wanted.default_mode != before.default_mode {
            self.set_default_game_mode(wanted.default_mode);
        }
        if wanted.personal_mode != before.personal_mode {
            self.set_mode_id(wanted.personal_mode);
        }
        if wanted.cheats != before.cheats {
            self.set_allow_commands(wanted.cheats);
        }
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

#[cfg(test)]
mod tests {
    use super::*;

    fn options(cheats: bool, hardcore: bool) -> WorldOptions {
        let mut o = WorldOptions {
            default_mode: 1,
            personal_mode: 1,
            cheats,
            difficulty: 2,
            lan: false,
            port: EditBox::new(0.0, 0.0, 150.0, 20.0),
            hint: 25565,
            guest_commands: false,
            force: true,
            hardcore,
            scroll: 0.0,
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
        o.initial = o.snapshot();
        o
    }

    #[test]
    fn layout_follows_the_header_footer_and_grids() {
        let at = |b: Button| {
            let o = options(true, false);
            WorldOptions::buttons(427.0, o.top())
                .into_iter()
                .chain(WorldOptions::footer(427.0, 240.0))
                .find(|e| e.0 == b)
                .map(|(_, x, y, w)| (x, y, w))
                .unwrap()
        };
        // Header 33, content padding 8, heading 17, rows 4 apart.
        assert_eq!(at(Button::DefaultMode), (59.0, 62.0, 308.0));
        assert_eq!(at(Button::Cheats), (59.0, 110.0, 150.0));
        assert_eq!(at(Button::Difficulty), (217.0, 110.0, 130.0));
        assert_eq!(at(Button::Lock), (347.0, 110.0, 20.0));
        assert_eq!(at(Button::Restrictions), (217.0, 134.0, 150.0));
        // The Multiplayer grid 24 below: padding, spacing, padding.
        assert_eq!(at(Button::Lan), (59.0, 199.0, 150.0));
        assert_eq!(at(Button::Force), (217.0, 223.0, 150.0));
        assert_eq!(at(Button::Apply), (59.0, 213.0, 150.0));
        // 218 pixels of content over 174 between the separators.
        assert_eq!(WorldOptions::max_scroll(240.0), 44.0);
        assert_eq!(WorldOptions::max_scroll(400.0), 0.0);
    }

    #[test]
    fn buttons_follow_commands_hardcore_and_lan() {
        let o = options(true, false);
        assert!(o.active(Button::DefaultMode) && o.active(Button::Difficulty));
        assert!(!o.active(Button::CommandAccess) && !o.active(Button::Force));
        assert!(!o.active(Button::Apply));
        let o = options(false, false);
        assert!(!o.active(Button::DefaultMode) && !o.active(Button::PersonalMode));
        let o = options(false, true);
        assert!(!o.active(Button::Cheats) && !o.active(Button::Difficulty));
        assert!(!o.active(Button::Force));
        // Scrolled widgets only take clicks between the separators.
        let mut o = options(true, false);
        o.scroll = 44.0;
        assert_eq!(o.hit((100.0, 30.0), (427.0, 240.0)), None);
        assert_eq!(o.hit((100.0, 70.0), (427.0, 240.0)), Some(Button::Cheats));
    }
}
