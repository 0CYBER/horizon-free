//! The crafter (`CrafterBlock`, `CrafterBlockEntity`, `CrafterMenu`,
//! `CrafterScreen`): redstone-triggered crafting of its 3x3 grid, ejecting
//! the result and remainders into the container in front or as items, the
//! crafting animation ticks, and the menu with its toggleable slots.
use super::*;
use horizon_client::{
    advancements as adv,
    containers::Crafter,
    hud::{HudBatch, rgba},
    menu::{Maps as _, Menu},
};
use horizon_protocol::packets::play::{self, Serverbound as S};
use serde_json::Value;

/// `CrafterBlock`'s crafting animation length.
const CRAFTING_TICKS: i32 = 6;
const RESULT_SLOT: usize = 309;

fn center(pos: Pos) -> [f64; 3] {
    [
        f64::from(pos.0) + 0.5,
        f64::from(pos.1) + 0.5,
        f64::from(pos.2) + 0.5,
    ]
}
/// The UUIDs of the players `CrafterRecipeCraftedTrigger` reaches for the
/// crafter at `pos` (`getEntitiesOfClass(ServerPlayer.class, ...)`): each of
/// the level's `players` that is not a spectator and whose box meets the
/// 17-block box around the crafter's centre (8.5 from the centre to a face:
/// the player's own width and height reach into it, not its eye point).
/// `AABB.intersects` is strict: a box that only touches a face is out.
fn crafter_witnesses(
    pos: Pos,
    players: impl IntoIterator<Item = crate::game::natural_spawning::LevelPlayer>,
) -> Vec<u128> {
    let c = center(pos);
    players
        .into_iter()
        .filter(|p| {
            let (lo, hi) = (p.bounds.min_position(), p.bounds.max_position());
            !p.spectator
                && (lo.x < c[0] + 8.5 && hi.x > c[0] - 8.5)
                && (lo.y < c[1] + 8.5 && hi.y > c[1] - 8.5)
                && (lo.z < c[2] + 8.5 && hi.z > c[2] - 8.5)
        })
        .map(|p| p.uuid)
        .collect()
}
/// The connections of the `guests` that `near` names, each to take the
/// trigger in their own turn; a dead one is left out. A guest whose client is
/// still loading (placed, no `PlayerLoaded` yet) counts: `getEntitiesOfClass`
/// finds every `ServerPlayer` in the level, where `for_each_player` waits for
/// that packet.
fn witness_turns<'g>(
    near: &[u128],
    guests: impl IntoIterator<Item = &'g horizon_client::lan::guest::Guest>,
) -> Vec<u32> {
    guests
        .into_iter()
        .filter(|g| !g.dead && near.contains(&g.uuid))
        .map(|g| g.conn)
        .collect()
}
/// Whether an advancement condition's `recipes` entry names the recipe.
fn recipe_matches(condition: &Value, id: &str) -> bool {
    match condition {
        Value::String(s) => s == id,
        Value::Array(list) => list.iter().any(|v| v.as_str() == Some(id)),
        _ => true,
    }
}

impl Game<'_> {
    /// `Level.setBlock` of one boolean property of the block at `pos`.
    pub(crate) fn set_bool_prop(&mut self, pos: Pos, key: &str, value: bool, flags: i32) {
        let registry = self.pack.assets.registry;
        let old = self.world.block(pos.0, pos.1, pos.2);
        let Some(mut state) = registry.state(old).cloned() else {
            return;
        };
        if !state.properties.contains_key(key) {
            return;
        }
        state.properties.insert(key.to_owned(), value.to_string());
        let Ok(id) = registry.resolve(&state) else {
            return;
        };
        self.with_level(|b, level, ticks| {
            b.set_block(level, ticks, BlockPos::new(pos.0, pos.1, pos.2), id, flags)
        });
        let _ = self.process_level_events();
    }
    fn crafter_at(&self, pos: Pos) -> Option<&Crafter> {
        match self.world.block_entities.get(&pos)? {
            BlockEntity::Crafter(c) => Some(c),
            _ => None,
        }
    }
    /// `CrafterBlockEntity.serverTick` of the crafter at `pos`, and the
    /// powered flag following the block state.
    pub(in crate::game) fn tick_crafter(&mut self, pos: Pos) {
        let state = self.world.block(pos.0, pos.1, pos.2);
        let triggered = self.containers().prop(state, "triggered") == Some("true");
        let mut finished = false;
        if let Some(BlockEntity::Crafter(c)) = self.world.block_entities.get_mut(&pos) {
            // CrafterBlock.neighborChanged sets the flag with the state; a
            // loaded crafter keeps its saved flag until then.
            if c.state_triggered.is_some_and(|t| t != triggered) {
                c.triggered = i32::from(triggered);
            }
            c.state_triggered = Some(triggered);
            let remaining = c.crafting_ticks - 1;
            if remaining >= 0 {
                c.crafting_ticks = remaining;
                finished = remaining == 0;
            }
        }
        if finished {
            self.set_bool_prop(
                pos,
                "crafting",
                false,
                horizon_world::level::flags::UPDATE_ALL,
            );
        }
    }
    /// `CrafterBlock.dispenseFrom` (the scheduled tick after a redstone pulse).
    pub(super) fn crafter_dispense(&mut self, pos: BlockPos, state: BlockStateId) -> Result<()> {
        let key = (pos.x, pos.y, pos.z);
        let Some(crafter) = self.crafter_at(key) else {
            return Ok(());
        };
        let grid: Vec<Option<ItemStack>> = crafter.items.to_vec();
        // Query active world recipes with this level's saved-map context.
        let maps = self.level_maps(grid.iter().flatten());
        let found = self
            .recipes()
            .find_with(&self.loop_context, &grid, 3, &|id| {
                maps.state(id).map(|(_, scale)| scale)
            })
            .map(|(name, result)| (name.to_owned(), result))
            .filter(|(_, result)| result.count > 0);
        let Some((recipe, mut result)) = found else {
            self.dispenser_event(1050, key, 0);
            return Ok(());
        };
        let Some(facing) = self
            .containers()
            .prop(state, "orientation")
            .and_then(|o| o.split('_').next())
            .and_then(dir_from_name)
        else {
            return Ok(());
        };
        if let Some(BlockEntity::Crafter(c)) = self.world.block_entities.get_mut(&key) {
            c.crafting_ticks = CRAFTING_TICKS;
        }
        self.set_bool_prop(
            key,
            "crafting",
            true,
            horizon_world::level::flags::UPDATE_CLIENTS,
        );
        // ItemStack.onCraftedBySystem invokes MapItem.onCraftedPostProcess
        // before dispensing, registering the derived map under its new id.
        Menu::map_crafted(Some(&maps), &mut result);
        self.register_made_maps(maps);
        // The remainders come from the grid as it was before anything moved.
        let remainders = self
            .recipes()
            .remainders(&recipe, &grid, 3, &|s| self.crafting_remainder(s));
        self.crafter_eject(key, facing, result, &recipe);
        for rest in remainders.into_iter().flatten() {
            self.crafter_eject(key, facing, rest, &recipe);
        }
        // One of each input used, from the slots that held one: a part in front may have handed
        // the output on into an empty slot of this grid.
        if let Some(BlockEntity::Crafter(c)) = self.world.block_entities.get_mut(&key) {
            for (slot, _) in c
                .items
                .iter_mut()
                .zip(&grid)
                .filter(|(_, used)| used.is_some())
            {
                if let Some(s) = slot {
                    s.count -= 1;
                    if s.count == 0 {
                        *slot = None;
                    }
                }
            }
        }
        self.container_changed(key);
        self.native_crafter_craft([key.0, key.1, key.2], &recipe);
        Ok(())
    }
    /// `CrafterBlock.dispenseItem`: into the container in front, else out as an item.
    fn crafter_eject(&mut self, pos: Pos, facing: Direction, stack: ItemStack, recipe: &str) {
        let target = offset(pos, facing);
        let mut rest = stack;
        let side = facing.opposite();
        // `HopperBlockEntity.getBlockContainer`: a decorated pot keeps its slot in
        // the chunk's saved tag, which `view` does not read.
        if self.item_part_at(target, side).is_none()
            && self.containers().view(target, false).is_none()
            && let Some(pot) = self.containers().hopper_insertion_view(target)
        {
            self.crafter_insert(&pot, &mut rest, side, false);
            if rest.count == 0 {
                return;
            }
            self.spawn_dispensed(pos, facing, rest, 6);
            self.crafter_crafted_notice(pos, facing, recipe);
            return;
        }
        self.with_container_at(target, side, |game, into| match into {
            ContainerTarget::Part(part) => {
                // `insertCrafterOutput`: the handler takes what it will; the rest is dispensed.
                let at = [target.0, target.1, target.2];
                rest.count -= game.put_into_part(&part, at, &rest, |_, _| Some(()));
            }
            ContainerTarget::View(into) => {
                let into_is_crafter = game.crafter_at(target).is_some();
                game.crafter_insert(into, &mut rest, side, into_is_crafter);
            }
        });
        if rest.count == 0 {
            return;
        }
        self.spawn_dispensed(pos, facing, rest, 6);
        self.crafter_crafted_notice(pos, facing, recipe);
    }
    /// The trigger and the sound and particle events after an ejected result.
    fn crafter_crafted_notice(&mut self, pos: Pos, facing: Direction, recipe: &str) {
        // The crafter crafts in its scheduled tick, outside any player's turn,
        // and a server has no player of its own: each of the level's players
        // the trigger reaches hears it in their own turn.
        let near = crafter_witnesses(pos, self.level_players_iter());
        if !near.is_empty() {
            let id = horizon_client::crafting::qualified(recipe);
            let me = horizon_client::lan::playerdata::ints_uuid(self.acting_uuid());
            if self.player.is_some() && !self.lan_acting_fake && near.contains(&me) {
                self.trigger_crafter_recipe_crafted(&id);
            }
            let guests = self.lan.iter().flat_map(|h| h.hub.guests.iter());
            for conn in witness_turns(&near, guests) {
                self.as_guest(conn, |g| g.trigger_crafter_recipe_crafted(&id));
            }
        }
        self.dispenser_event(1049, pos, 0);
        self.dispenser_event(2010, pos, facing as i32);
    }
    /// `CrafterRecipeCraftedTrigger` for the player in place.
    fn trigger_crafter_recipe_crafted(&mut self, id: &str) {
        let me = self.player_entity_for("minecraft:crafter_recipe_crafted");
        self.trigger("minecraft:crafter_recipe_crafted", move |c, w| {
            let ctx = adv::Entities {
                this: Some(&me),
                origin: me.position,
                tool: None,
                block_pos: None,
            };
            adv::conditions(&c["player"], &ctx, w) && recipe_matches(&c["recipes"], id)
        });
    }
    /// The two insert loops of `dispenseItem`: one item at a time when the
    /// container holds fewer than the stack (`Container.getMaxStackSize`) or is
    /// a crafter, else the whole stack; what is left stays in `rest`.
    fn crafter_insert(
        &mut self,
        into: &View,
        rest: &mut ItemStack,
        face: Direction,
        into_is_crafter: bool,
    ) {
        let one_by_one = into_is_crafter || rest.count > self.containers().max_stack_in(into, rest);
        // One container instance serves the whole call, and a composter's
        // `InputContainer` is `changed` once it composted: it takes no more.
        let composter = matches!(into.kind, Kind::Composter(_));
        while rest.count > 0 {
            let before = rest.count;
            if one_by_one {
                let one = ItemStack {
                    count: 1,
                    ..rest.clone()
                };
                if self.add_item(None, into, one, Some(face)).is_some() {
                    break;
                }
                rest.count -= 1;
            } else {
                match self.add_item(None, into, rest.clone(), Some(face)) {
                    Some(left) => *rest = left,
                    None => rest.count = 0,
                }
                if rest.count == before {
                    break;
                }
            }
            if composter {
                break;
            }
        }
    }

    // ---- the menu ----

    /// The grid's current recipe result (`CrafterMenu.refreshRecipeResult`).
    pub(in crate::game) fn crafter_result(&self) -> Option<ItemStack> {
        let key = self.container?;
        let c = self.crafter_at(key)?;
        self.recipes().craft(&self.loop_context, &c.items, 3)
    }
    pub(in crate::game) fn crafter_open(&self) -> bool {
        matches!(self.container_kind(), Some(BlockEntity::Crafter(_)))
    }
    /// `CrafterMenu` slot layout: the grid at (26, 17), the result at (134, 35).
    pub(in crate::game) fn crafter_slot_position(&self, i: usize) -> Option<(f32, f32)> {
        if !self.crafter_open() {
            return None;
        }
        match i {
            300..309 => Some((
                26.0 + ((i - 300) % 3) as f32 * 18.0,
                17.0 + ((i - 300) / 3) as f32 * 18.0,
            )),
            RESULT_SLOT => Some((134.0, 35.0)),
            _ => None,
        }
    }
    /// `CrafterScreen`'s background, disabled slots and powered arrow (the
    /// verified `screen_layout::crafter`) and its centred title.
    pub(in crate::game) fn draw_crafter_screen(&self, batch: &mut HudBatch, ox: f32, oy: f32) {
        let Some(c) = self.container.and_then(|k| self.crafter_at(k)) else {
            return;
        };
        // CrafterScreen draws the arrow from `menu.isPowered()`: data slot 9, the int, is 1.
        let powered = c.is_triggered();
        let draws =
            horizon_client::screen_layout::crafter(&horizon_client::screen_layout::CrafterView {
                width: 176,
                height: 166,
                left: 0,
                top: 0,
                disabled: c.disabled,
                powered,
            });
        // The background goes first, then the labels, then the slot decorations.
        let (background, rest) = draws.split_at(2.min(draws.len()));
        self.paint_screen(batch, ox, oy, background, true);
        self.draw_screen_phase(batch, horizon_client::screen_draw::RenderPhase::Background);
        let atlas = &self.gui.atlas;
        let grey = rgba(64, 64, 64, 255);
        // A server's crafter menu has the title it was given.
        let title = self.remote_menu_title().unwrap_or_else(|| {
            horizon_client::text::Text::literal(
                self.container_title(self.translate_with_fallback("container.crafter", "Crafter")),
            )
            .resolve_legacy()
        });
        crate::game::screen_info::name_container(|| title.plain());
        let tw = atlas.font.width_text(&title);
        batch.text_styled(
            atlas,
            &title,
            ox + ((176.0 - tw) / 2.0).trunc(),
            oy + 6.0,
            grey,
            false,
        );
        let inventory = self.translate_with_fallback("container.inventory", "Inventory");
        batch.text(atlas, &inventory, ox + 8.0, oy + 72.0, grey, false);
        self.paint_screen(batch, ox, oy, rest, true);
    }
    /// `gui.togglable_slot` over an empty, enabled grid slot.
    pub(in crate::game) fn crafter_tooltip(&self, slot: usize) -> Option<String> {
        let c = self.container.and_then(|k| self.crafter_at(k))?;
        let i = slot.checked_sub(300).filter(|i| *i < 9)?;
        (self.carried.is_none() && c.items[i].is_none() && !c.disabled[i] && !self.spectator())
            .then(|| self.translate_with_fallback("gui.togglable_slot", "Click to disable slot"))
    }
    /// A click in the crafter menu: `CrafterMenu` on the verified menu engine.
    pub(in crate::game) fn crafter_click(&mut self, slot: usize, shift: bool, right: bool) -> bool {
        if !self.crafter_open() {
            return false;
        }
        let input = if shift {
            horizon_client::menu::Input::QuickMove
        } else {
            horizon_client::menu::Input::Pickup
        };
        self.click_open_inventory(Some(slot), i32::from(right), input);
        true
    }
    /// `CrafterScreen.slotClicked`: an empty grid slot is enabled by a click
    /// (or a number key with a hotbar item) when disabled, and disabled by a
    /// click with nothing carried; the menu then gets the click itself.
    pub(in crate::game) fn crafter_slot_clicked(
        &mut self,
        slot: usize,
        button: i32,
        input: horizon_client::menu::Input,
    ) {
        use horizon_client::menu::Input;
        if !self.crafter_open() || !(300..309).contains(&slot) || self.spectator() {
            return;
        }
        let i = slot - 300;
        let Some(c) = self.container.and_then(|k| self.crafter_at(k)) else {
            return;
        };
        if c.items[i].is_some() {
            return;
        }
        let disabled = c.disabled[i];
        let enable = match input {
            Input::Pickup if disabled => Some(true),
            Input::Pickup if self.carried.is_none() => Some(false),
            Input::Swap if disabled => usize::try_from(button)
                .ok()
                .and_then(|b| {
                    if b == 40 {
                        Some(&self.offhand)
                    } else {
                        self.inventory.get(b)
                    }
                })
                .is_some_and(Option::is_some)
                .then_some(true),
            _ => None,
        };
        let Some(enabled) = enable else {
            return;
        };
        // ServerboundContainerSlotStateChangedPacket: CrafterBlockEntity.setSlotState.
        self.menu_session(|menu, _, _| {
            menu.set_slot_state(i, enabled);
            Some(true)
        });
        if let Some((container, _)) = self.remote.as_ref().and_then(|r| r.menu.as_ref().cloned()) {
            self.send(S::ContainerSlotStateChanged(
                play::ContainerSlotStateChanged {
                    slot: i as i32,
                    container,
                    enabled,
                },
            ));
        }
        let pitch = if enabled { 1.0 } else { 0.75 };
        self.audio
            .play("minecraft:ui.button.click", None, 0.4, pitch);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stack(item: &str, count: u32) -> Option<ItemStack> {
        Some(ItemStack::new(item, count))
    }
    #[test]
    fn signal_counts_filled_and_disabled_slots() {
        let mut c = Crafter::default();
        assert_eq!(c.redstone_signal(), 0);
        c.items[0] = stack("minecraft:stick", 3);
        c.set_slot_state(4, false);
        assert_eq!(c.redstone_signal(), 2);
        // A filled slot cannot be disabled.
        c.set_slot_state(0, false);
        assert!(!c.disabled[0]);
        c.set_slot_state(4, true);
        assert_eq!(c.redstone_signal(), 1);
    }
    /// `CrafterBlockEntity.setSlotState`: a slot toggled in the menu lands on
    /// the block entity itself, whose `setChanged` marks the chunk.
    #[test]
    #[ignore = "daily"]
    fn a_slot_toggled_in_the_menu_lands_on_the_crafter() {
        use crate::game::game_tests::tests::server_game;
        let mut resources = None;
        let mut game = server_game(&mut resources);
        let key = (2, 4, 3);
        let state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:crafter")
            .unwrap();
        game.world.set(key.0, key.1, key.2, state).unwrap();
        let mut crafter = Crafter::default();
        crafter.set_slot_state(3, false);
        game.world
            .block_entities
            .insert(key, BlockEntity::Crafter(crafter));
        game.container = Some(key);
        game.world.modified.clear();

        game.crafter_slot_clicked(303, 0, horizon_client::menu::Input::Pickup);

        let Some(BlockEntity::Crafter(c)) = game.world.block_entities.get(&key) else {
            panic!("no crafter");
        };
        assert!(!c.disabled[3], "the click enabled the slot");
        assert!(
            game.world.modified.contains(&(0, 0)),
            "the toggled slot left the chunk unmarked"
        );
    }
    #[test]
    fn hoppers_fill_the_smaller_stacks_first() {
        let mut c = Crafter::default();
        let stick = ItemStack::new("minecraft:stick", 1);
        c.items[0] = stack("minecraft:stick", 2);
        c.items[1] = stack("minecraft:stick", 1);
        // A later empty slot counts as smaller: stacks level out front to back.
        assert!(!c.can_place(0, &stick, 64));
        assert!(!c.can_place(1, &stick, 64));
        assert!(c.can_place(2, &stick, 64));
        c.disabled[5] = true;
        assert!(!c.can_place(5, &stick, 64));
        // With every later slot at least as big, a slot accepts again.
        for i in 2..9 {
            c.items[i] = stack("minecraft:stick", 2);
        }
        assert!(c.can_place(1, &stick, 64));
        // A full stack accepts nothing.
        c.items[2] = stack("minecraft:stick", 64);
        assert!(!c.can_place(2, &stick, 64));
    }
    #[test]
    fn recipes_condition_accepts_a_name_or_a_list() {
        assert!(recipe_matches(
            &serde_json::json!("minecraft:crafter"),
            "minecraft:crafter"
        ));
        assert!(!recipe_matches(
            &serde_json::json!(["minecraft:stick"]),
            "minecraft:crafter"
        ));
        assert!(recipe_matches(&Value::Null, "minecraft:crafter"));
    }
    /// `CrafterRecipeCraftedTrigger` reaches every player whose box meets the
    /// 17-block box around the crafter. A server has no player of its own, so
    /// a player it serves standing 8 blocks off must be reached, or nobody
    /// there earns "Crafters Crafting Crafters". A spectator is not
    /// (`NO_SPECTATORS`), nor a box that only touches the far face; a box that
    /// reaches in is, though the feet are further out.
    #[test]
    fn a_crafter_reaches_the_players_a_server_serves() {
        use crate::game::natural_spawning::LevelPlayer;
        // A player's 0.6 wide box from `x` up.
        let served = |uuid: u128, x: f64, spectator: bool| LevelPlayer {
            uuid,
            position: Vec3::new(x + 0.3, 64.0, 0.5),
            bounds: Aabb::new(Vec3::new(x, 64.0, 0.2), Vec3::new(x + 0.6, 65.8, 0.8)),
            alive: true,
            creative: false,
            spectator,
        };
        // The crafter at (0, 64, 0): the box runs from -8 to 9 along x.
        let pos = (0, 64, 0);
        assert_eq!(crafter_witnesses(pos, [served(7, 8.25, false)]), vec![7]);
        let players = [
            served(1, 8.25, true),
            served(2, 9.0, false),
            served(3, -8.5, false),
        ];
        assert_eq!(crafter_witnesses(pos, players), vec![3]);
    }
    /// A served player whose client is still loading (placed, no
    /// `PlayerLoaded` yet) is in the level all the same: `getEntitiesOfClass`
    /// finds it, so the trigger reaches it in its own turn as it does a loaded
    /// one, or a craft in that window passes it by. A guest the box missed is
    /// not reached.
    #[test]
    fn a_crafter_reaches_a_served_player_whose_client_is_still_loading() {
        use crate::game::native_player_death::tests::new_guest;
        let mut playing = new_guest(1, 11);
        (playing.loaded, playing.loading) = (true, false);
        let mut loading = new_guest(2, 12);
        (loading.loaded, loading.loading) = (false, true);
        let guests = [playing, loading, new_guest(3, 13)];
        assert_eq!(witness_turns(&[11, 12], &guests), vec![1, 2]);
    }
    /// `CrafterBlock.dispenseItem` ejects into the container entity in front of
    /// it (`HopperBlockEntity.getContainerAt`'s `getEntityContainer`), not
    /// beside it: a chest or hopper minecart, or a chest boat, is filled.
    #[test]
    #[ignore = "daily"]
    fn a_crafter_fills_the_cart_in_front_of_it() {
        use crate::game::boats::Boat;
        use crate::game::game_tests::tests::server_game;
        use crate::game::minecarts::{CartKind, Minecart};
        use horizon_core::geometry::Vec3;
        // One game for the three vehicles: loading the resources is the slow part.
        let mut resources = None;
        let mut game = server_game(&mut resources);
        let at = Vec3::new(9.5, 64.0, 8.5);
        for kind in [CartKind::Chest, CartKind::Hopper] {
            game.carts.clear();
            game.boats.clear();
            game.carts.push(Minecart::new(1, kind, at));
            game.crafter_eject(
                (8, 64, 8),
                Direction::East,
                ItemStack::new("minecraft:pumpkin_pie", 3),
                "minecraft:pumpkin_pie",
            );
            let cart = game.carts.first().expect("the cart is still there");
            assert_eq!(cart.items[0].as_ref().map(|s| s.count), Some(3));
            assert!(game.dropped.is_empty(), "nothing is dropped beside it");
        }
        game.carts.clear();
        game.boats
            .push(Boat::new(1, Arc::from("oak"), at, 0.0).with_chest());
        game.crafter_eject(
            (8, 64, 8),
            Direction::East,
            ItemStack::new("minecraft:pumpkin_pie", 3),
            "minecraft:pumpkin_pie",
        );
        let boat = game.boats.first().expect("the boat is still there");
        assert_eq!(boat.items[0].as_ref().map(|s| s.count), Some(3));
        assert!(game.dropped.is_empty(), "nothing is dropped beside it");
    }
    /// One transfer into a composter is one item, however big the stack: the
    /// `InputContainer` is `changed` after it composts, and its
    /// `getMaxStackSize` of 1 keeps the crafter from pushing the stack whole.
    #[test]
    #[ignore = "daily"]
    fn one_transfer_to_a_composter_adds_one_layer_and_eats_one_item() {
        use crate::game::game_tests::tests::server_game;
        let mut resources = None;
        let mut game = server_game(&mut resources);
        let pos = (8, 63, 8);
        let state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:composter")
            .unwrap();
        game.world.set(pos.0, pos.1, pos.2, state).unwrap();
        // Pumpkin pie composts one layer always (`compostable/always_add_one`).
        game.crafter_eject(
            (8, 64, 8),
            Direction::Down,
            ItemStack::new("minecraft:pumpkin_pie", 9),
            "minecraft:pumpkin_pie",
        );
        assert_eq!(game.composter_level([pos.0, pos.1, pos.2]), Some(1));
        let dropped: Vec<(String, u32)> = game
            .dropped
            .iter()
            .map(|d| (d.stack.item.to_string(), d.stack.count))
            .collect();
        assert_eq!(dropped, [("minecraft:pumpkin_pie".to_owned(), 8)]);
    }
}

#[cfg(test)]
mod part_remainder_tests {
    use super::*;
    struct Milk;
    #[mod_api::horizon_part]
    impl mod_api::ItemPart for Milk {
        fn crafting_remainder(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::CraftingRemainderEvent<'_>,
        ) {
            *event.remainder = None;
        }
    }
    struct Remainders;
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Remainders {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            let milk = r.item_key("minecraft:milk_bucket");
            r.item_part(milk, Milk);
        }
    }
    #[test]
    #[ignore = "daily"]
    fn crafter_dispense_observes_part_replaced_remainders() {
        let mods: mod_api::Mods = vec![mod_api::ModEntry::new("milk", Box::new(Remainders))].into();
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                crate::game::native_entity_refs::tests::resolve_parts(game).unwrap();
                let state = game
                    .block_state_id("minecraft:crafter[orientation=north_up]")
                    .unwrap();
                let pos = BlockPos::new(0, 100, 0);
                let grid = [
                    "milk_bucket",
                    "milk_bucket",
                    "milk_bucket",
                    "sugar",
                    "egg",
                    "sugar",
                    "wheat",
                    "wheat",
                    "wheat",
                ];
                let mut crafter = Crafter::default();
                for (slot, item) in crafter.items.iter_mut().zip(grid) {
                    *slot = Some(ItemStack::new(&format!("minecraft:{item}"), 1));
                }
                game.world
                    .block_entities
                    .insert((0, 100, 0), BlockEntity::Crafter(crafter));
                game.dropped.clear();
                game.crafter_dispense(pos, state).unwrap();
                let output: Vec<_> = game
                    .dropped
                    .iter()
                    .map(|e| (e.stack.item.to_string(), e.stack.count))
                    .collect();
                assert_eq!(
                    output,
                    [("minecraft:cake".to_owned(), 1)],
                    "the Part suppresses all three data-only buckets"
                );
            });
        });
    }
}
