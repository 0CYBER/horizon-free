//! Whether an entity may destroy a block: the block's parts' answer to vanilla's per-block
//! rule (`canEntityDestroy`), then the mods' veto (`LivingDestroyBlockEvent`).
use super::*;

impl Game<'_> {
    /// `IBlockExtension.canEntityDestroy` of the block at `pos` for the entity `entity`:
    /// `vanilla`, the entity's own rule, unless the block's parts answer otherwise on a
    /// logical server. A panicking part is contained with a warning, and `vanilla` stands.
    pub(in crate::game) fn entity_may_destroy(
        &self,
        entity: u32,
        pos: [i32; 3],
        vanilla: bool,
    ) -> bool {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        if !self.is_logical_server()
            || !crate::game::api_values::block_state::block_listens(
                mod_api::HookId::CanEntityDestroy,
                state,
            )
        {
            return vanilla;
        }
        let Some(epoch) = self.native_lifecycle.epoch() else {
            return vanilla;
        };
        let mut may_destroy = None;
        let at = horizon_core::position::BlockPos::new(pos[0], pos[1], pos[2]);
        let entity = mod_api::EntityRef::new(epoch, entity);
        if let Err(error) = horizon_client::crash::contain(|| {
            self.native_can_entity_destroy(at, state, entity, &mut may_destroy);
        }) {
            log::warn!("can_entity_destroy callback failed: {error}");
            return vanilla;
        }
        may_destroy.unwrap_or(vanilla)
    }
    /// `LivingDestroyBlockEvent` for the mob `id` about to destroy the block at
    /// `pos`: false when a mod canceled. Nothing is built when no mod is loaded.
    pub(in crate::game) fn mob_may_destroy(&mut self, id: u32, pos: [i32; 3]) -> bool {
        if !mod_api::__private::listened(mod_api::HookId::LivingDestroyBlock) {
            return true;
        }
        let Some(kind) = self
            .mobs
            .iter()
            .find(|m| m.id == id)
            .map(crate::game::native_hooks::mob_kind)
        else {
            return true;
        };
        self.may_destroy(id, &kind, pos)
    }
    /// Asks the mods whether the entity `id` of type `kind` may destroy the block at `pos`.
    pub(in crate::game) fn may_destroy(&mut self, id: u32, kind: &str, pos: [i32; 3]) -> bool {
        let state = self.world.block(pos[0], pos[1], pos[2]);
        let mut cancel = false;
        self.native_living_destroy_block(id, kind, pos, state, &mut cancel);
        !cancel
    }
}
