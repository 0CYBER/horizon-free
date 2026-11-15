//! Live living-entity weapon reads for death loot.
use super::{Game, commands::Target};
use horizon_client::items::ItemStack;
use mod_api::{Hand, LivingRef};

impl Game<'_> {
    pub(super) fn living_weapon_item(&self, living: LivingRef) -> Option<ItemStack> {
        let motion = match self.native_living_target(living)? {
            Target::Player => self.player.as_ref(),
            Target::Guest(conn) => self.command_guest(conn)?.motion.as_deref(),
            _ => None,
        };
        if let Some(player) = motion.filter(|player| player.auto_spin_attack_ticks > 0)
            && let Some(stack) = &player.auto_spin_attack_item
        {
            return Some(stack.clone());
        }
        self.living_hand_item(living, Hand::Main)
    }
}
