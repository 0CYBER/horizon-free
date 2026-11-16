use super::*;

impl HitLevel<'_> {
    /// `IBlockExtension.getBounceRestitution`: entity `entity` of type `kind` landed on `state`
    /// at `pos` (`Entity.getOnPosLegacy`). `factor` is the block's own restitution, which its
    /// parts may change. Asked on both sides, in the level of the side that ticks the entity.
    pub(in crate::game) fn native_bounce_restitution(
        &self,
        entity: i32,
        kind: &str,
        state: BlockStateId,
        pos: BlockPos,
        factor: &mut f32,
    ) {
        self.fire_bounce_restitution(entity, kind, state, pos, factor);
    }
}

/// The restitution the simulation's own code (client-lib) asks of a block an entity lands on.
const RESTITUTION_HOOKS: horizon_client::block_contact::RestitutionHooks =
    horizon_client::block_contact::RestitutionHooks {
        listens: |state| {
            crate::game::api_values::block_state::block_listens(
                mod_api::HookId::BounceRestitution,
                state,
            )
        },
        restitution: |level, c, factor| {
            HitLevel(level.reborrow())
                .native_bounce_restitution(c.entity, c.kind, c.state, c.position, factor);
        },
    };

/// Bind the block contacts and the bounce restitution the simulation reports to the block
/// parts; the first binding stays.
pub(in crate::game) fn install_block_contact_hooks() {
    let _ = horizon_client::block_contact::HOOKS.set(BLOCK_CONTACT_HOOKS);
    let _ = horizon_client::block_contact::RESTITUTION_HOOKS.set(RESTITUTION_HOOKS);
}
