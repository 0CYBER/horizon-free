//! The enchanting power around an enchanting table (`EnchantmentMenu.slotsChanged`): the
//! bookshelves and the blocks whose parts answer `enchant_power_bonus`.
use super::Game;
use horizon_client::stations::bookshelf_offsets;
use horizon_core::position::BlockPos;
use horizon_world::registry::BlockStateId;

impl Game<'_> {
    /// The power around the table at `pos`: the sum, over `EnchantingTableBlock.BOOKSHELF_OFFSETS`
    /// whose block between it and the table is in `#minecraft:enchantment_power_transmitter`, of
    /// each block's `getEnchantPowerBonus`, truncated as NeoForge's `(int) power`. The offers cap
    /// it at 15.
    pub(super) fn count_bookshelves(&self, pos: [i32; 3]) -> i32 {
        let table = BlockPos::new(pos[0], pos[1], pos[2]);
        let power: f32 = bookshelf_offsets()
            .into_iter()
            .filter(|o| {
                self.enchant_transmits(self.world.block(
                    pos[0] + o[0] / 2,
                    pos[1] + o[1],
                    pos[2] + o[2] / 2,
                ))
            })
            .map(|o| {
                let at = BlockPos::new(pos[0] + o[0], pos[1] + o[1], pos[2] + o[2]);
                let state = self.world.block(at.x, at.y, at.z);
                self.enchant_power(state, at, table)
            })
            .sum();
        // A negative sum would make vanilla's `nextInt(bookcases + 1)` throw.
        (power as i32).max(0)
    }

    fn enchant_transmits(&self, state: BlockStateId) -> bool {
        self.items()
            .block_tags
            .get("minecraft:enchantment_power_transmitter")
            .is_some_and(|tag| tag.contains(&*self.res.facts().get(state).name))
    }

    /// Shared enchant-power answer: block parts may override the provider-tag default;
    /// both offers and the client particle-table rebuild use this answer.
    pub(in crate::game) fn enchant_power(
        &self,
        state: BlockStateId,
        pos: BlockPos,
        table: BlockPos,
    ) -> f32 {
        let own = self
            .items()
            .default_enchant_power(&self.res.facts().get(state).name);
        if !super::api_values::block_state::block_listens(mod_api::HookId::EnchantPowerBonus, state)
        {
            return own;
        }
        let mut power = own;
        self.native_enchant_power_bonus(state, pos, table, &mut power);
        if power.is_finite() { power } else { own }
    }

    /// Snapshot at world entry/parts reload, or after received tags and tag-update listeners
    /// are published. Particle workers receive state answers, never a host or mod callback.
    pub(in crate::game) fn resolve_enchant_particles(&self) {
        if self.is_logical_server() {
            return;
        }
        self.ambience.enchant_power.rebuild(
            self.pack.assets.registry.len(),
            |state| {
                let origin = BlockPos::new(0, 0, 0);
                self.enchant_power(state, origin, origin)
            },
            |state| self.enchant_transmits(state),
        );
    }
}
