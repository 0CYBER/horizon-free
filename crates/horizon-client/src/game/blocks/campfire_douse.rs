//! `CampfireBlock.useItemOn`: an item in `#minecraft:douses_campfires` (the
//! shovels) put on a lit campfire puts it out.
use super::{Game, Hit};
use horizon_client::{Result, animate_tick::make_campfire_smoke, items::ItemStack};
use horizon_core::position::BlockPos;
use horizon_world::chunk::BlockState;

impl Game<'_> {
    /// The lit campfire at `pos` that `held` puts out, when the item is in
    /// `#minecraft:douses_campfires` (`CampfireBlock.useItemOn`).
    fn campfire_to_douse(&self, pos: [i32; 3], held: Option<&ItemStack>) -> Option<BlockState> {
        held.filter(|s| {
            self.res
                .items()
                .item_tags
                .get("minecraft:douses_campfires")
                .is_some_and(|members| members.contains(&*s.item))
        })?;
        self.pack
            .assets
            .registry
            .state(self.world.block(pos[0], pos[1], pos[2]))
            .filter(|s| matches!(&*s.name, "minecraft:campfire" | "minecraft:soul_campfire"))
            .filter(|s| s.properties.get("lit").map(String::as_str) == Some("true"))
            .cloned()
    }

    /// `CampfireBlock.useItemOn` as this client runs it before the server's
    /// answer (`MultiPlayerGameMode.useItemOn`): the twenty smoke puffs of
    /// `CampfireBlock.douse` and the campfire gone out, kept as predicted
    /// under `seq`. The level event's sound comes from the server. True when
    /// it doused (SUCCESS), which ends the click for both hands.
    pub(super) fn predict_campfire_douse(
        &mut self,
        hit: &Hit,
        held: Option<&ItemStack>,
        seq: i32,
    ) -> bool {
        let pos = hit.pos;
        let (Some(held), Some(mut state)) = (held, self.campfire_to_douse(pos, held)) else {
            return false;
        };
        state.properties.insert("lit".into(), "false".into());
        let Ok(unlit) = self.pack.assets.registry.resolve(&state) else {
            return false;
        };
        let Some(unlit) = self.modified_campfire(hit, held, unlit) else {
            return false;
        };
        let signal = state.properties.get("signal_fire").map(String::as_str) == Some("true");
        for _ in 0..20 {
            make_campfire_smoke(&mut self.particles, pos, signal, true);
        }
        let at = BlockPos::new(pos[0], pos[1], pos[2]);
        self.predict_blocks(&vec![(at, unlit)], seq);
        true
    }

    fn modified_campfire(
        &self,
        hit: &Hit,
        stack: &ItemStack,
        unlit: horizon_world::registry::BlockStateId,
    ) -> Option<horizon_world::registry::BlockStateId> {
        let block = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        if !mod_api::__private::listened(mod_api::HookId::BlockToolModification)
            && !crate::game::api_values::block_state::block_listens(
                mod_api::HookId::ToolModifiedState,
                block,
            )
        {
            return Some(unlit);
        }
        let tool = mod_api::ToolUse::from(mod_api::__private::built::ToolUse {
            player: self.player.is_some().then(|| self.own_entity_id() as u32),
            hand: self.hand,
            pos: hit.pos,
            face: hit.face,
            click: hit.point,
            ability: mod_api::ToolAbility::ShovelDouse,
            simulated: false,
        });
        let mut state = Some(unlit);
        let mut cancel = false;
        self.mod_tool_modification(stack, block, &tool, &mut state, &mut cancel);
        if cancel { None } else { state }
    }

    /// `CampfireBlock.useItemOn` with `held` in the hand that acts, the block
    /// at `pos` clicked: true when it doused a lit campfire (SUCCESS).
    pub(super) fn use_campfire_douse(
        &mut self,
        hit: &Hit,
        held: Option<&ItemStack>,
    ) -> Result<bool> {
        let pos = hit.pos;
        let (Some(used), Some(mut state)) = (held.cloned(), self.campfire_to_douse(pos, held))
        else {
            return Ok(false);
        };
        state.properties.insert("lit".into(), "false".into());
        let Ok(unlit) = self.pack.assets.registry.resolve(&state) else {
            return Ok(false);
        };
        let Some(unlit) = self.modified_campfire(hit, &used, unlit) else {
            return Ok(false);
        };
        // Level event 1009 (SOUND_EXTINGUISH_FIRE) goes to every player.
        self.level_event_at(1009, pos, 0);
        // CampfireBlock.douse: the acting client's own smoke (twenty puffs),
        // then the block change game event from the player.
        if self.lan_acting.is_none() {
            let signal = state.properties.get("signal_fire").map(String::as_str) == Some("true");
            for _ in 0..20 {
                make_campfire_smoke(&mut self.particles, pos, signal, true);
            }
        }
        self.post_block_game_event("minecraft:block_change", pos, self.sculk_actor(), None);
        self.level_set(pos, unlit)?;
        self.damage_held(1);
        // ServerPlayerGameMode.useItemOn: the action was consumed.
        self.trigger_block("minecraft:item_used_on_block", pos, &used);
        Ok(true)
    }
}
