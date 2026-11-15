//! The Rust side of tools/test_screen_build_parity.py: with
//! `HORIZON_SCREEN_BUILD_FIXTURE=CASES,OUTPUT` the game loads its resources,
//! opens each case's container or pause screen state, builds its layout and
//! prints the panel, the origin, the slot boxes, the slot a cursor point
//! hovers, the pause menu's buttons and icons, and the chat and command block
//! screens' widgets, in the format of `VanillaScreenBuildOracle`.
use super::{Game, Screen, screen_build::ScreenLayout};
use horizon_client::{
    Result,
    chat::{ChatAbilities, ChatScreen, ChatVisibility, Commands},
    chunk::Chunk,
    containers::{BlockEntity, Furnace, FurnaceKind},
    hud::{GuiScale, HudBatch},
    recipe_book::BookType,
    screen_draw::Measure,
    text::Text,
    window::GuiSize,
};
use serde_json::Value;
use std::io::Write as _;

/// A container screen's block and block entity at the fixture's one position.
const POS: (i32, i32, i32) = (0, 0, 0);

impl Game<'_> {
    /// Run every case of `spec` ("CASES,OUTPUT") and write the results.
    pub(super) fn screen_build_fixture(&mut self, spec: &str) -> Result<()> {
        let (input, output) = spec
            .split_once(',')
            .ok_or("HORIZON_SCREEN_BUILD_FIXTURE=CASES,OUTPUT")?;
        // The container screens read their panel from the block at POS.
        self.world
            .chunks
            .insert((0, 0), std::sync::Arc::new(Chunk::empty(0, 0)));
        let text = std::fs::read_to_string(input)?;
        let mut out = std::io::BufWriter::new(std::fs::File::create(output)?);
        for line in text.lines() {
            let case: Value = serde_json::from_str(line)?;
            match self.screen_build_case(&case) {
                Ok(value) => writeln!(out, "{value}")?,
                Err(e) => writeln!(out, "{}", serde_json::json!({"error": e.to_string()}))?,
            }
        }
        Ok(())
    }

    /// One case's screen state (`kind`, rows, recipe book, window, scale) and
    /// the build that shows it: the panel, the origin, the slot boxes and the
    /// slot each cursor point hovers, as `VanillaScreenBuildOracle` prints them.
    fn screen_build_case(&mut self, case: &Value) -> Result<Value> {
        let kind = case["kind"].as_str().ok_or("kind")?;
        let rows = case["rows"].as_u64().unwrap_or(3);
        let book = case["book"].as_bool().unwrap_or(false);
        let window = [
            case["window"][0].as_u64().ok_or("window")? as u32,
            case["window"][1].as_u64().ok_or("window")? as u32,
        ];
        let scale = case["scale"].as_u64().ok_or("scale")? as u32;

        // The pause menu: the same layout the game builds on open and resize.
        if kind == "pause" {
            self.screen = Screen::Paused;
            self.title = None;
            let batch = HudBatch::new(
                window,
                GuiScale {
                    requested: scale,
                    unicode: false,
                },
            );
            let gui = GuiSize::new(window, scale, false);
            let layout = self.pause_layout(batch.width, batch.height);
            self.screen_state.build.sync(
                self.screen,
                true,
                None,
                gui,
                Some(ScreenLayout::Pause(layout)),
            );
            let Some(layout) = self.built_pause(batch.width, batch.height) else {
                return Err("no build".into());
            };
            let buttons: Vec<Value> = layout
                .buttons
                .iter()
                .map(|&(x, y, w, key, _, _)| {
                    let label = Text::translate(key, &[]).plain();
                    serde_json::json!([x as i32, y as i32, w as i32, label])
                })
                .collect();
            let icons: Vec<Value> = layout
                .icons
                .iter()
                .map(|&(x, y, _)| serde_json::json!([x as i32, y as i32]))
                .collect();
            return Ok(serde_json::json!({
                "gui": [gui.scale, gui.size[0], gui.size[1]],
                "pause": {"buttons": buttons, "icons": icons},
            }));
        }
        // Chat (`InBedChatScreen`) and the command block edit screens: each
        // opens at another size, so the build must resize its boxes; then
        // every widget's rectangle in `init`'s order.
        if let Some(cart) = match kind {
            "chat" => Some(None),
            "command_block" => Some(Some(false)),
            "minecart_command_block" => Some(Some(true)),
            _ => None,
        } {
            let gui = GuiSize::new(window, scale, false);
            self.title = None;
            let rect = |x, y, w, h| serde_json::json!([x, y, w, h]);
            let mut widgets = Vec::new();
            match cart {
                None => {
                    let m = Measure(&self.gui.atlas.font);
                    let c = Commands {
                        dispatcher: horizon_client::command::vanilla::game_dispatcher(),
                        registries: &self.ui.registries,
                    };
                    let abilities = ChatAbilities::new(ChatVisibility::Full);
                    let chat = ChatScreen::new(&m, &c, "", false, 0, 1, 1, abilities);
                    self.ui.chat_screen = Some(chat);
                    self.screen = Screen::Chat;
                }
                Some(cart) => {
                    use horizon_client::command_block::{CommandBlock, Mode};
                    let block = CommandBlock::default();
                    self.open_edit_screen(POS, cart.then_some(0), block, Mode::Redstone, false);
                    self.screen = Screen::CommandBlock;
                }
            }
            self.screen_state.build.reopened();
            let layout = self.text_screen_layout(None, gui);
            self.screen_state
                .build
                .sync(self.screen, true, None, gui, layout);
            let (w, h) = (gui.size[0] as f32, gui.size[1] as f32);
            if let Some(chat) = &self.ui.chat_screen {
                let i = &chat.input;
                widgets.push(rect(i.x, i.y, i.width, i.height));
                let (x, y, bw, bh) = self.built_chat(w, h).ok_or("no build")?.leave_bed;
                widgets.push(rect(x, y, bw, bh));
            }
            if let Some(screen) = &self.command_block_screen {
                let (i, p) = (&screen.command.input, screen.previous());
                widgets.push(rect(i.x, i.y, i.width, i.height));
                widgets.push(rect(p[0] as i32, p[1] as i32, p[2] as i32, p[3] as i32));
                let layout = self.built_command_block(w, h).ok_or("no build")?;
                for &(_, x, y, bw) in &layout.buttons {
                    widgets.push(rect(x as i32, y as i32, bw as i32, 20));
                }
            }
            self.ui.chat_screen = None;
            self.command_block_screen = None;
            return Ok(serde_json::json!({
                "gui": [gui.scale, gui.size[0], gui.size[1]],
                "widgets": widgets,
            }));
        }
        // The case's screen: a survival/creative container in the inventory
        // screen, with the block entity the menu shows.
        self.screen = Screen::Inventory;
        self.title = None;
        self.craft_size = 2;
        self.container = None;
        self.world.block_entities.clear();
        self.level.recipe_book.open = [false; 4];
        let state = match kind {
            "chest" | "hopper" | "shulker" | "furnace" => {
                let (block, slots) = match kind {
                    "chest" => ("minecraft:chest", rows as usize * 9),
                    "hopper" => ("minecraft:hopper", 5),
                    "shulker" => ("minecraft:shulker_box", 27),
                    _ => ("minecraft:furnace", 3),
                };
                let state = self
                    .pack
                    .assets
                    .registry
                    .default_state(block)
                    .ok_or(block)?;
                let entity = if kind == "furnace" {
                    BlockEntity::Furnace(Furnace {
                        kind: FurnaceKind::Furnace,
                        items: [None, None, None],
                        lit_remaining: 0,
                        lit_total: 0,
                        cooking_timer: 0,
                        cooking_total: 0,
                        speed: 1.0,
                        recipes_used: std::collections::HashMap::new(),
                    })
                } else {
                    BlockEntity::Chest(vec![None; slots])
                };
                self.world.block_entities.insert(POS, entity);
                self.container = Some(POS);
                state
            }
            "crafting" => {
                self.craft_size = 3;
                self.pack
                    .assets
                    .registry
                    .default_state("minecraft:air")
                    .ok_or("air")?
            }
            "inventory" => self
                .pack
                .assets
                .registry
                .default_state("minecraft:air")
                .ok_or("air")?,
            other => return Err(format!("unknown screen {other}").into()),
        };
        self.world.set(POS.0, POS.1, POS.2, state)?;
        let book_type = match kind {
            "furnace" => BookType::Furnace,
            _ => BookType::Crafting,
        };
        self.level.recipe_book.open[book_type.index()] = book;

        let batch = HudBatch::new(
            window,
            GuiScale {
                requested: scale,
                unicode: false,
            },
        );
        let gui = GuiSize::new(window, scale, false);
        let layout = self.container_layout(&batch);
        // The frame's build, the way `observe_screen` makes it: the hit tests
        // and the panel readers below go through it.
        self.screen_state.build.sync(
            self.screen,
            true,
            None,
            gui,
            Some(ScreenLayout::Container(layout)),
        );

        let Some(layout) = self.built_container(&batch) else {
            return Err("no build".into());
        };
        let image = self.container_image(&batch);
        let (ox, oy) = self.inventory_origin(&batch);
        let mut slots: Vec<[i32; 2]> = layout
            .slots
            .iter()
            .map(|(_, r)| [r[0] as i32 + 1, r[1] as i32 + 1])
            .collect();
        slots.sort_unstable();
        let mut hits = Vec::new();
        for point in case["points"].as_array().ok_or("points")? {
            let x = point[0].as_f64().ok_or("point x")? as f32;
            let y = point[1].as_f64().ok_or("point y")? as f32;
            let hit = self.slot_at(&batch, x, y).map(|slot| {
                let rect = layout
                    .slots
                    .iter()
                    .find(|(s, _)| *s == slot)
                    .map(|(_, r)| *r);
                rect.map(|r| serde_json::json!([r[0] as i32 + 1, r[1] as i32 + 1]))
            });
            hits.push(hit.flatten().unwrap_or(Value::Null));
        }
        Ok(serde_json::json!({
            "gui": [gui.scale, gui.size[0], gui.size[1]],
            "image": [image.left, image.top, image.width, image.height],
            "origin": [ox as i32, oy as i32],
            "slots": slots,
            "hits": hits,
        }))
    }
}
