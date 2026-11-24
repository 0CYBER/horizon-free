//! Block-state facts a mod reads without a position (light, friction, solid render, the
//! correct-tool rule) and a player's harvest rule: the game's own tables (`StateFacts`, the
//! world's `Items::correct_for_drops` as the dig reads it).
use super::Game;
use super::native_server::PlayerRead;
use horizon_client::items::ItemStack;
use horizon_world::registry::BlockStateId;
use mod_api::PlayerRef;

impl Game<'_> {
    /// `BlockState.getLightEmission()`. None for an unknown state.
    pub(super) fn block_state_light_emission(&self, state: BlockStateId) -> Option<u8> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.facts().get(state).emission)
    }
    /// `Block.getFriction()`. None for an unknown state.
    pub(super) fn block_state_friction(&self, state: BlockStateId) -> Option<f32> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.facts().get(state).friction)
    }
    /// `BlockState.isSolidRender()`. None for an unknown state.
    pub(super) fn block_state_solid_render(&self, state: BlockStateId) -> Option<bool> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.facts().get(state).solid_render)
    }
    /// `BlockTags.FIRE` (`minecraft:fire`, `minecraft:soul_fire`): the state's block is in the tag.
    /// None for an unknown state.
    pub(super) fn block_state_is_fire(&self, state: BlockStateId) -> Option<bool> {
        self.pack.assets.registry.state(state)?;
        let name = &self.res.facts().get(state).name;
        Some(self.items().block_in_tag("minecraft:fire", name))
    }
    /// `BlockState.isRedstoneConductor` for the state alone (without a level). None for an unknown
    /// state.
    pub(super) fn block_state_is_redstone_conductor(&self, state: BlockStateId) -> Option<bool> {
        self.pack.assets.registry.state(state)?;
        Some(
            self.res.behaviours().facts(state).flags
                & horizon_world::level::data::state_flags::CONDUCTOR
                != 0,
        )
    }
    /// `BlockState.requiresCorrectToolForDrops()`. None for an unknown state.
    pub(super) fn block_state_requires_correct_tool(&self, state: BlockStateId) -> Option<bool> {
        self.pack.assets.registry.state(state)?;
        Some(self.res.facts().get(state).requires_tool)
    }
    /// `ItemStack.isCorrectToolForDrops(state)`. None for an unknown state.
    pub(super) fn block_state_correct_tool_for_drops(
        &self,
        state: BlockStateId,
        stack: &ItemStack,
    ) -> Option<bool> {
        self.pack.assets.registry.state(state)?;
        let name = &self.res.facts().get(state).name;
        Some(self.items().correct_for_drops(Some(stack), name))
    }
    /// `Player.hasCorrectToolForDrops(state)` for the stack in the player's selected slot,
    /// without the mods' harvest hooks. None for an unknown state or an unavailable player.
    pub(super) fn player_has_correct_tool_for_drops(
        &self,
        player: PlayerRef,
        state: BlockStateId,
    ) -> Option<bool> {
        self.pack.assets.registry.state(state)?;
        // The selected hotbar stack (`Inventory.getSelectedItem`), never the acting hand: an
        // off-hand use leaves the player's tool rule as it was.
        let held = match self.player_read_both(player)? {
            PlayerRead::Acting { .. } => self.stack_in(self.selected),
            PlayerRead::Guest(guest) => guest.inventory.get(guest.selected)?.as_ref(),
        };
        let facts = self.res.facts().get(state);
        Some(
            self.items()
                .has_correct_tool_for_drops(held, facts.requires_tool, &facts.name),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_guest_turn_tests::with_guests;
    use crate::game::api_values::TryIntoApi;
    use horizon_client::items::ItemStack;
    use mod_api::{BlockState, HostBlockStates};

    /// Each read answers from vanilla's tables for a block whose answer is known.
    #[test]
    #[ignore = "daily"]
    fn block_state_reads_answer_vanillas_facts_through_the_host() {
        with_guests(|game| {
            let registry = game.pack.assets.registry;
            let state = |name: &str| -> BlockState {
                registry
                    .default_state(name)
                    .unwrap()
                    .try_into_api()
                    .unwrap()
            };
            let (stone, obsidian, dirt) = (
                state("minecraft:stone"),
                state("minecraft:obsidian"),
                state("minecraft:dirt"),
            );
            let (glowstone, ice, glass) = (
                state("minecraft:glowstone"),
                state("minecraft:ice"),
                state("minecraft:glass"),
            );
            let facts = game.block_states();
            assert_eq!(facts.light_emission(glowstone), Some(15));
            assert_eq!(facts.light_emission(stone), Some(0));
            assert_eq!(facts.friction(ice), Some(0.98));
            assert_eq!(facts.friction(stone), Some(0.6));
            assert_eq!(facts.is_solid_render(stone), Some(true));
            assert_eq!(facts.is_solid_render(glass), Some(false));
            assert_eq!(facts.is_fire(state("minecraft:fire")), Some(true));
            assert_eq!(facts.is_fire(state("minecraft:soul_fire")), Some(true));
            assert_eq!(facts.is_fire(stone), Some(false));
            assert_eq!(facts.is_fire(state("minecraft:air")), Some(false));
            assert_eq!(facts.is_redstone_conductor(stone), Some(true));
            assert_eq!(facts.is_redstone_conductor(glass), Some(false));
            assert_eq!(
                facts.is_redstone_conductor(state("minecraft:air")),
                Some(false)
            );
            assert_eq!(facts.requires_correct_tool(obsidian), Some(true));
            assert_eq!(facts.requires_correct_tool(dirt), Some(false));
            // A wooden pickaxe mines stone's drops but not obsidian's (it needs diamond).
            let wooden = ItemStack::new("minecraft:wooden_pickaxe", 1)
                .try_into_api()
                .unwrap();
            assert_eq!(facts.is_correct_tool_for_drops(stone, &wooden), Some(true));
            assert_eq!(
                facts.is_correct_tool_for_drops(obsidian, &wooden),
                Some(false)
            );

            // Guest 12's hand: empty harvests dirt but not stone; a wooden pickaxe stone too.
            let player = game.player(12).unwrap();
            let hold = |game: &mut super::Game<'_>, stack: Option<ItemStack>| {
                game.lan
                    .as_mut()
                    .unwrap()
                    .hub
                    .update_guest(12, |guest| guest.inventory[guest.selected] = stack)
                    .unwrap();
            };
            hold(game, None);
            assert_eq!(player.has_correct_tool_for_drops(game, dirt), Some(true));
            assert_eq!(player.has_correct_tool_for_drops(game, stone), Some(false));
            hold(game, Some(ItemStack::new("minecraft:wooden_pickaxe", 1)));
            assert_eq!(player.has_correct_tool_for_drops(game, stone), Some(true));
            assert_eq!(
                player.has_correct_tool_for_drops(game, obsidian),
                Some(false)
            );
        });
    }

    /// The acting player's tool rule reads the selected slot even while the off hand acts
    /// (`Player.hasCorrectToolForDrops` asks `Inventory.getSelectedItem`).
    #[test]
    #[ignore = "daily"]
    fn the_acting_players_tool_rule_reads_the_selected_slot_during_an_off_hand_use() {
        super::super::native_entity_refs::tests::with_game(|game| {
            use horizon_client::motion::PlayerMotion;
            use horizon_core::geometry::Vec3;

            game.options.dedicated = Some(Vec::new());
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 64.0, 4.5)).unwrap());
            game.lan_acting_uuid = Some(123);
            game.lan_acting_entity = Some(0);
            game.lan_acting_name = Some("Miner".into());
            game.lan_acting_permission = Some(0);
            game.inventory[game.selected] = Some(ItemStack::new("minecraft:wooden_pickaxe", 1));
            game.offhand = Some(ItemStack::new("minecraft:stick", 1));
            game.hand = crate::game::Hand::Off;
            let player = game.native_acting_player().unwrap();
            let stone: BlockState = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap()
                .try_into_api()
                .unwrap();
            assert_eq!(player.has_correct_tool_for_drops(game, stone), Some(true));
        });
    }

    /// Stone conducts redstone and glass doesn't, read at a position of the level.
    #[test]
    #[ignore = "daily"]
    fn a_level_handle_reads_whether_a_block_conducts_redstone() {
        super::super::native_entity_refs::tests::with_game(|game| {
            use horizon_client::chunk::Chunk;
            use std::sync::Arc;

            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            let registry = game.pack.assets.registry;
            game.world
                .set(5, 64, 5, registry.default_state("minecraft:stone").unwrap())
                .unwrap();
            game.world
                .set(6, 64, 5, registry.default_state("minecraft:glass").unwrap())
                .unwrap();
            let level = game.level(game.dim.id()).unwrap();
            assert_eq!(level.is_redstone_conductor(game, [5, 64, 5]), Some(true));
            assert_eq!(level.is_redstone_conductor(game, [6, 64, 5]), Some(false));
            // A chunk the level doesn't hold has no block to read.
            assert_eq!(level.is_redstone_conductor(game, [500, 64, 5]), None);
        });
    }
}
