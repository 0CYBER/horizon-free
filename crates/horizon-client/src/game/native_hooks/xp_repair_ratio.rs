use super::*;

impl Game<'_> {
    /// `IItemExtension.getXpRepairRatio`: the durability points Mending
    /// restores to `stack` per point of `experience`, arriving as vanilla's.
    /// Server-side, including `PlayerRef::give_experience_points`.
    pub(in crate::game) fn native_xp_repair_ratio(
        &self,
        player: u128,
        stack: &ItemStack,
        experience: u32,
        ratio: &mut f32,
    ) {
        self.fire_xp_repair_ratio(player, stack, experience, ratio);
    }
}
