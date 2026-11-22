//! Game modes beyond survival and creative (`GameType`): adventure players
//! cannot build, spectators fly through blocks and only watch.
use super::{Game, Screen};
use horizon_client::{collisions::SceneCollisions, command::args::GameMode, items::ItemStack};
use horizon_world::world_movement::in_range_of_ground;

pub(super) fn mode_from_id(id: i32) -> GameMode {
    GameMode::from_id(id).unwrap_or(GameMode::Survival)
}
/// `ChunkMap.skipPlayer`: a player is skipped when `player.isSpectator() &&
/// !level.getGameRules().getBoolean(SPECTATORS_GENERATE_CHUNKS)`. A skipped
/// player holds no distance-manager ticket and streams no chunks around
/// itself, but still receives the chunks other players load (chunk sending
/// is untouched).
pub(super) fn loads_chunks(spectators_generate_chunks: bool, spectator: bool) -> bool {
    !spectator || spectators_generate_chunks
}

/// Fail closed at the new projectile caller when the existing matcher cannot
/// evaluate an entire conjunct. This does not change held-item interpretation.
pub(super) fn projectile_predicate_supported(tag: &horizon_core::nbt::Tag) -> bool {
    use horizon_core::nbt::Tag;
    let supported = |p: &Tag| {
        matches!(p, Tag::Compound(_))
            && ["nbt", "components", "predicates"]
                .iter()
                .all(|key| p.get(key).is_none())
            && match p.get("blocks") {
                None | Some(Tag::String(_)) => true,
                Some(Tag::List(ids)) => ids.iter().all(|id| matches!(id, Tag::String(_))),
                _ => false,
            }
            && match p.get("state") {
                None => true,
                Some(Tag::Compound(properties)) => properties.values().all(|value| match value {
                    Tag::String(_) => true,
                    Tag::Compound(_) => ["min", "max"].iter().all(|key| {
                        value
                            .get(key)
                            .is_none_or(|bound| matches!(bound, Tag::String(_)))
                    }),
                    _ => false,
                }),
                _ => false,
            }
    };
    match tag {
        Tag::Compound(_) => supported(tag),
        Tag::List(predicates) => !predicates.is_empty() && predicates.iter().all(supported),
        _ => false,
    }
}
/// `InventoryScreen.containerTick`: a survival inventory left open when the
/// player gains infinite materials becomes the creative one, so the player
/// stays in a menu. `CreativeModeInventoryScreen.containerTick` is the way
/// back: without infinite materials, in any mode (spectator included), the
/// creative screen is replaced by the survival one. Vanilla has no other
/// branch: a spectator's open survival inventory stays open.
fn inventory_after_mode(screen: Screen, creative: bool) -> Screen {
    match screen {
        Screen::Inventory if creative => Screen::Creative,
        Screen::Creative if !creative => Screen::Inventory,
        _ => screen,
    }
}

impl Game<'_> {
    /// `InventoryScreen.containerTick` and `CreativeModeInventoryScreen.containerTick`
    /// on a mode change: the inventory the player has open follows the mode.
    fn mode_changed_under_inventory(&mut self) {
        let from = self.screen;
        match inventory_after_mode(from, self.creative) {
            Screen::Creative => {
                if let Some(c) = self.carried.take() {
                    let _ = self.add_stack_to_inventory(c);
                }
                self.screen = Screen::Creative;
                self.refresh_creative_tabs();
                self.creative_screen.reopen();
                self.refresh_saved_hotbars();
            }
            // `ItemPickerMenu` and the new `InventoryScreen` share
            // `player.inventoryMenu`, and `removed` does nothing on a client
            // player: the carried stack stays on the cursor.
            next if next != from => self.set_screen(next),
            _ => {}
        }
    }

    pub(in crate::game) fn spectator(&self) -> bool {
        self.mode == GameMode::Spectator
    }
    /// `Abilities.mayBuild`.
    pub(super) fn may_build(&self) -> bool {
        self.acting_abilities.map_or(
            matches!(self.mode, GameMode::Survival | GameMode::Creative),
            |a| a.may_build,
        )
    }
    /// `Abilities.instabuild`. A client and the in-frame world derive it from the mode.
    pub(super) fn instabuild(&self) -> bool {
        self.acting_abilities
            .map_or(self.creative, |a| a.instabuild)
    }
    pub(super) fn mode_id(&self) -> i32 {
        self.mode.id()
    }
    pub(super) fn set_mode_id(&mut self, id: i32) {
        self.set_mode(mode_from_id(id));
    }
    /// `ServerPlayer.setGameMode` and `GameType.updatePlayerAbilities`.
    pub(super) fn set_mode(&mut self, mode: GameMode) {
        let was_spectator = self.spectator();
        let changed = mode != self.mode;
        // MultiPlayerGameMode.setLocalMode: the mode it leaves.
        if changed {
            self.previous_mode = Some(self.mode);
        }
        self.mode = mode;
        self.creative = mode == GameMode::Creative;
        // GameType.updatePlayerAbilities applies at once.
        if let Some(a) = self.acting_abilities.as_mut() {
            a.update_for_mode(mode.id());
        }
        if let Some(p) = self.player.as_mut() {
            p.invulnerable = mode == GameMode::Creative || mode == GameMode::Spectator;
            p.may_fly = p.invulnerable;
        }
        if mode == GameMode::Spectator {
            self.mining = None;
            self.stop_active_item_use();
            self.left = false;
            self.dismount_for_spectator();
            if let Some(p) = self.player.as_mut() {
                p.flying = true;
                p.spectator = true;
            }
        } else if let Some(p) = self.player.as_mut() {
            p.spectator = false;
            if was_spectator {
                p.flying_speed = 0.05;
            }
            if !self.creative {
                p.flying = false;
            } else if was_spectator {
                p.flying = true;
            }
        }
        // ServerPlayerGameMode.changeGameModeForPlayer: a player left flying
        // by the change lands when the ground is within a block.
        let flying = self
            .player
            .as_ref()
            .filter(|p| p.flying)
            .map(|p| p.bounds());
        if changed
            && mode != GameMode::Spectator
            && let Some(bounds) = flying
        {
            let mut scene = SceneCollisions {
                world: &self.world,
                binding: self.collision,
                subject: None,
            };
            if in_range_of_ground(&mut scene, bounds)
                && let Some(p) = self.player.as_mut()
            {
                p.flying = false;
                if let Some(a) = self.acting_abilities.as_mut() {
                    a.flying = false;
                }
            }
        }
        // InventoryScreen.containerTick: the open inventory follows the mode. Here, not in
        // the command, so the window hears it too: it changes mode through the server's
        // game event (`set_mode_id`), never through `set_game_mode`.
        self.mode_changed_under_inventory();
    }
}
impl Game<'_> {
    /// `ItemStack.canBreakBlockInAdventureMode` / `canPlaceOnBlockInAdventureMode`
    /// for the main-hand stack: some predicate of its `can_break` or
    /// `can_place_on` component (`AdventureModePredicate`: one block predicate
    /// or a list) matches the block at `pos`, as `BlockPredicate.matches(BlockInWorld)`
    /// tests it: the state, then `nbt` against the block entity's
    /// `saveWithFullMetadata` (no block entity fails). `components` is not
    /// looked at on this path.
    fn adventure_predicate(&self, component: &str, pos: [i32; 3]) -> bool {
        self.held()
            .is_some_and(|stack| self.stack_adventure_predicate(stack, component, pos))
    }
    /// `adventure_predicate` for any `stack`, not only the main hand's.
    fn stack_adventure_predicate(&self, stack: &ItemStack, component: &str, pos: [i32; 3]) -> bool {
        let Some(tag) = stack.data().and_then(|d| d.other(component)) else {
            return false;
        };
        self.adventure_predicate_at(tag, pos)
    }
    /// The existing supported block/state portion of `AdventureModePredicate.test`.
    /// The held-item wrapper and the projectile caller supply their own live value.
    pub(super) fn adventure_predicate_at(
        &self,
        tag: &horizon_core::nbt::Tag,
        pos: [i32; 3],
    ) -> bool {
        use horizon_core::nbt::Tag;
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let name = &self.res.facts().get(state).name;
        let properties = self
            .pack
            .assets
            .registry
            .state(state)
            .map(|s| &s.properties);
        let matches = |p: &Tag| {
            let block_ok = match p.get("blocks") {
                None => true,
                Some(Tag::String(id)) => self.block_in_holder_set(&id.to_string_lossy(), name),
                Some(Tag::List(ids)) => ids.iter().any(|id| match id {
                    Tag::String(id) => self.block_in_holder_set(&id.to_string_lossy(), name),
                    _ => false,
                }),
                Some(_) => false,
            };
            // StatePropertiesPredicate: every property exact, or in a min/max range.
            let state_ok = match p.get("state") {
                None => true,
                Some(Tag::Compound(want)) => want.iter().all(|(key, matcher)| {
                    let Some(have) = properties.and_then(|p| p.get(&key.to_string_lossy())) else {
                        return false;
                    };
                    match matcher {
                        Tag::String(value) => *have == value.to_string_lossy(),
                        Tag::Compound(_) => {
                            let bound = |k: &str| match matcher.get(k) {
                                Some(Tag::String(v)) => Some(v.to_string_lossy()),
                                _ => None,
                            };
                            let in_range = |have: &str, v: Option<String>, above: bool| {
                                v.is_none_or(|v| match (have.parse::<i64>(), v.parse::<i64>()) {
                                    (Ok(h), Ok(b)) if above => h >= b,
                                    (Ok(h), Ok(b)) => h <= b,
                                    _ => false,
                                })
                            };
                            in_range(have, bound("min"), true)
                                && in_range(have, bound("max"), false)
                        }
                        _ => false,
                    }
                }),
                Some(_) => false,
            };
            // NbtPredicate (TagParser.LENIENT_CODEC: SNBT or a compound),
            // NbtUtils.compareNbt(pattern, data, true).
            let nbt_ok = || match p.get("nbt") {
                None => true,
                Some(Tag::String(snbt)) => horizon_core::nbt::parse_snbt(&snbt.to_string_lossy())
                    .is_ok_and(|(pattern, _)| {
                        horizon_client::nbt_path::compare_nbt(
                            &pattern,
                            self.block_entity_nbt(pos).as_ref(),
                        )
                    }),
                Some(pattern @ Tag::Compound(_)) => horizon_client::nbt_path::compare_nbt(
                    pattern,
                    self.block_entity_nbt(pos).as_ref(),
                ),
                Some(_) => false,
            };
            block_ok && state_ok && nbt_ok()
        };
        match tag {
            Tag::List(list) => list.iter().any(matches),
            single => matches(single),
        }
    }
    /// A `HolderSet<Block>` entry: a block id or a `#tag`.
    fn block_in_holder_set(&self, entry: &str, block: &str) -> bool {
        let namespaced = |id: &str| {
            if id.contains(':') {
                id.to_owned()
            } else {
                format!("minecraft:{id}")
            }
        };
        match entry.strip_prefix('#') {
            Some(tag) => self.res.items().block_in_tag(&namespaced(tag), block),
            None => namespaced(entry) == block,
        }
    }
    /// `Player.blockActionRestricted` for the block at `pos`: spectators never
    /// act on blocks, adventure players only with a main-hand item whose
    /// `can_break` matches it.
    pub(super) fn block_action_restricted(&self, pos: [i32; 3]) -> bool {
        if self.spectator() {
            return true;
        }
        !self.may_build() && !self.can_break_in_adventure(pos)
    }
    /// `ItemStack.canBreakBlockInAdventureMode`: an adventure player may break,
    /// or prime TNT at, only a block their held stack's `can_break` matches.
    pub(super) fn can_break_in_adventure(&self, pos: [i32; 3]) -> bool {
        self.adventure_predicate("minecraft:can_break", pos)
    }
    /// `ItemStack.useOn` in adventure mode: the used item's `can_place_on`
    /// matches the clicked block (`Player.mayUseItemAt`).
    pub(super) fn can_place_on_in_adventure(&self, clicked: [i32; 3]) -> bool {
        self.adventure_predicate("minecraft:can_place_on", clicked)
    }
    /// `Player.mayUseItemAt(pos, face, stack)` for a player who `may_build`
    /// or not: builders always may, anyone else only when `stack`'s
    /// `can_place_on` matches the block clicked, the one `pos` lies against.
    pub(super) fn may_use_item_at(
        &self,
        may_build: bool,
        pos: [i32; 3],
        face: horizon_core::geometry::Direction,
        stack: &ItemStack,
    ) -> bool {
        may_build
            || self.stack_adventure_predicate(
                stack,
                "minecraft:can_place_on",
                relative(pos, face.opposite()),
            )
    }
    /// `ItemStack.canPlaceOnBlockInAdventureMode` of the stack a use names.
    pub(super) fn stack_can_place_on_in_adventure(
        &self,
        stack: &ItemStack,
        clicked: [i32; 3],
    ) -> bool {
        self.stack_adventure_predicate(stack, "minecraft:can_place_on", clicked)
    }
    /// `ItemStack.useOn`'s gate before any item's own `useOn`: a player who
    /// may build, or an adventure player whose held item's `can_place_on`
    /// matches the clicked block.
    pub(super) fn may_use_on(&self, clicked: [i32; 3]) -> bool {
        self.may_build() || self.can_place_on_in_adventure(clicked)
    }
    /// `ServerPlayer.setGameMode(SPECTATOR)`: `stopRiding`.
    fn dismount_for_spectator(&mut self) {
        let Some(id) = self.riding else {
            return;
        };
        if let Some(i) = self.boats.iter().position(|b| b.id == id) {
            self.dismount(i);
        } else if let Some(i) = self.carts.iter().position(|c| c.id == id) {
            self.dismount_cart(i);
        } else {
            self.riding = None;
        }
    }
}

// BlockPos.relative uses Java's wrapping integer arithmetic, including for mod input.
fn relative(pos: [i32; 3], face: horizon_core::geometry::Direction) -> [i32; 3] {
    let [dx, dy, dz] = face.normal();
    [
        pos[0].wrapping_add(dx),
        pos[1].wrapping_add(dy),
        pos[2].wrapping_add(dz),
    ]
}

#[cfg(test)]
#[path = "tests/adventure_gate_tests.rs"]
mod adventure_gate_tests;

#[cfg(test)]
mod tests {
    #[test]
    fn face_offsets_wrap_at_integer_boundaries() {
        use horizon_core::geometry::Direction::*;
        for (face, axis, positive) in [
            (East, 0, true),
            (West, 0, false),
            (Up, 1, true),
            (Down, 1, false),
            (South, 2, true),
            (North, 2, false),
        ] {
            for boundary in [i32::MIN, i32::MAX] {
                let mut pos = [5; 3];
                pos[axis] = boundary;
                let mut expected = pos;
                expected[axis] = match (positive, boundary) {
                    (true, i32::MAX) => i32::MIN,
                    (false, i32::MIN) => i32::MAX,
                    (true, _) => i32::MIN + 1,
                    (false, _) => i32::MAX - 1,
                };
                assert_eq!(
                    super::relative(pos, face),
                    expected,
                    "{face:?} at {boundary}"
                );
            }
        }
    }

    use super::{Screen, inventory_after_mode, loads_chunks, projectile_predicate_supported};

    // Added after the frozen old Game proof: unsupported conjuncts must not
    // become permission from their otherwise matching blocks portion.
    #[test]
    fn projectile_predicates_require_a_complete_supported_match() {
        for extra in [
            "nbt:{marker:1}",
            "components:{}",
            "predicates:{}",
            "state:{age:{min:1}}",
        ] {
            let text = format!("{{blocks:\"minecraft:chorus_flower\",{extra}}}");
            let (tag, _) = horizon_core::nbt::parse_snbt(&text).unwrap();
            assert!(!projectile_predicate_supported(&tag), "{text}");
        }
        for text in ["7", "[]", "{blocks:[7]}"] {
            let (tag, _) = horizon_core::nbt::parse_snbt(text).unwrap();
            assert!(!projectile_predicate_supported(&tag), "{text}");
        }
        let (tag, _) = horizon_core::nbt::parse_snbt(
            "{blocks:\"minecraft:chorus_flower\",state:{age:{min:\"0\",max:\"5\"}}}",
        )
        .unwrap();
        assert!(projectile_predicate_supported(&tag));
    }

    /// `InventoryScreen.containerTick`: the survival inventory becomes the
    /// creative one when the player gets infinite materials, and any other
    /// screen is left where it is.
    #[test]
    fn a_survival_inventory_becomes_the_creative_one() {
        assert!(inventory_after_mode(Screen::Inventory, true) == Screen::Creative);
        let stays = inventory_after_mode(Screen::Inventory, false);
        assert!(stays == Screen::Inventory, "survival keeps its inventory");
        // Vanilla's only branch is `hasInfiniteMaterials`: a spectator's open
        // inventory stays open.
        // `CreativeModeInventoryScreen.containerTick`: no infinite materials
        // replaces the creative screen with the survival one, spectator
        // included; infinite materials keep it.
        assert!(inventory_after_mode(Screen::Creative, false) == Screen::Inventory);
        assert!(inventory_after_mode(Screen::Creative, true) == Screen::Creative);
        for screen in [
            Screen::Playing,
            Screen::Paused,
            Screen::Stats,
            Screen::Advancements,
        ] {
            assert!(inventory_after_mode(screen, true) == screen);
        }
    }

    /// `ChunkMap.skipPlayer`: only a spectator with `spectators_generate_chunks`
    /// off stops loading chunks around itself.
    #[test]
    fn a_skipped_spectator_loads_no_chunks() {
        assert!(!loads_chunks(false, true), "spectator, rule off");
        assert!(loads_chunks(true, true), "spectator, rule on");
        assert!(
            loads_chunks(false, false),
            "survival, creative or adventure"
        );
        assert!(
            loads_chunks(true, false),
            "survival, creative or adventure, rule on"
        );
    }

    /// `AbstractContainerMenu.removed` does nothing on a client player and
    /// both screens share `player.inventoryMenu`: leaving creative keeps the
    /// carried stack on the cursor.
    #[test]
    #[ignore = "daily"]
    fn leaving_creative_keeps_the_carried_stack() {
        use horizon_client::{command::args::GameMode, items::ItemStack};
        let mut game = crate::game::native_machines::tests::server_game();
        game.set_mode(GameMode::Creative);
        game.screen = Screen::Creative;
        game.carried = Some(ItemStack {
            item: "minecraft:stone".into(),
            count: 3,
            damage: 0,
            extra: None,
        });
        game.set_mode(GameMode::Survival);
        assert!(game.screen == Screen::Inventory);
        let kept = game.carried.as_ref().expect("the cursor stack is kept");
        assert_eq!((&*kept.item, kept.count), ("minecraft:stone", 3));
    }

    /// `CreativeModeInventoryScreen.containerTick` is the other way back: the
    /// tick swaps to the survival screen and the cursor stack stays.
    #[test]
    #[ignore = "daily"]
    fn the_tick_swap_out_of_creative_keeps_the_carried_stack() {
        use horizon_client::items::ItemStack;
        let mut game = crate::game::native_machines::tests::server_game();
        // The window's lead runs on a client's own game only.
        game.options.execution_role = crate::game::execution_role::ExecutionRole::Client;
        game.screen = Screen::Creative;
        game.creative = false;
        game.carried = Some(ItemStack {
            item: "minecraft:stone".into(),
            count: 3,
            damage: 0,
            extra: None,
        });
        let _ = game.tick();
        assert!(game.screen == Screen::Inventory);
        assert_eq!(game.carried.as_ref().map(|c| c.count), Some(3));
    }
}
