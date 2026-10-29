//! `AbstractThrownPotion`: what a splash or lingering potion does around where
//! it lands besides its effects, as `#minecraft:douses_fire`,
//! `#minecraft:hurts_water_sensitive_entities`, `#minecraft:extinguishes_entities`
//! and `#minecraft:rehydrates_axolotls` (plain water in vanilla) decide.
use super::Game;
use horizon_client::items::ItemStack;

impl Game<'_> {
    /// `PotionContents.is`: the potion of `stack` is in the potion tag `tag`
    /// (`minecraft:douses_fire` and the others, read from the data pack's
    /// `tags/potion`). A stack with no potion is in no tag.
    pub(super) fn potion_in_tag(&self, stack: &ItemStack, tag: &str) -> bool {
        stack.potion().is_some_and(|potion| {
            self.res
                .stations()
                .trades
                .potion_tags
                .get(tag)
                .is_some_and(|members| members.iter().any(|m| m.as_str() == potion))
        })
    }
}
