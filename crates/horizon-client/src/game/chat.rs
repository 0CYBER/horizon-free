//! Chat, the command registries and the per-world state behind commands
//! (game rules, the world clock, difficulty), bound to the game.
use super::{Game, Screen};
use horizon_client::{
    chat::{Chat, ChatAction, ChatScreen, Commands, SYSTEM_INDICATOR},
    command::Registries,
    gamerules::GameRules,
    hud::HudBatch,
    screen_draw::Measure,
    text::{ClickEvent, Style, Text},
};
use std::collections::BTreeMap;

/// The tree the chat parses and completes with: a server's, once it has
/// sent one (ClientPacketListener.commands), else the game's own.
fn chat_tree<'a>(
    remote: &'a Option<super::remote::Remote>,
    mods: &'a super::mods::ModsState,
) -> &'a horizon_client::command::Dispatcher {
    match remote.as_ref().and_then(|r| r.commands.as_ref()) {
        Some(tree) => tree,
        None => mods.dispatcher(),
    }
}
use winit::{
    event::KeyEvent,
    keyboard::{KeyCode, PhysicalKey},
};

/// The local player's name (offline single player).
pub const PLAYER_NAME: &str = "Player";

/// ServerClockManager.ServerClockInstance for the overworld clock (its
/// total ticks are the world's day time).
#[derive(Clone, Copy, Debug)]
pub struct WorldClock {
    pub partial: f32,
    pub rate: f32,
    pub paused: bool,
}
impl Default for WorldClock {
    fn default() -> Self {
        Self {
            partial: 0.0,
            rate: 1.0,
            paused: false,
        }
    }
}
/// A clock time marker from the timelines (ClockTimeMarker).
#[derive(Clone, Copy, Debug)]
pub struct TimeMarker {
    pub ticks: i64,
    pub period: Option<i64>,
    pub show: bool,
}
impl TimeMarker {
    pub fn occurs_at(&self, total: i64) -> bool {
        match self.period {
            Some(p) => self.ticks == total.rem_euclid(p),
            None => self.ticks == total,
        }
    }
    pub fn resolve(&self, total: i64) -> i64 {
        match self.period {
            Some(p) => {
                let d = self.ticks - total.rem_euclid(p);
                total + if d > 0 { d } else { p + d }
            }
            None => self.ticks,
        }
    }
}
/// UI and command state that lives beside the game.
#[derive(Default)]
pub struct UiState {
    pub chat: Chat,
    pub chat_screen: Option<ChatScreen>,
    /// When the chat screen opened (its input took the focus: the cursor blink).
    pub chat_opened: Option<std::time::Instant>,
    pub registries: Registries,
    pub rules: GameRules,
    pub clock: WorldClock,
    /// Time markers and timelines of the overworld clock.
    pub markers: BTreeMap<String, TimeMarker>,
    pub timelines: BTreeMap<String, Option<i64>>,
    /// 0 peaceful .. 3 hard.
    pub difficulty: u8,
    /// F1: hide the HUD.
    pub hide_gui: bool,
    /// Death message key set by the cause of death (e.g. /kill).
    pub death_cause: Option<String>,
    /// Path of an F2 screenshot waiting for its frame.
    pub screenshot_message: Option<std::path::PathBuf>,
    /// The options screens.
    pub options: horizon_client::options_ui::OptionsUi,
    /// The prepared resources before resource packs are layered on.
    pub vanilla_root: std::path::PathBuf,
    /// Death screen cause and when the player died (buttons wait 20 ticks).
    pub death_text: Option<Text>,
    pub died_at: Option<std::time::Instant>,
}

impl Game<'_> {
    /// `PlayerList.isOp` for the single-player owner: the world's Allow Commands.
    pub(super) fn allow_commands(&self) -> bool {
        self.level.allow_commands == Some(true)
    }
    /// `IntegratedServer.setWorldAllowCommands`, then `updatePermissions`.
    pub(super) fn set_allow_commands(&mut self, allow: bool) {
        self.level.allow_commands = Some(allow);
        self.ui.registries.permission = owner_permission(allow);
    }
    /// Commands the game issues itself (the server console, harnesses, the
    /// control socket) run at the owner's level whatever Allow Commands says.
    pub(super) fn as_console<T>(&mut self, f: impl FnOnce(&mut Self) -> T) -> T {
        let level = std::mem::replace(&mut self.ui.registries.permission, 4);
        let out = f(self);
        self.ui.registries.permission = level;
        out
    }
    /// Registry contents for argument parsing (built when a world opens).
    pub(super) fn build_registries(&mut self) {
        let permission = owner_permission(self.allow_commands());
        let reg = &mut self.ui.registries;
        reg.permission = permission;
        {
            let dir = |sub: &str| -> Vec<String> {
                let root = self.options.resources.join("data/minecraft").join(sub);
                let mut out = Vec::new();
                let mut stack = vec![root.clone()];
                while let Some(d) = stack.pop() {
                    let Ok(entries) = std::fs::read_dir(&d) else {
                        continue;
                    };
                    for e in entries.flatten() {
                        let p = e.path();
                        if p.is_dir() {
                            stack.push(p);
                        } else if p.extension().and_then(|x| x.to_str()) == Some("json")
                            && let Ok(rel) = p.strip_prefix(&root)
                        {
                            out.push(format!(
                                "minecraft:{}",
                                rel.with_extension("").to_string_lossy().replace('\\', "/")
                            ));
                        }
                    }
                }
                out.sort();
                out
            };
            self.ui.registries.recipes = dir("recipe");
            self.ui.registries.damage_types = dir("damage_type");
            self.ui.registries.loot_tables = dir("loot_table");
            self.ui.registries.templates = {
                let root = self.options.resources.join("worldgen/structure/minecraft");
                let mut out = Vec::new();
                let mut stack = vec![root.clone()];
                while let Some(d) = stack.pop() {
                    let Ok(entries) = std::fs::read_dir(&d) else {
                        continue;
                    };
                    for e in entries.flatten() {
                        let p = e.path();
                        if p.is_dir() {
                            stack.push(p);
                        } else if p.extension().and_then(|x| x.to_str()) == Some("nbt")
                            && let Ok(rel) = p.strip_prefix(&root)
                        {
                            out.push(format!(
                                "minecraft:{}",
                                rel.with_extension("").to_string_lossy().replace('\\', "/")
                            ));
                        }
                    }
                }
                out.sort();
                out
            };
        }
        self.ui.registries.sounds = self.audio.event_names();
        self.ui.registries.attributes = [
            "armor",
            "armor_toughness",
            "attack_damage",
            "attack_knockback",
            "attack_speed",
            "block_break_speed",
            "block_interaction_range",
            "burning_time",
            "camera_distance",
            "explosion_knockback_resistance",
            "entity_interaction_range",
            "fall_damage_multiplier",
            "flying_speed",
            "follow_range",
            "gravity",
            "jump_strength",
            "knockback_resistance",
            "luck",
            "max_absorption",
            "max_health",
            "mining_efficiency",
            "movement_efficiency",
            "movement_speed",
            "oxygen_bonus",
            "safe_fall_distance",
            "scale",
            "sneaking_speed",
            "spawn_reinforcements",
            "step_height",
            "submerged_mining_speed",
            "sweeping_damage_ratio",
            "tempt_range",
            "water_movement_efficiency",
        ]
        .iter()
        .map(|a| format!("minecraft:{a}"))
        .collect();
        let reg = &mut self.ui.registries;
        let mut items: Vec<String> = self
            .res
            .items()
            .order
            .iter()
            .map(|s| s.to_string())
            .collect();
        items.sort();
        reg.items = items;
        reg.item_tags = self
            .res
            .items()
            .item_tags
            .keys()
            .map(|k| horizon_client::command::args::normalize(k))
            .collect();
        reg.item_tags.sort();
        reg.block_tags = self
            .res
            .items()
            .block_tags
            .keys()
            .map(|k| horizon_client::command::args::normalize(k))
            .collect();
        reg.block_tags.sort();
        let registry = self.pack.assets.registry;
        let mut blocks: BTreeMap<String, BTreeMap<String, Vec<String>>> = BTreeMap::new();
        for id in 0..registry.len() {
            let Some(state) = registry.state(horizon_world::registry::BlockStateId(id as u32))
            else {
                continue;
            };
            let props = blocks.entry(state.name.clone()).or_default();
            for (k, v) in &state.properties {
                let values = props.entry(k.clone()).or_default();
                if !values.contains(v) {
                    values.push(v.clone());
                }
            }
        }
        reg.blocks = blocks
            .into_iter()
            .map(|(k, v)| (k, v.into_iter().collect()))
            .collect();
        let mut summonable: Vec<String> = self
            .res
            .mob_types()
            .keys()
            .map(|k| horizon_client::command::args::normalize(k))
            .collect();
        summonable.extend(
            [
                "minecraft:item",
                "minecraft:experience_orb",
                "minecraft:falling_block",
                "minecraft:arrow",
                "minecraft:lightning_bolt",
                "minecraft:minecart",
                "minecraft:chest_minecart",
                "minecraft:hopper_minecart",
                "minecraft:furnace_minecart",
                "minecraft:tnt_minecart",
            ]
            .map(String::from),
        );
        summonable.sort();
        summonable.dedup();
        reg.summonable = summonable;
        let root = self.options.resources.clone();
        let mut types: Vec<String> = serde_json::from_slice::<serde_json::Value>(
            &std::fs::read(root.join("entity-types.json")).unwrap_or_default(),
        )
        .ok()
        .and_then(|v| {
            v.as_object().map(|o| {
                o.keys()
                    .map(|k| horizon_client::command::args::normalize(k))
                    .collect()
            })
        })
        .unwrap_or_default();
        if types.is_empty() {
            types = reg.summonable.clone();
            types.push("minecraft:player".into());
        }
        types.sort();
        reg.entity_types = types;
        let list = |dir: &str| -> Vec<String> {
            let mut out: Vec<String> = std::fs::read_dir(root.join(dir))
                .into_iter()
                .flatten()
                .flatten()
                .filter_map(|e| {
                    e.file_name()
                        .to_str()?
                        .strip_suffix(".json")
                        .map(|n| format!("minecraft:{n}"))
                })
                .collect();
            out.sort();
            out
        };
        // The mob effect registry: every effect `/effect` and the
        // `mob_effect` resource arguments accept.
        reg.effects = horizon_client::brewing::EFFECTS
            .iter()
            .map(|e| format!("minecraft:{}", e.id))
            .collect();
        reg.effects.sort();
        reg.enchantments = list("data/minecraft/enchantment");
        reg.structures = list("data/minecraft/worldgen/structure");
        reg.biomes = list("data/minecraft/worldgen/biome");
        reg.clocks = list("data/minecraft/world_clock");
        reg.dimensions = vec![
            "minecraft:overworld".into(),
            "minecraft:the_nether".into(),
            "minecraft:the_end".into(),
        ];
        reg.players = vec![PLAYER_NAME.into()];
        // Timelines of the overworld clock and their time markers.
        self.ui.markers.clear();
        self.ui.timelines.clear();
        for name in list("data/minecraft/timeline") {
            let path = root.join(format!(
                "data/minecraft/timeline/{}.json",
                name.trim_start_matches("minecraft:")
            ));
            let Ok(v) = serde_json::from_slice::<serde_json::Value>(
                &std::fs::read(path).unwrap_or_default(),
            ) else {
                continue;
            };
            if v["clock"].as_str() != Some("minecraft:overworld") {
                continue;
            }
            let period = v["period_ticks"].as_i64();
            self.ui.timelines.insert(name.clone(), period);
            for (marker, def) in v["time_markers"].as_object().into_iter().flatten() {
                let (ticks, show) = match def {
                    serde_json::Value::Number(n) => (n.as_i64().unwrap_or(0), false),
                    other => (
                        other["ticks"].as_i64().unwrap_or(0),
                        other["show_in_commands"].as_bool().unwrap_or(false),
                    ),
                };
                self.ui.markers.insert(
                    marker.clone(),
                    TimeMarker {
                        ticks,
                        period,
                        show,
                    },
                );
            }
        }
        // Vanilla's built-in registries (mob effects, components, dialogs...).
        for dir in [&self.options.resources, &self.ui.vanilla_root] {
            if let Ok(bytes) = std::fs::read(dir.join("command-registries.json"))
                && let Ok(v) = serde_json::from_slice::<serde_json::Value>(&bytes)
            {
                self.ui.registries.fill_from_export(&v);
                break;
            }
        }
        let reg = &mut self.ui.registries;
        reg.timelines = self.ui.timelines.keys().cloned().collect();
        reg.time_markers = self
            .ui
            .markers
            .iter()
            .filter(|(_, m)| m.show)
            .map(|(k, _)| k.clone())
            .collect();
    }
    /// The block the player is looking at, for coordinate suggestions.
    fn refresh_dynamic_registries(&mut self) {
        self.ui.registries.target_block = self.pick().map(|h| h.pos);
        // ClientPacketListener.getOnlinePlayerNames on a server.
        if let Some(names) = self.remote_online_names() {
            self.ui.registries.players = names;
        }
    }
    pub(super) fn open_chat(&mut self, initial: &str) {
        if self.player.is_none() {
            return;
        }
        self.refresh_dynamic_registries();
        let size = self
            .renderer
            .as_ref()
            .map_or([1280, 720], |r| [r.config.width, r.config.height]);
        let b = HudBatch::new(size);
        let m = Measure(&self.gui.atlas.font);
        let c = Commands {
            dispatcher: chat_tree(&self.remote, &self.mods),
            registries: &self.ui.registries,
        };
        let mut screen = ChatScreen::new(
            &m,
            &c,
            initial,
            self.ui.chat.recent.len(),
            b.width as i32,
            b.height as i32,
        );
        screen.auto_suggestions = self.settings.auto_suggestions;
        self.ui.chat_screen = Some(screen);
        self.ui.chat_opened = Some(std::time::Instant::now());
        self.screen = Screen::Chat;
        self.keys.clear();
        self.left = false;
        self.right = false;
        self.capture(false);
    }
    pub(super) fn close_chat(&mut self) {
        self.ui.chat_screen = None;
        self.ui.chat.reset_scroll();
        if self.screen == Screen::Chat {
            self.screen = Screen::Playing;
            self.capture(true);
        }
    }
    /// A chat message from the game (ChatComponent.addServerSystemMessage).
    pub(super) fn system_message(&mut self, text: Text) {
        if let Some(capture) = self.chat_capture.as_mut() {
            capture.push(text);
            return;
        }
        // Connected to a server (single player's own included), the chat on
        // screen is the connection's: the game's own messages join it.
        if self.remote.is_some() {
            self.chat_line(text);
            return;
        }
        let focused = self.screen == Screen::Chat;
        let heard = self.mods.running().then(|| text.plain());
        self.ui.chat.add(
            &self.gui.atlas,
            text,
            Some(SYSTEM_INDICATOR),
            self.ticks,
            focused,
        );
        if let Some(line) = heard {
            self.mods_chat(&line);
        }
    }
    /// Keys while the chat screen is open.
    pub(super) fn chat_key(&mut self, event: &KeyEvent) {
        if let Some(shortcut) = self.edit_shortcut(event) {
            let m = Measure(&self.gui.atlas.font);
            let c = Commands {
                dispatcher: chat_tree(&self.remote, &self.mods),
                registries: &self.ui.registries,
            };
            if let Some(screen) = self.ui.chat_screen.as_mut() {
                screen.shortcut(&m, &c, shortcut, &mut self.clipboard);
            }
            return;
        }
        let PhysicalKey::Code(key) = event.physical_key else {
            return;
        };
        self.chat_key_code(key, event.text.as_deref());
    }
    /// A key (and the text it typed) while the chat screen is open.
    pub(super) fn chat_key_code(&mut self, key: KeyCode, typed_text: Option<&str>) {
        // InputQuirks.EDIT_SHORTCUT_KEY_MODIFIER: Cmd on macOS, Ctrl elsewhere.
        let (left, right) = if cfg!(target_os = "macos") {
            (KeyCode::SuperLeft, KeyCode::SuperRight)
        } else {
            (KeyCode::ControlLeft, KeyCode::ControlRight)
        };
        let ctrl = self.keys.contains(&left) || self.keys.contains(&right);
        let shift =
            self.keys.contains(&KeyCode::ShiftLeft) || self.keys.contains(&KeyCode::ShiftRight);
        match key {
            // ChatScreen.keyPressed: the chat scrolls a page.
            KeyCode::PageUp | KeyCode::PageDown
                if !self
                    .ui
                    .chat_screen
                    .as_ref()
                    .is_some_and(|s| s.suggestions_visible()) =>
            {
                let n = self.ui.chat.lines_per_page(true) as i32 - 1;
                self.ui
                    .chat
                    .scroll_by(if key == KeyCode::PageUp { n } else { -n }, true);
                return;
            }
            _ => {}
        }
        let Some(mut screen) = self.ui.chat_screen.take() else {
            return;
        };
        let m = Measure(&self.gui.atlas.font);
        let c = Commands {
            dispatcher: chat_tree(&self.remote, &self.mods),
            registries: &self.ui.registries,
        };
        match screen.key(&m, &c, &self.ui.chat.recent, key, shift, ctrl) {
            ChatAction::Close => self.close_chat(),
            ChatAction::Submit(message) => {
                self.close_chat();
                if !message.is_empty() {
                    self.handle_chat_input(&message);
                }
            }
            ChatAction::None => {
                // charTyped: the text the key typed.
                if let Some(text) = typed_text
                    && !ctrl
                {
                    screen.type_text(&m, &c, text);
                }
                self.ui.chat_screen = Some(screen);
            }
        }
    }
    /// The server's completions for an `ask_server` request
    /// (ClientPacketListener.handleCommandSuggestions).
    pub(super) fn chat_server_suggestions(
        &mut self,
        id: i32,
        range: (usize, usize),
        list: Vec<(String, Option<String>)>,
    ) {
        let m = Measure(&self.gui.atlas.font);
        let c = Commands {
            dispatcher: chat_tree(&self.remote, &self.mods),
            registries: &self.ui.registries,
        };
        if let Some(screen) = self.ui.chat_screen.as_mut() {
            screen.server_suggestions(&m, &c, id, range, list);
        }
    }
    /// Text typed into the chat box (control socket `chat`).
    pub(super) fn chat_type(&mut self, text: &str) {
        let m = Measure(&self.gui.atlas.font);
        let c = Commands {
            dispatcher: chat_tree(&self.remote, &self.mods),
            registries: &self.ui.registries,
        };
        if let Some(screen) = self.ui.chat_screen.as_mut() {
            screen.type_text(&m, &c, text);
        }
    }
    /// ChatScreen.handleChatInput: history, then a command or a chat line.
    pub(super) fn handle_chat_input(&mut self, message: &str) {
        self.ui.chat.add_recent(message);
        // On a server (single player's own included) the server runs it,
        // unless a client mod owns the command.
        if self.remote.is_some() {
            if let Some(command) = message.strip_prefix('/')
                && self.mod_command(command)
            {
                return;
            }
            self.send_chat(message);
            return;
        }
        if let Some(command) = message.strip_prefix('/') {
            self.run_command(command);
        } else {
            let line = Text::translate(
                "chat.type.text",
                &[Text::literal(PLAYER_NAME), Text::literal(message)],
            );
            let focused = self.screen == Screen::Chat;
            let heard = self.mods.running().then(|| line.plain());
            self.ui
                .chat
                .add(&self.gui.atlas, line, None, self.ticks, focused);
            if let Some(line) = heard {
                self.mods_chat(&line);
            }
            self.lan_host_chat(message);
        }
    }
    pub(super) fn chat_scroll(&mut self, amount: f32) {
        let cursor = self.cursor;
        let height = self.renderer.as_ref().map_or(720.0, |r| {
            HudBatch::new([r.config.width, r.config.height]).height
        });
        let _ = height;
        let mouse = (cursor.0.floor() as i32, cursor.1.floor() as i32);
        if let Some(screen) = self.ui.chat_screen.as_mut()
            && screen.scroll(mouse, amount as f64)
        {
            return;
        }
        let shift = self.keys.contains(&KeyCode::ShiftLeft);
        let amount = amount.clamp(-1.0, 1.0) * if shift { 1.0 } else { 7.0 };
        self.ui.chat.scroll_by(amount as i32, true);
    }
    pub(super) fn chat_click(&mut self) {
        let cursor = self.cursor;
        let height = self.renderer.as_ref().map_or(720.0, |r| {
            HudBatch::new([r.config.width, r.config.height]).height
        });
        let m = Measure(&self.gui.atlas.font);
        let c = Commands {
            dispatcher: chat_tree(&self.remote, &self.mods),
            registries: &self.ui.registries,
        };
        if let Some(screen) = self.ui.chat_screen.as_mut() {
            if screen.click(&m, &c, cursor.0 as f64, cursor.1 as f64) {
                return;
            }
        } else {
            return;
        }
        match self.ui.chat.click_at(
            &self.gui.atlas.font,
            height as i32,
            cursor.0 as f64,
            cursor.1 as f64,
        ) {
            Some(ClickEvent::CopyToClipboard(value)) => {
                use horizon_client::clipboard::Clipboard;
                self.clipboard.set(&value);
            }
            Some(ClickEvent::OpenFile(path)) => {
                super::packs::open_folder(std::path::Path::new(&path));
            }
            Some(ClickEvent::SuggestCommand(command)) => {
                if let Some(screen) = self.ui.chat_screen.as_mut() {
                    screen.suggest_command(&m, &c, &command);
                }
            }
            None => {}
        }
    }
    /// Chat lines over the HUD (focused while the chat screen is open).
    pub(super) fn draw_chat(&mut self, batch: &mut HudBatch) {
        let focused = self.screen == Screen::Chat;
        if self.ui.hide_gui && !focused {
            return;
        }
        self.ui
            .chat
            .draw(batch, &self.gui.atlas, self.ticks, focused);
    }
    pub(super) fn draw_chat_screen(&mut self, batch: &mut HudBatch) {
        let Some(mut screen) = self.ui.chat_screen.take() else {
            return;
        };
        let m = Measure(&self.gui.atlas.font);
        let c = Commands {
            dispatcher: chat_tree(&self.remote, &self.mods),
            registries: &self.ui.registries,
        };
        screen.resize(&m, &c, batch.width as i32, batch.height as i32);
        // Options.getBackgroundColor(Integer.MIN_VALUE).
        let background = if self.settings.file.bool("backgroundForChatOnly") {
            i32::MIN
        } else {
            self.settings.text_background(0.0) as i32
        };
        let mouse = (self.cursor.0.floor() as i32, self.cursor.1.floor() as i32);
        let blink = self
            .ui
            .chat_opened
            .map_or(0, |t| t.elapsed().as_millis() as u64);
        let draws = screen.draw(&m, mouse, blink, background);
        horizon_client::screen_draw::paint(batch, &self.gui.atlas, &draws);
        self.ui.chat_screen = Some(screen);
        // The hovered message part's tooltip, one row per line of its text.
        if let Some(hover) = self.ui.chat.hover_at(
            &self.gui.atlas.font,
            batch.height as i32,
            f64::from(self.cursor.0),
            f64::from(self.cursor.1),
        ) {
            let lines = hover_rows(&hover);
            self.draw_tooltip_lines(batch, &lines, self.cursor.0, self.cursor.1);
        }
    }
    /// Advance the overworld clock (ServerClockManager.tick) when
    /// `advance_time` allows it.
    pub(super) fn advance_clock(&mut self) {
        if !self.ui.rules.bool("advance_time") || self.ui.clock.paused {
            return;
        }
        let c = &mut self.ui.clock;
        c.partial += c.rate;
        let whole = c.partial.floor();
        c.partial -= whole;
        self.world.day_time += whole as i64;
    }
    /// `ServerClockInstance.packNetworkState`: the rate players' clients
    /// advance the overworld clock at, 0 while it is paused (`/time pause`)
    /// or `advance_time` is off.
    pub(super) fn network_clock_rate(&self) -> f32 {
        if self.ui.rules.bool("advance_time") && !self.ui.clock.paused {
            self.ui.clock.rate
        } else {
            0.0
        }
    }
    /// The engine-side rule values (block behaviour and health).
    pub(super) fn level_rules(&self) -> horizon_world::level::GameRules {
        let r = &self.ui.rules;
        horizon_world::level::GameRules {
            water_source_conversion: r.bool("water_source_conversion"),
            lava_source_conversion: r.bool("lava_source_conversion"),
            random_tick_speed: r.int("random_tick_speed"),
            block_drops: r.bool("block_drops"),
            mob_griefing: r.bool("mob_griefing"),
            max_snow_accumulation_height: r.int("max_snow_accumulation_height"),
            spawn_mobs: r.bool("spawn_mobs"),
            spawn_monsters: r.bool("spawn_monsters"),
        }
    }
    /// Apply rules mirrored in other state (natural regeneration, difficulty).
    pub(super) fn sync_rules(&mut self) {
        self.world.health_rules.natural_regeneration =
            self.ui.rules.bool("natural_health_regeneration");
        use horizon_world::food::Difficulty;
        self.world.health_rules.difficulty = match self.ui.difficulty {
            0 => Difficulty::Peaceful,
            1 => Difficulty::Easy,
            3 => Difficulty::Hard,
            _ => Difficulty::Normal,
        };
    }
    /// Load per-world command state when a world opens.
    pub(super) fn load_world_state(&mut self) {
        self.ui.rules = GameRules::load(&self.world_root);
        self.ui.clock = WorldClock::default();
        self.ui.difficulty = read_difficulty(&self.world_root).unwrap_or(2);
        if let Ok(bytes) = std::fs::read(self.world_root.join("data/minecraft/world_clocks.dat"))
            && let Ok(raw) = horizon_world::compression::Compression::Gzip.decode(&bytes, 1 << 20)
            && let Ok((root, _)) =
                horizon_world::nbt::read_named(&raw, horizon_world::nbt::Limits::default())
            && let Some(c) = root
                .tag
                .get("data")
                .and_then(|d| d.get("minecraft:overworld"))
        {
            use horizon_world::nbt::Tag;
            if let Some(Tag::Float(r)) = c.get("rate") {
                self.ui.clock.rate = f32::from_bits(*r);
            }
            if let Some(Tag::Float(p)) = c.get("partial_tick") {
                self.ui.clock.partial = f32::from_bits(*p);
            }
            if let Some(Tag::Byte(p)) = c.get("paused") {
                self.ui.clock.paused = *p != 0;
            }
        }
        self.ui.chat.clear();
        self.build_registries();
        self.sync_rules();
    }
    /// Save the clock and the difficulty beside level.dat (Level.write saves
    /// the game rules: a second writer of game_rules.dat could leave it with
    /// the rules loaded when the world opened).
    pub(super) fn save_world_state(&mut self) {
        if self.world_root.as_os_str().is_empty() {
            return;
        }
        use horizon_world::nbt::Tag;
        let c = self.ui.clock;
        // The other clocks stay as saved; the overworld one keeps the fields Horizon
        // does not model, and (like vanilla's codec) omits rate, partial tick and
        // pause while they have their defaults.
        let path = self.world_root.join("data/minecraft/world_clocks.dat");
        let existing = horizon_client::save::read_dat(&path);
        let mut data = match existing.as_ref().and_then(|r| r.get("data")) {
            Some(d @ Tag::Compound(_)) => d.clone(),
            _ => Tag::Compound(Default::default()),
        };
        let mut clock = match data.get("minecraft:overworld") {
            Some(c @ Tag::Compound(_)) => c.clone(),
            _ => Tag::Compound(Default::default()),
        };
        if let Tag::Compound(map) = &mut clock {
            map.insert("total_ticks".into(), Tag::Long(self.world.day_time));
            for (key, value, default) in [
                (
                    "partial_tick",
                    Tag::Float(c.partial.to_bits()),
                    c.partial == 0.0,
                ),
                ("rate", Tag::Float(c.rate.to_bits()), c.rate == 1.0),
                ("paused", Tag::Byte(i8::from(c.paused)), !c.paused),
            ] {
                if default {
                    map.remove(&key.into());
                } else {
                    map.insert(key.into(), value);
                }
            }
        }
        if let Tag::Compound(map) = &mut data {
            map.insert("minecraft:overworld".into(), clock);
        }
        let root = Tag::compound([
            ("data", data),
            ("DataVersion", Tag::Int(horizon_world::DATA_VERSION)),
        ]);
        if let Err(e) = horizon_client::save::write_dat(&path, root) {
            log::warn!("the world clocks were not written: {e}");
        }
        if let Err(e) = write_difficulty(&self.world_root, self.ui.difficulty) {
            log::warn!("the difficulty was not written: {e}");
        }
    }
    /// Death message in chat (CombatTracker.getDeathMessage, as far as the
    /// cause is known to the client).
    pub(super) fn death_message(&mut self) {
        let cause = self.ui.death_cause.take();
        self.ui.died_at = Some(std::time::Instant::now());
        self.ui.death_text = None;
        if !self.ui.rules.bool("show_death_messages") {
            return;
        }
        let Some(p) = self.player.as_ref() else {
            return;
        };
        let below_world = p.position.y < f64::from(horizon_client::chunk::MIN_Y - 64);
        let name = self.player_display_text();
        let message = match cause {
            Some(key) => Text::translate(&key, &[name]),
            None if below_world => Text::translate("death.attack.outOfWorld", &[name]),
            None => self.combat_death_message(),
        };
        self.ui.death_text = Some(message.clone());
        // ServerPlayer.die: a team that hides death messages keeps it out of
        // the dying player's own chat (the death screen still shows it).
        if horizon_client::scoreboard::own_death_message_shown(self.player_team()) {
            self.system_message(message);
        }
    }
    /// F1 hides the HUD (see `hud`); the screenshot key still works.
    pub(super) fn toggle_hide_gui(&mut self) {
        self.ui.hide_gui = !self.ui.hide_gui;
    }
    /// F2: ScreenShot.grab into `screenshots/` with vanilla's file name.
    pub(super) fn take_screenshot(&mut self) {
        let dir = std::env::current_dir()
            .unwrap_or_default()
            .join("screenshots");
        if std::fs::create_dir_all(&dir).is_err() {
            return;
        }
        let name = screenshot_name(&dir);
        let Some(renderer) = self.renderer.as_mut() else {
            return;
        };
        let path = dir.join(name);
        renderer.screenshot = Some(path.clone());
        self.ui.screenshot_message = Some(path);
    }
    /// After a frame: report a finished F2 screenshot in chat.
    pub(super) fn finish_screenshot(&mut self, result: &std::result::Result<[u32; 2], String>) {
        let Some(path) = self.ui.screenshot_message.take() else {
            return;
        };
        let message = match result {
            Ok(_) => {
                let name = path.file_name().unwrap_or_default().to_string_lossy();
                let link = Text::styled(
                    name,
                    Style {
                        underlined: Some(true),
                        click: Some(ClickEvent::OpenFile(path.to_string_lossy().into_owned())),
                        ..Style::default()
                    },
                );
                Text::translate("screenshot.success", &[link])
            }
            Err(e) => Text::translate("screenshot.failure", &[Text::literal(e.clone())]),
        };
        let focused = self.screen == Screen::Chat;
        self.ui
            .chat
            .add(&self.gui.atlas, message, None, self.ticks, focused);
    }
}
/// `yyyy-MM-dd_HH.mm.ss[_n].png` (Util.getFilenameFormattedDateTime, local time
/// approximated as UTC).
pub fn screenshot_name(dir: &std::path::Path) -> String {
    let secs = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |d| d.as_secs());
    let days = (secs / 86400) as i64;
    let (h, m, s) = ((secs % 86400) / 3600, (secs % 3600) / 60, secs % 60);
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let mo = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(mo <= 2);
    let base = format!("{y}-{mo:02}-{d:02}_{h:02}.{m:02}.{s:02}");
    let mut n = 1;
    loop {
        let name = if n == 1 {
            format!("{base}.png")
        } else {
            format!("{base}_{n}.png")
        };
        if !dir.join(&name).exists() {
            return name;
        }
        n += 1;
    }
}
fn read_difficulty(world: &std::path::Path) -> Option<u8> {
    let bytes = std::fs::read(world.join("level.dat")).ok()?;
    let raw = horizon_world::compression::Compression::Gzip
        .decode(&bytes, 64 << 20)
        .ok()?;
    let (root, _) =
        horizon_world::nbt::read_named(&raw, horizon_world::nbt::Limits::default()).ok()?;
    match root.tag.get("Data")?.get("Difficulty")? {
        horizon_world::nbt::Tag::Byte(b) => Some((*b).clamp(0, 3) as u8),
        _ => None,
    }
}
/// Patch `Data.Difficulty` in level.dat (Level::write stores the default).
fn write_difficulty(world: &std::path::Path, difficulty: u8) -> horizon_client::Result<()> {
    use horizon_world::{compression::Compression, nbt};
    let path = world.join("level.dat");
    let bytes = std::fs::read(&path)?;
    let raw = Compression::Gzip.decode(&bytes, 64 << 20)?;
    let (mut root, _) = nbt::read_named(&raw, nbt::Limits::default()).map_err(|e| e.to_string())?;
    if let Some(data) = root.tag.get_mut("Data")
        && let Some(d) = data.get_mut("Difficulty")
    {
        *d = nbt::Tag::Byte(difficulty as i8);
    }
    let out = nbt::write_named(&root, nbt::Limits::default()).map_err(|e| e.to_string())?;
    std::fs::write(&path, Compression::Gzip.encode(&out)?)?;
    Ok(())
}

/// `IntegratedServer.getProfilePermissions` for the owner: `OWNER` (4) when
/// commands are allowed, else `ALL` (0).
fn owner_permission(allow_commands: bool) -> u8 {
    if allow_commands { 4 } else { 0 }
}

/// A hover text's rows for the tooltip: split at line breaks, each in the
/// colour of its first styled character (white without one).
fn hover_rows(hover: &horizon_client::text::Text) -> Vec<(String, u32)> {
    let mut rows = vec![(String::new(), None)];
    for (c, style) in hover.resolve_legacy().chars() {
        if c == '\n' {
            rows.push((String::new(), None));
            continue;
        }
        let row = rows.last_mut().expect("one row");
        if row.0.is_empty() {
            row.1 = style.color;
        }
        row.0.push(c);
    }
    rows.into_iter()
        .map(|(text, color)| {
            let [_, r, g, b] = color.unwrap_or(0xFF_FFFF).to_be_bytes();
            (text, horizon_client::hud::rgba(r, g, b, 255))
        })
        .collect()
}
