//! The player's attack strength (`Player.attackStrengthTicker`,
//! `itemSwapTicker`, `getAttackStrengthScale`, `getCurrentItemAttackStrengthDelay`):
//! the charge the attack indicator shows and attacks scale their damage by.

/// `Attributes.ATTACK_SPEED` base value.
pub const BASE_ATTACK_SPEED: f64 = 4.0;

#[derive(Clone, Debug, Default, PartialEq)]
pub struct AttackStrength {
    /// `LivingEntity.attackStrengthTicker`.
    pub ticker: i32,
    /// `LivingEntity.itemSwapTicker`.
    pub swap_ticker: i32,
    /// `Player.lastItemInMainHand`: the stack as last compared.
    last_item: Option<HeldStack>,
}
/// What `ItemStack.matches` / `isSameItem` compare of the main-hand stack:
/// its item id and the rest of the stack (count and components) as a key.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HeldStack {
    pub item: String,
    pub rest: String,
}
/// `Player.getCurrentItemAttackStrengthDelay` for an ATTACK_SPEED value.
pub fn delay(attack_speed: f64) -> f32 {
    (1.0 / attack_speed * 20.0) as f32
}
impl AttackStrength {
    /// The attack-strength part of `Player.tick`: both tickers advance, and a
    /// main-hand change to a different item resets them
    /// (`!ItemStack.matches` and `!ItemStack.isSameItem`).
    /// Whether they were reset.
    pub fn tick(&mut self, main_hand: Option<HeldStack>) -> bool {
        self.ticker += 1;
        self.swap_ticker += 1;
        let mut reset = false;
        if self.last_item != main_hand {
            let same_item = match (&self.last_item, &main_hand) {
                (Some(a), Some(b)) => a.item == b.item,
                (None, None) => true,
                _ => false,
            };
            if !same_item {
                self.reset();
                reset = true;
            }
            self.last_item = main_hand;
        }
        reset
    }
    /// `resetAttackStrengthTicker`: swinging at nothing, attacking or
    /// stopping a block break.
    pub fn reset(&mut self) {
        self.ticker = 0;
        self.swap_ticker = 0;
    }
    /// `resetOnlyAttackStrengthTicker` (`Player.onAttack`).
    pub fn reset_attack(&mut self) {
        self.ticker = 0;
    }
    /// `getAttackStrengthScale(a)` with the ATTACK_SPEED attribute value.
    pub fn scale(&self, a: f32, attack_speed: f64) -> f32 {
        ((self.ticker as f32 + a) / delay(attack_speed)).clamp(0.0, 1.0)
    }
    /// `getItemSwapScale(a)`.
    pub fn swap_scale(&self, a: f32, attack_speed: f64) -> f32 {
        ((self.swap_ticker as f32 + a) / delay(attack_speed)).clamp(0.0, 1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    fn held(item: &str, rest: &str) -> Option<HeldStack> {
        Some(HeldStack {
            item: item.into(),
            rest: rest.into(),
        })
    }
    #[test]
    fn charges_over_the_attack_delay() {
        let mut s = AttackStrength::default();
        // A sword's 1.6 attacks per second: 12.5 ticks to full strength.
        assert_eq!(delay(1.6), 12.5);
        for _ in 0..6 {
            s.tick(None);
        }
        assert_eq!(s.scale(0.25, 1.6), 6.25 / 12.5);
        for _ in 0..10 {
            s.tick(None);
        }
        assert_eq!(s.scale(0.0, 1.6), 1.0);
    }
    #[test]
    fn switching_items_resets_but_restacking_does_not() {
        let mut s = AttackStrength::default();
        s.tick(held("minecraft:stone", "64"));
        assert_eq!(s.ticker, 0, "picking up an item resets");
        s.tick(held("minecraft:stone", "63"));
        assert_eq!(
            s.ticker, 1,
            "the same item with another count keeps charging"
        );
        s.tick(held("minecraft:diamond_sword", ""));
        assert_eq!((s.ticker, s.swap_ticker), (0, 0));
        s.tick(held("minecraft:diamond_sword", ""));
        s.reset_attack();
        assert_eq!((s.ticker, s.swap_ticker), (0, 1));
    }
}
