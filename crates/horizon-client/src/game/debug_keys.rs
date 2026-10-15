//! The debug keys in the game (26.3 `KeyboardHandler.keyPress`,
//! `handleDebugKeys` and `tick`): F3 combinations, F3+Escape, the held
//! debug-crash keys, and the debug renderers they switch on.
use super::{Game, Screen};
use horizon_client::{
    Result,
    command::args::GameMode,
    debug_keys::{CrashNotice, DebugAction, DebugHost, KeyAction, OpenScreen},
    gizmos::Gizmos,
    keybinds,
    text::Text,
};
use horizon_core::geometry::Axis;
use winit::keyboard::KeyCode;

/// `ChatFormatting.YELLOW` and `RED`.
const YELLOW: u32 = 0xFF_FF55;
const RED: u32 = 0xFF_5555;

/// The game as `KeyboardHandler` sees it while it runs one key event.
struct Host<'g, 'a> {
    game: &'g mut Game<'a>,
}

impl DebugHost for Host<'_, '_> {
    fn screen(&self) -> OpenScreen {
        self.game.open_screen()
    }
    fn toggle_overlay(&mut self) {
        self.game.debug = !self.game.debug;
    }
    fn pause_game(&mut self, without_menu: bool) {
        // Gui.setPauseScreen: PauseScreen(false) only when the game really
        // pauses (single player, not open to the network). Escape alone opens
        // the menu as it always has (key_pressed).
        let g = &mut *self.game;
        if without_menu && g.screen == Screen::Playing {
            g.pause_menu = !(g.remote.is_none() && g.lan.is_none());
            g.screen = Screen::Paused;
            g.capture(false);
            g.mining = None;
        }
    }
    fn debug_action(&mut self, action: DebugAction, shift: bool) -> bool {
        self.game.run_debug_action(action, shift)
    }
}

impl Game<'_> {
    fn open_screen(&self) -> OpenScreen {
        match self.screen {
            Screen::GameModeSwitcher => OpenScreen::GameModeSwitcher,
            Screen::Playing if self.book.is_none() && self.sign_edit.is_none() => OpenScreen::None,
            Screen::Paused if !self.pause_menu => OpenScreen::PauseWithoutMenu,
            _ => OpenScreen::Other,
        }
    }
    /// A physical key event (already in `physical_keys`) before the game's
    /// own handling. Returns true when it is used up (a debug combination,
    /// F3 itself, F3+Escape).
    pub(super) fn debug_key(&mut self, key: KeyCode, action: KeyAction) -> bool {
        let Some(name) = keybinds::key_name(key) else {
            return false;
        };
        let pressed = action != KeyAction::Release;
        let switcher_open = self.screen == Screen::GameModeSwitcher;
        if switcher_open && pressed {
            if name == self.settings.keys.get("key.debug.switchGameMode") {
                if let Some(switcher) = &mut self.gamemode_switcher {
                    switcher.cycle();
                }
            } else if key == KeyCode::Escape {
                self.close_gamemode_switcher(false);
            } else if matches!(key, KeyCode::F2 | KeyCode::F11) {
                return false;
            }
            return true;
        }
        // Screens with a text box take every key press first
        // (EditBox.keyPressed); releases still reach KeyboardHandler.
        let typing = matches!(
            self.screen,
            Screen::Chat | Screen::Options | Screen::WorldOptions
        ) || self.sign_edit.is_some()
            || self.book.is_some()
            || (self.screen == Screen::Creative && self.creative_screen.tab().search)
            || self.recipe_search_focused()
            || self.station_typing()
            || self.remote_chat_open();
        // Container screens use Escape, the inventory key and the hotbar and
        // drop keys first; any other screen closes on Escape.
        let screen_key = match self.screen {
            Screen::Inventory | Screen::Creative => matches!(
                key,
                KeyCode::Escape
                    | KeyCode::KeyE
                    | KeyCode::KeyF
                    | KeyCode::KeyQ
                    | KeyCode::Digit1
                    | KeyCode::Digit2
                    | KeyCode::Digit3
                    | KeyCode::Digit4
                    | KeyCode::Digit5
                    | KeyCode::Digit6
                    | KeyCode::Digit7
                    | KeyCode::Digit8
                    | KeyCode::Digit9
            ),
            Screen::Playing => false,
            _ => key == KeyCode::Escape,
        };
        if pressed && (typing || screen_key) {
            return false;
        }
        let crash_down = keybinds::key_code(self.settings.keys.get("key.debug.crash"))
            .is_some_and(|k| self.physical_keys.contains(&k));
        let shift = self.physical_keys.contains(&KeyCode::ShiftLeft)
            || self.physical_keys.contains(&KeyCode::ShiftRight);
        let now = self.debug_millis();
        let binds = self.settings.keys.clone();
        let mut keys = std::mem::take(&mut self.debug_keys);
        let modifier_was_down = keys.modifier_down;
        let to_game = keys.key_press(
            &mut Host { game: self },
            &binds,
            name,
            action,
            shift,
            crash_down,
            now,
        );
        self.debug_keys = keys;
        if switcher_open {
            if name == binds.get("key.debug.modifier") {
                self.close_gamemode_switcher(true);
                return true;
            }
            return false;
        }
        if !pressed {
            return false;
        }
        // The debug modifier does nothing else.
        if name == binds.get("key.debug.modifier") || name == binds.get("key.debug.overlay") {
            return true;
        }
        if self.open_screen() == OpenScreen::None {
            !to_game
        } else {
            modifier_was_down
        }
    }
    /// `Util.getMillis` (positive: KeyboardHandler treats 0 as unset).
    pub(super) fn debug_millis(&self) -> i64 {
        self.started.elapsed().as_millis() as i64 + 1_000_000
    }
    /// `KeyboardHandler.tick`: the debug crash countdown.
    pub(super) fn tick_debug_keys(&mut self) -> Result<()> {
        let now = self.debug_millis();
        match self.debug_keys.tick(now) {
            None => {}
            Some(CrashNotice::Message) => {
                let args = [
                    self.key_text("key.debug.modifier"),
                    self.key_text("key.debug.crash"),
                ];
                self.debug_feedback(Text::translate("debug.crash.message", &args));
            }
            Some(CrashNotice::Warning(seconds)) => self.debug_warning(Text::translate(
                "debug.crash.warning",
                &[Text::literal(seconds.to_string())],
            )),
            Some(CrashNotice::Crash) => {
                // Minecraft.emergencySaveAndCrash: the world is saved, then the
                // game stops; main writes the crash report.
                let crash = horizon_client::crash::DebugCrash::new();
                let _ = self.save_game();
                return Err(crash.into());
            }
        }
        Ok(())
    }
    /// The network charts' samples each tick: `PingDebugMonitor.tick` while
    /// they show, `BandwidthDebugMonitor.tick` always.
    pub(super) fn tick_debug_charts(&mut self) {
        let shown = self.debug && self.debug_charts.network;
        let Some(r) = &self.remote else {
            // The integrated server answers within the tick.
            if shown {
                self.debug_charts.ping.log(0);
            }
            return;
        };
        let received = r
            .session
            .counters
            .lock()
            .unwrap()
            .as_ref()
            .map(|c| c.bytes_in.load(std::sync::atomic::Ordering::Relaxed));
        if let Some(total) = received {
            self.debug_charts.log_bandwidth(total);
        }
        if shown {
            use horizon_protocol::packets::{play::Serverbound, status::Ping};
            let time = self.debug_millis();
            self.send(Serverbound::PingRequest(Ping { time }));
        }
    }
    /// The overlay's chart lines (`DebugScreenOverlay`'s "Debug charts").
    pub(super) fn debug_chart_lines(&self) -> [String; 3] {
        let keys = &self.settings.keys;
        let modifier = keys.get("key.debug.modifier");
        let key = |mapping: &str| {
            let prefix = if modifier == "key.keyboard.unknown" {
                String::new()
            } else {
                format!("{}+", keybinds::display_name(modifier).plain())
            };
            format!(
                "[{prefix}{}]",
                keybinds::display_name(keys.get(mapping)).plain()
            )
        };
        let chart = |mapping: &str, name: &str, on: bool| {
            format!(
                "{} {name} {}",
                key(mapping),
                if on { "visible" } else { "hidden" }
            )
        };
        let remote = self.remote.is_some();
        let c = &self.debug_charts;
        [
            format!(
                "Debug charts: {}; {};",
                chart("key.debug.profilingChart", "Profiler", false),
                chart(
                    "key.debug.fpsCharts",
                    if remote { "fps" } else { "fps + tps" },
                    c.fps
                ),
            ),
            format!(
                "{}; {}",
                chart(
                    "key.debug.networkCharts",
                    if remote { "Bandwidth + Ping" } else { "Ping" },
                    c.network
                ),
                chart("key.debug.lightmapTexture", "Lightmap", c.lightmap),
            ),
            format!("To edit: press {}", key("key.debug.debugOptions")),
        ]
    }
    /// `KeyMapping.getTranslatedKeyMessage`.
    fn key_text(&self, mapping: &str) -> Text {
        keybinds::display_name(self.settings.keys.get(mapping))
    }
    /// `KeyboardHandler.decorateDebugComponent`: `[Debug]:` in bold, a space,
    /// the message, shown with `Minecraft.showDebugChat`.
    fn debug_decorated(&mut self, color: u32, message: Text) {
        let prefix = Text::translate("debug.prefix", &[])
            .with_color(color)
            .bold();
        self.system_message(
            Text::empty()
                .append(prefix)
                .append(Text::literal(" "))
                .append(message),
        );
    }
    fn debug_feedback(&mut self, message: Text) {
        self.debug_decorated(YELLOW, message);
    }
    fn debug_warning(&mut self, message: Text) {
        self.debug_decorated(RED, message);
    }
    fn debug_translated(&mut self, key: &str) {
        self.debug_feedback(Text::translate(key, &[]));
    }
    /// `LocalPlayer.isReducedDebugInfo`: the server's rule or the option.
    fn reduced_debug_info(&self) -> bool {
        self.settings.reduced_debug_info || self.ui.rules.bool("reduced_debug_info")
    }
    /// `Permissions.COMMANDS_GAMEMASTER` (`GameModeCommand.PERMISSION_CHECK`).
    pub(super) fn gamemaster(&self) -> bool {
        self.ui.registries.permission >= 2
    }
    fn toggle_option(&mut self, key: &str) -> bool {
        let on = !self.settings.file.bool(key);
        self.settings
            .file
            .set(key, horizon_client::options_file::V::Bool(on));
        self.settings.save(&Self::options_path());
        on
    }
    /// One matching mapping of `handleDebugKeys`; whether it counts.
    fn run_debug_action(&mut self, action: DebugAction, shift: bool) -> bool {
        let player = self.player.is_some() && !self.reduced_debug_info();
        match action {
            DebugAction::ReloadChunks => {
                self.all_sections_changed();
                self.debug_translated("debug.reload_chunks.message");
                true
            }
            DebugAction::ShowHitboxes if player => {
                self.show_hitboxes = !self.show_hitboxes;
                self.debug_translated(if self.show_hitboxes {
                    "debug.show_hitboxes.on"
                } else {
                    "debug.show_hitboxes.off"
                });
                true
            }
            DebugAction::ClearChat => {
                // ChatComponent.clearMessages(false): sent history stays.
                self.ui.chat.clear();
                true
            }
            DebugAction::ShowChunkBorders if player => {
                self.show_chunk_borders = !self.show_chunk_borders;
                self.debug_translated(if self.show_chunk_borders {
                    "debug.chunk_boundaries.on"
                } else {
                    "debug.chunk_boundaries.off"
                });
                true
            }
            DebugAction::AdvancedTooltips => {
                // The message comes before Options.save here.
                let on = !self.settings.file.bool("advancedItemTooltips");
                self.debug_translated(if on {
                    "debug.advanced_tooltips.on"
                } else {
                    "debug.advanced_tooltips.off"
                });
                self.toggle_option("advancedItemTooltips");
                true
            }
            DebugAction::CopyRecreateCommand => {
                if player {
                    self.copy_recreate_command(self.gamemaster(), !shift);
                }
                true
            }
            DebugAction::Spectate => {
                if self.player.is_none() || !self.gamemaster() {
                    self.debug_translated("debug.creative_spectator.error");
                } else if self.mode != GameMode::Spectator {
                    self.change_game_mode(GameMode::Spectator);
                } else {
                    let back = self.previous_mode.unwrap_or(GameMode::Creative);
                    self.change_game_mode(back);
                }
                true
            }
            DebugAction::SwitchGameMode if self.open_screen() == OpenScreen::None => {
                if self.player.is_none() || !self.gamemaster() {
                    self.debug_translated("debug.gamemodes.error");
                } else {
                    self.open_gamemode_switcher();
                }
                true
            }
            DebugAction::FocusPause => {
                let on = self.toggle_option("pauseOnLostFocus");
                self.debug_translated(if on {
                    "debug.pause_focus.on"
                } else {
                    "debug.pause_focus.off"
                });
                true
            }
            DebugAction::ReloadResourcePacks => {
                self.debug_translated("debug.reload_resourcepacks.message");
                self.reload_packs();
                true
            }
            DebugAction::CopyLocation if player => {
                self.debug_translated("debug.copy_location.message");
                if let Some(p) = &self.player {
                    use horizon_client::java_text::format_fixed2 as f2;
                    let command = format!(
                        "/execute in {} run tp @s {} {} {} {} {}",
                        self.dim.id(),
                        f2(p.position.x),
                        f2(p.position.y),
                        f2(p.position.z),
                        f2(f64::from(self.yaw)),
                        f2(f64::from(self.pitch)),
                    );
                    self.set_clipboard(&command);
                }
                true
            }
            DebugAction::DumpVersion => {
                self.debug_translated("debug.version.header");
                for line in self.version_lines() {
                    self.system_message(line);
                }
                true
            }
            DebugAction::ImprovedTransparency => {
                self.settings.improved_transparency = !self.settings.improved_transparency;
                self.settings.save(&Self::options_path());
                self.apply_settings();
                self.debug_translated(if self.settings.improved_transparency {
                    "debug.improvedTransparency.on"
                } else {
                    "debug.improvedTransparency.off"
                });
                true
            }
            DebugAction::FpsCharts => {
                self.debug_charts.toggle_fps(&mut self.debug);
                true
            }
            DebugAction::NetworkCharts => {
                self.debug_charts.toggle_network(&mut self.debug);
                true
            }
            DebugAction::LightmapTexture => {
                self.debug_charts.toggle_lightmap(&mut self.debug);
                true
            }
            // Screens and overlays Horizon does not draw yet (the debug
            // options screen, the profiler and its chart, the texture dump):
            // the keys are used up as in vanilla.
            DebugAction::DebugOptions
            | DebugAction::DumpDynamicTextures
            | DebugAction::Profiling
            | DebugAction::ProfilingChart => true,
            DebugAction::ShowHitboxes
            | DebugAction::ShowChunkBorders
            | DebugAction::SwitchGameMode
            | DebugAction::CopyLocation => false,
        }
    }
    /// `ServerboundChangeGameModePacket` handled by the integrated server:
    /// `GameModeCommand.setGameMode` with its feedback.
    pub(super) fn change_game_mode(&mut self, mode: GameMode) {
        if self.remote.is_some() {
            use horizon_protocol::packets::play::{ChangeGameMode, Serverbound};
            let mode = match mode {
                GameMode::Survival => 0,
                GameMode::Creative => 1,
                GameMode::Adventure => 2,
                GameMode::Spectator => 3,
            };
            self.send(Serverbound::ChangeGameMode(ChangeGameMode { mode }));
            return;
        }
        if self.set_game_mode(mode) {
            let name = Text::translate(&format!("gameMode.{}", mode.name()), &[]);
            self.feedback(Text::translate("commands.gamemode.success.self", &[name]));
        }
    }
    /// `KeyboardHandler.copyRecreateCommand`: the targeted block or mob as a
    /// `/setblock` or `/summon` command. In a single-player world the
    /// server's data (no Shift) is the game's own.
    fn copy_recreate_command(&mut self, add_nbt: bool, from_server: bool) {
        use horizon_client::java_text::format_fixed2 as f2;
        let side = if add_nbt && from_server {
            "server"
        } else {
            "client"
        };
        if self.entity_targeted()
            && let Some((index, _)) = self.mob_hit()
        {
            let (id, pos, mob) = {
                let m = &self.mobs[index];
                (m.kind.id.clone(), m.position, m.id)
            };
            let mut command = format!("/summon {id} {} {} {}", f2(pos.x), f2(pos.y), f2(pos.z));
            if add_nbt && let Some(mut tag) = self.entity_nbt(super::commands::Target::Mob(mob)) {
                if let horizon_world::nbt::Tag::Compound(c) = &mut tag {
                    for key in ["id", "UUID", "Pos"] {
                        c.remove(&key.into());
                    }
                }
                command.push(' ');
                command.push_str(&horizon_client::snbt::pretty(&tag).plain());
            }
            self.set_clipboard(&command);
            self.debug_translated(&format!("debug.inspect.{side}.entity"));
            return;
        }
        let Some(hit) = self.pick() else {
            return;
        };
        let p = hit.pos;
        let mut description = self.state_string(self.world.block(p[0], p[1], p[2]));
        if add_nbt && let Some(e) = self.world.block_entities.get(&(p[0], p[1], p[2])) {
            // BlockEntity.saveWithoutMetadata: no id or position.
            let name = self.data_block_name(p);
            let mut tag = horizon_client::save::encode_block_entity((p[0], p[1], p[2]), e, &name);
            if let horizon_world::nbt::Tag::Compound(c) = &mut tag {
                for key in ["id", "x", "y", "z", "keepPacked"] {
                    c.remove(&key.into());
                }
            }
            description.push_str(&horizon_client::snbt::to_snbt(&tag));
        }
        self.set_clipboard(&format!(
            "/setblock {} {} {} {description}",
            p[0], p[1], p[2]
        ));
        self.debug_translated(&format!("debug.inspect.{side}.block"));
    }
    /// `KeyboardHandler.setClipboard`.
    fn set_clipboard(&mut self, text: &str) {
        use horizon_client::clipboard::Clipboard;
        self.clipboard.set(text);
    }
    /// The debug renderers' gizmos for a frame (`DebugRenderer.emitGizmos`).
    pub(super) fn debug_gizmos(&self, alpha: f32) -> Gizmos {
        let mut g = Gizmos::default();
        if self.show_chunk_borders
            && let Some(p) = &self.player
        {
            let block = [
                p.position.x.floor() as i32,
                p.position.y.floor() as i32,
                p.position.z.floor() as i32,
            ];
            let min_y = horizon_client::chunk::MIN_Y;
            g.chunk_borders(block, min_y, min_y + horizon_client::chunk::HEIGHT - 1);
        }
        if !self.show_hitboxes {
            return g;
        }
        let lerp = |a: f64, b: f64| a + f64::from(alpha) * (b - a);
        let lerpf = |a: f32, b: f32| a + alpha * (b - a);
        for m in &self.mobs {
            if m.removed || m.has_effect("minecraft:invisibility") {
                continue;
            }
            let pos = [
                lerp(m.previous.x, m.position.x),
                lerp(m.previous.y, m.position.y),
                lerp(m.previous.z, m.position.z),
            ];
            let view = (
                lerpf(m.previous_head_pitch, m.head_pitch),
                lerpf(m.previous_head_yaw, m.head_yaw),
            );
            g.hitbox(
                pos,
                (m.width(), m.height()),
                m.eye_height() as f32,
                true,
                view,
            );
        }
        for item in &self.dropped {
            let pos = [
                lerp(item.previous.x, item.position.x),
                lerp(item.previous.y, item.position.y),
                lerp(item.previous.z, item.position.z),
            ];
            // EntityType.ITEM: 0.25 by 0.25, eyes at 0.2125.
            g.hitbox(pos, (0.25, 0.25), 0.2125, false, (0.0, 0.0));
        }
        // The dragon's parts.
        for (id, b) in self.special_targets() {
            if (super::dragon::SPECIAL..super::dragon::SPECIAL + 8).contains(&id) {
                g.cuboid(
                    [b.min(Axis::X), b.min(Axis::Y), b.min(Axis::Z)],
                    [b.max(Axis::X), b.max(Axis::Y), b.max(Axis::Z)],
                    horizon_client::gizmos::DRAGON_PART,
                    horizon_client::gizmos::STROKE_WIDTH,
                );
            }
        }
        // The player's own, unless the camera is in its head.
        if !self.camera_type.first_person()
            && let Some(p) = &self.player
        {
            let d = p.dimensions;
            let pos = [
                lerp(self.previous_position.x, p.position.x),
                lerp(self.previous_position.y, p.position.y),
                lerp(self.previous_position.z, p.position.z),
            ];
            g.hitbox(
                pos,
                (d.width, d.height),
                d.eye_height,
                true,
                (self.pitch, self.yaw),
            );
        }
        g
    }
}
