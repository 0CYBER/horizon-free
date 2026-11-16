//! `Entity.getBlockBounciness` for the player: the block's restitution after its parts.
use super::{MotionWorld, Result, movement_support};
use crate::{block_contact, health::DamageLevel};
use horizon_world::{native_load_context::LoopSide, registry::BlockStateId};

impl MotionWorld<'_, '_, '_> {
    /// `Block.getBounceRestitution` of `state` for the player landing on it at
    /// `Entity.getOnPosLegacy`, after the block's `bounce_restitution` parts in the player's
    /// level on the side that ticks it (the server, or the client predicting or replaying its
    /// own move). Without a part, no level is built.
    pub(super) fn part_bounce_restitution(&mut self, state: BlockStateId) -> Result<f32> {
        let block = self.resources.blocks.get(state)?;
        let own = block.coefficients.bounce_restitution;
        if !block_contact::restitution_listens(state) {
            return Ok(own);
        }
        let position = self.support(movement_support::LEGACY_SUPPORT_OFFSET)?;
        let client_side = self.loop_context.side() == LoopSide::Client;
        let contact = block_contact::Contact {
            entity: self.player.entity_id,
            kind: "minecraft:player",
            state,
            position,
        };
        let level = DamageLevel::of(
            &mut *self.scene,
            self.assets.registry,
            self.tag_items,
            self.block_registries,
            self.damage_hooks,
        );
        Ok(block_contact::bounce_restitution(
            &mut level.on_client(client_side),
            contact,
            own,
        ))
    }
}
