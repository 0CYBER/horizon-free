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

/// `Player.attack`'s extra knockback from the weapon's knockback enchantment
/// (`getKnockback`, halved) and a sprinting full-strength hit (0.5), and
/// whether the hit is that sprinting knockback attack.
pub fn extra_knockback(enchanted: f32, strength: f32, sprinting: bool) -> (f32, bool) {
    let knockback_attack = strength > 0.9 && sprinting;
    let bonus = if knockback_attack { 0.5 } else { 0.0 };
    (enchanted / 2.0 + bonus, knockback_attack)
}

/// What the attacking client's own `Player.attack` does to itself on a hit
/// at another player, whose `RemotePlayer.hurtClient` lets the hit through:
/// whether `causeExtraKnockback` cuts its horizontal speed to 0.6 and stops
/// its sprint. `attack_damage` is the ATTACK_DAMAGE value before clamping
/// (the hand and weapon plus strength and weakness, or a riptide spin's),
/// `enchant_damage` the weapon's enchantment damage, `strength`
/// `getAttackStrengthScale(0.5)` and `knockback` from [`extra_knockback`].
pub fn attacker_slows_after_hit(
    attack_damage: f32,
    enchant_damage: f32,
    strength: f32,
    knockback: f32,
) -> bool {
    // The attribute's range is [0, 2048]: weakness can't make it negative.
    let base = attack_damage.clamp(0.0, 2048.0) * (0.2 + strength * strength * 0.8);
    let enchant = enchant_damage * strength;
    // Player.attack stops before the hurt and the knockback when neither
    // part of the damage is left.
    (base > 0.0 || enchant > 0.0) && knockback > 0.0
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
    /// The attacker's side of a hit on another player, as the client sees it.
    fn slows(damage: f32, enchant: f32, strength: f32, sprinting: bool, kb_enchant: f32) -> bool {
        let (knockback, _) = extra_knockback(kb_enchant, strength, sprinting);
        attacker_slows_after_hit(damage, enchant, strength, knockback)
    }
    #[test]
    fn a_charged_sprint_hit_slows_the_attacker() {
        // A diamond sword (1 + 6), full strength, sprinting.
        assert!(slows(7.0, 0.0, 1.0, true, 0.0));
        assert_eq!(extra_knockback(0.0, 1.0, true), (0.5, true));
    }
    #[test]
    fn an_uncharged_sprint_hit_does_not() {
        assert!(!slows(7.0, 0.0, 0.5, true, 0.0));
        assert!(
            !slows(7.0, 0.0, 0.9, true, 0.0),
            "the bonus needs more than 0.9"
        );
    }
    #[test]
    fn a_hit_without_sprinting_or_knockback_does_not() {
        assert!(!slows(7.0, 0.0, 1.0, false, 0.0));
    }
    #[test]
    fn a_knockback_weapon_slows_even_uncharged() {
        // Knockback I: getKnockback 1, halved; no sprint needed.
        assert!(slows(7.0, 0.0, 0.2, false, 1.0));
    }
    #[test]
    fn a_hit_that_does_no_damage_stops_first() {
        // An empty hand (1) under Weakness I (-4): the attribute clamps to 0,
        // so Player.attack ends before causeExtraKnockback.
        assert!(!slows(1.0 - 4.0, 0.0, 1.0, true, 0.0));
        assert!(!slows(0.0, 0.0, 1.0, true, 1.0));
        // Enchantment damage alone still lands the hit.
        assert!(slows(0.0, 1.25, 1.0, true, 0.0));
    }
    #[test]
    fn a_server_attack_speed_charges_at_once() {
        // 1.8-style servers raise ATTACK_SPEED: with 1024 the delay is under
        // a tick, so getAttackStrengthScale(0.5) is full right after a hit.
        let mut s = AttackStrength::default();
        s.reset_attack();
        assert_eq!(s.scale(0.5, 1024.0), 1.0);
        assert!(slows(7.0, 0.0, s.scale(0.5, 1024.0), true, 0.0));
        // A sword's own 1.6 leaves it uncharged.
        assert!(!slows(7.0, 0.0, s.scale(0.5, 1.6), true, 0.0));
    }
}
