//! Game-owned residency state; no second membership directory.
use super::*;
pub(super) struct ResidentEntities {
    pub table: Box<RefCell<horizon_client::join_seq::residency::table::EntityTable>>,
    pub local_player: horizon_client::join_seq::residency::member::Member,
    pub knots: horizon_client::join_seq::JoinList<leads::Knot>,
    /// The game's general entity rows (arismoko/horizon#792), beside the knots this store
    /// already keeps for the resident table's lookups.
    pub general: super::general::GeneralStore,
}

impl Default for ResidentEntities {
    fn default() -> Self {
        Self {
            table: Box::new(RefCell::new(
                horizon_client::join_seq::residency::table::EntityTable::new(Default::default()),
            )),
            local_player: Default::default(),
            knots: Default::default(),
            general: Default::default(),
        }
    }
}

impl Game<'_> {
    /// `Player.drop(stack, false)`, as `ItemUtils.createFilledResult` drops what no longer fits
    /// in the inventory: thrown from the eyes, with no thrower, statistic or drop hook.
    pub(super) fn drop_plain(&mut self, stack: ItemStack) -> Option<u32> {
        self.drop_stack_from_eyes(stack, false)
    }
}
