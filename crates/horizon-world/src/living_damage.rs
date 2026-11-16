//! Inner living/player damage application after the outer hurt gates.
use crate::combat_rules;
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum DamageStat {
    Absorbed,
    DealtAbsorbed,
    Taken,
}
/// What took a share of a hit before the health, as NeoForge's `DamageContainer.Reduction`
/// names it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Reduction {
    /// The damage cooldown: what the last hit already dealt (`lastHurt`), taken off a hit
    /// that lands within it.
    Invulnerability,
    /// Armor and toughness (`getDamageAfterArmorAbsorb`).
    Armor,
    /// Protection enchantments (`EnchantmentHelper.getDamageProtection`).
    Enchantments,
    /// Resistance (`getDamageAfterMagicAbsorb`'s mob effect step).
    MobEffects,
    /// The absorption hearts.
    Absorption,
    /// What the entity's own kind resists: a witch's magic resistance.
    InnateResistance,
}
const REDUCTIONS: usize = 6;
/// How a modifier changes what a reduction took (NeoForge's `IReductionsModifier`, as plain
/// data so it also crosses into a wasm mod).
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum ReductionChange {
    /// Take this much more off (less when negative).
    Add(f32),
    /// Take this many times as much.
    Scale(f32),
    /// Take at least this much.
    AtLeast(f32),
    /// Take at most this much.
    AtMost(f32),
}
impl ReductionChange {
    fn apply(self, taken: f32) -> f32 {
        match self {
            Self::Add(by) => taken + by,
            Self::Scale(by) => taken * by,
            Self::AtLeast(least) => taken.max(least),
            Self::AtMost(most) => taken.min(most),
        }
    }
}
/// One hit's damage sequence from `hurtServer` to the health, as NeoForge's
/// `DamageContainer` carries it: the damage that came in, what blocking, the cooldown and
/// each reduction took, the damage `LivingDamageEvent.Pre` left and what the health lost.
/// Numeric values are 0 until the step that sets them has run; side effects are false until
/// `hurtServer` decides whether the hit is fresh.
#[derive(Clone, Debug, PartialEq)]
pub struct DamageContainer {
    original: f32,
    new_damage: f32,
    blocked: f32,
    shield_damage: f32,
    post_attack_invulnerability_ticks: i32,
    should_cause_side_effects: bool,
    reductions: [f32; REDUCTIONS],
    /// The modifiers registered for this hit, in the order they were added.
    reduction_modifiers: Vec<(Reduction, ReductionChange)>,
    entered_pre: f32,
    inflicted: f32,
    health_damage: f32,
}
/// A hit's damage sequence as plain values, field for field: the form it crosses into a wasm
/// mod as.
#[derive(Clone, Debug, PartialEq)]
pub struct DamageValues {
    pub original: f32,
    pub new_damage: f32,
    pub blocked: f32,
    pub shield_damage: f32,
    pub post_attack_invulnerability_ticks: i32,
    pub should_cause_side_effects: bool,
    pub reductions: [f32; 6],
    pub reduction_modifiers: Vec<(Reduction, ReductionChange)>,
    pub entered_pre: f32,
    pub inflicted: f32,
    pub health_damage: f32,
}

impl From<&DamageContainer> for DamageValues {
    fn from(d: &DamageContainer) -> Self {
        Self {
            original: d.original,
            new_damage: d.new_damage,
            blocked: d.blocked,
            shield_damage: d.shield_damage,
            post_attack_invulnerability_ticks: d.post_attack_invulnerability_ticks,
            should_cause_side_effects: d.should_cause_side_effects,
            reductions: d.reductions,
            reduction_modifiers: d.reduction_modifiers.clone(),
            entered_pre: d.entered_pre,
            inflicted: d.inflicted,
            health_damage: d.health_damage,
        }
    }
}

/// A damage sequence from its values (the form a wasm mod gives it back in).
impl From<DamageValues> for DamageContainer {
    fn from(v: DamageValues) -> Self {
        Self {
            original: v.original,
            new_damage: v.new_damage,
            blocked: v.blocked,
            shield_damage: v.shield_damage,
            post_attack_invulnerability_ticks: v.post_attack_invulnerability_ticks,
            should_cause_side_effects: v.should_cause_side_effects,
            reductions: v.reductions,
            reduction_modifiers: v.reduction_modifiers,
            entered_pre: v.entered_pre,
            inflicted: v.inflicted,
            health_damage: v.health_damage,
        }
    }
}

impl DamageContainer {
    /// A hit of `original` damage, as `hurtServer` was given it, with vanilla's 20 ticks of
    /// damage cooldown after it.
    pub fn new(original: f32) -> Self {
        Self {
            original,
            new_damage: original,
            blocked: 0.0,
            shield_damage: 0.0,
            post_attack_invulnerability_ticks: 20,
            should_cause_side_effects: false,
            reductions: [0.0; REDUCTIONS],
            reduction_modifiers: Vec::new(),
            entered_pre: 0.0,
            inflicted: 0.0,
            health_damage: 0.0,
        }
    }
    /// The damage the hit came in with (`getOriginalDamage`), before anything changed it.
    pub fn original(&self) -> f32 {
        self.original
    }
    /// The damage the sequence goes on with (`getNewDamage`).
    pub fn new_damage(&self) -> f32 {
        self.new_damage
    }
    /// Change the damage the sequence goes on with (`setNewDamage`). Set in
    /// `LivingDamageEvent.Pre`, it is what the absorption hearts and the health take: 0 or
    /// less takes nothing, and a value that is not a number is put back as it was.
    pub fn set_new_damage(&mut self, damage: f32) {
        self.new_damage = damage;
    }
    /// The damage a blocking item stopped (`getBlockedDamage`).
    pub fn blocked(&self) -> f32 {
        self.blocked
    }
    /// The durability the raised blocking item lost (`getShieldDamage`).
    pub fn shield_damage(&self) -> f32 {
        self.shield_damage
    }
    /// The damage cooldown the entity has once the hit is applied
    /// (`getPostAttackInvulnerabilityTicks`): 20 for a fresh hit, what is left of it for a hit
    /// within it.
    pub fn post_attack_invulnerability_ticks(&self) -> i32 {
        self.post_attack_invulnerability_ticks
    }
    /// Change the damage cooldown the entity has once the hit is applied
    /// (`setPostAttackInvulnerabilityTicks`).
    pub fn set_post_attack_invulnerability_ticks(&mut self, ticks: i32) {
        self.post_attack_invulnerability_ticks = ticks;
    }
    /// Whether `hurtServer` decided this hit was fresh (`shouldCauseSideEffects`), so it
    /// causes hurt sounds, knockback and other fresh-hit effects. False for a stronger hit
    /// inside the damage cooldown unless its source bypasses the cooldown. This is recorded
    /// before `actuallyHurt`, independently of the final damage or modded cooldown ticks.
    pub fn should_cause_side_effects(&self) -> bool {
        self.should_cause_side_effects
    }
    /// The damage sequence: `hurtServer`'s fresh-hit decision before the damage hooks run.
    #[doc(hidden)]
    pub fn record_should_cause_side_effects(&mut self, fresh: bool) {
        self.should_cause_side_effects = fresh;
    }
    /// What `reduction` took off the hit (`getReduction`).
    pub fn reduction(&self, reduction: Reduction) -> f32 {
        self.reductions[reduction as usize]
    }
    /// The damage `LivingDamageEvent.Pre` left (`getInflictedDamage`), absorption included.
    pub fn inflicted(&self) -> f32 {
        self.inflicted
    }
    /// The health the hit took (`getHealthDamage`).
    pub fn health_damage(&self) -> f32 {
        self.health_damage
    }
    /// The damage sequence: what blocking stopped.
    #[doc(hidden)]
    pub fn record_block(&mut self, blocked: f32) {
        self.blocked = blocked;
    }
    /// The damage sequence: the durability the blocking item lost.
    #[doc(hidden)]
    pub fn record_shield_damage(&mut self, durability: f32) {
        self.shield_damage = durability;
    }
    /// Change what `reduction` takes off this hit, as NeoForge's
    /// `DamageContainer.addReductionModifier` does. Add it from `living_incoming_damage`: it
    /// applies when the step that makes `reduction` runs, to the amount it took, and the
    /// recorded reduction and the damage the sequence goes on with are the changed ones.
    /// Modifiers on one reduction apply in the order they were added, each to the last one's
    /// result; the changed amount stays between 0 and what there was to take (the damage
    /// that reached the step, and for the absorption hearts what they hold), and one that
    /// is not a number leaves the amount as it was. A step the hit skips (a source that
    /// bypasses armor, no Resistance effect, ...) runs no modifier. They last for this hit.
    pub fn add_reduction_modifier(&mut self, reduction: Reduction, change: ReductionChange) {
        self.reduction_modifiers.push((reduction, change));
    }
    /// The damage sequence: whether a modifier is registered for `reduction`.
    #[doc(hidden)]
    pub fn has_reduction_modifier(&self, reduction: Reduction) -> bool {
        self.reduction_modifiers
            .iter()
            .any(|(r, _)| *r == reduction)
    }
    /// The damage sequence: what `reduction` took.
    #[doc(hidden)]
    pub fn record_reduction(&mut self, reduction: Reduction, amount: f32) {
        self.reductions[reduction as usize] = amount;
    }
    /// The registered modifiers applied to `taken`, kept within 0 and `limit`; none when
    /// no modifier applies or the result is not a number.
    #[doc(hidden)]
    pub fn modified_reduction(&self, reduction: Reduction, taken: f32, limit: f32) -> Option<f32> {
        let mut changes = self
            .reduction_modifiers
            .iter()
            .filter(|(r, _)| *r == reduction)
            .map(|(_, change)| change)
            .peekable();
        changes.peek()?;
        let modified = changes.fold(taken, |amount, change| change.apply(amount));
        (!modified.is_nan()).then(|| modified.clamp(0.0, limit.max(0.0)))
    }
    /// The damage sequence: `reduction` took `taken` off `before`, leaving `after`. Records
    /// what it took and returns the damage the sequence goes on with. `ran` is whether the
    /// step ran (not skipped by the source); only a step that ran applies the registered
    /// modifiers, and without any the values stand as they came.
    #[doc(hidden)]
    pub fn reduce(
        &mut self,
        reduction: Reduction,
        before: f32,
        taken: f32,
        after: f32,
        ran: bool,
    ) -> f32 {
        match ran
            .then(|| self.modified_reduction(reduction, taken, before))
            .flatten()
        {
            Some(modified) => {
                self.record_reduction(reduction, modified);
                before - modified
            }
            None => {
                self.record_reduction(reduction, taken);
                after
            }
        }
    }
    /// The damage sequence, before `LivingDamageEvent.Pre`: the damage it is told.
    #[doc(hidden)]
    pub fn enter_pre(&mut self, damage: f32) {
        self.entered_pre = damage;
        self.new_damage = damage;
    }
    /// The damage sequence, after `LivingDamageEvent.Pre`: the damage it left, which the
    /// sequence goes on with (`captureInflictedDamage`). Below 0 is none; not a number is
    /// what Pre was told.
    #[doc(hidden)]
    pub fn capture_inflicted(&mut self) -> f32 {
        if self.new_damage.is_nan() {
            self.new_damage = self.entered_pre;
        } else if self.new_damage < 0.0 {
            self.new_damage = 0.0;
        }
        self.inflicted = self.new_damage;
        self.inflicted
    }
    /// What `LivingDamageEvent.Pre` added to the damage (negative: took off). The damage
    /// cooldown remembers the hit with it (`lastHurt`).
    #[doc(hidden)]
    pub fn pre_change(&self) -> f32 {
        self.inflicted - self.entered_pre
    }
    /// The damage sequence: the health the hit took.
    #[doc(hidden)]
    pub fn record_health_damage(&mut self, damage: f32) {
        self.health_damage = damage;
    }
}
/// What `getDamageAfterMagicAbsorb` left of a hit, and how much of what it took was the
/// Resistance effect's (the rest is protection enchantments'), with what a modifier on
/// either needs to redo the step after it.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct MagicAbsorb {
    pub damage: f32,
    pub mob_effects: f32,
    /// Whether the Resistance step ran (the entity has the effect and the source does not
    /// bypass it).
    pub resisted: bool,
    /// The damage Resistance left, which the enchantments started from.
    pub after_effects: f32,
    /// The protection enchantments' total when their step ran (damage left, the source does
    /// not bypass them); none when it was skipped.
    pub protection: Option<f32>,
}
pub trait DamageWorld {
    type Error;
    type Attacker: Copy;
    fn invulnerable(&mut self) -> Result<bool, Self::Error>;
    fn after_armor(&mut self, damage: f32) -> Result<f32, Self::Error>;
    /// Whether the source bypasses armor (`#bypasses_armor`), so the armor step is skipped.
    /// Asked only when a modifier is registered for the armor reduction.
    fn bypasses_armor(&mut self) -> Result<bool, Self::Error>;
    fn after_magic(&mut self, damage: f32) -> Result<MagicAbsorb, Self::Error>;
    fn absorption(&mut self) -> Result<f32, Self::Error>;
    fn set_absorption(&mut self, value: f32) -> Result<(), Self::Error>;
    fn server_player_attacker(&mut self) -> Result<Option<Self::Attacker>, Self::Error>;
    fn award_attacker(
        &mut self,
        attacker: Self::Attacker,
        stat: DamageStat,
        amount: i32,
    ) -> Result<(), Self::Error>;
    fn award_self(&mut self, stat: DamageStat, amount: i32) -> Result<(), Self::Error>;
    fn source_food_exhaustion(&mut self) -> Result<f32, Self::Error>;
    fn cause_food_exhaustion(&mut self, amount: f32) -> Result<(), Self::Error>;
    fn record_damage(&mut self, amount: f32) -> Result<(), Self::Error>;
    fn health(&mut self) -> Result<f32, Self::Error>;
    fn set_health(&mut self, value: f32) -> Result<(), Self::Error>;
    fn damage_event(&mut self) -> Result<(), Self::Error>;
    /// `LivingDamageEvent.Pre`: the armor and magic reductions are in `container`, and
    /// the absorption hearts have not taken their share; it may change the new damage. The
    /// default does nothing.
    fn damage_pre(&mut self, _container: &mut DamageContainer) -> Result<(), Self::Error> {
        Ok(())
    }
}
pub(crate) fn positive(value: f32) -> f32 {
    if value.is_nan() {
        value
    } else if value <= 0.0 {
        0.0
    } else {
        value
    }
}
pub(crate) fn stat_amount(value: f32) -> i32 {
    let value = value * 10.0;
    // Java Math.round(float), including negative ties and integer saturation.
    let bits = value.to_bits() as i32;
    let shift = 149 - ((bits >> 23) & 0xff);
    if (0..32).contains(&shift) {
        let mut significand = (bits & 0x7f_ffff) | 0x80_0000;
        if bits < 0 {
            significand = -significand;
        }
        ((significand >> shift) + 1) >> 1
    } else {
        value as i32
    }
}
pub(crate) const STAT_LIMIT: f32 = 3.402_823_5e37;
pub fn actually_hurt<W: DamageWorld>(
    world: &mut W,
    container: &mut DamageContainer,
    damage: f32,
    player: bool,
) -> Result<(), W::Error> {
    if world.invulnerable()? {
        return Ok(());
    }
    actually_hurt_after_immunity_check(world, container, damage, player)
}
/// Caller must have performed this hit's native invulnerability query.
pub(crate) fn actually_hurt_after_immunity_check<W: DamageWorld>(
    world: &mut W,
    container: &mut DamageContainer,
    damage: f32,
    player: bool,
) -> Result<(), W::Error> {
    let after_armor = world.after_armor(damage)?;
    let ran = container.has_reduction_modifier(Reduction::Armor) && !world.bypasses_armor()?;
    let after_armor = container.reduce(
        Reduction::Armor,
        damage,
        damage - after_armor,
        after_armor,
        ran,
    );
    actually_hurt_after_armor(world, container, after_armor, player)
}
/// What the damage is once the Resistance and enchantment reductions (and the modifiers
/// registered for them) have taken their shares of `after_armor`.
fn after_magic_reductions(
    container: &mut DamageContainer,
    after_armor: f32,
    magic: MagicAbsorb,
) -> f32 {
    let modified = |reduction| container.has_reduction_modifier(reduction);
    if !(magic.resisted && modified(Reduction::MobEffects)
        || magic.protection.is_some() && modified(Reduction::Enchantments))
    {
        container.record_reduction(Reduction::MobEffects, magic.mob_effects);
        container.record_reduction(
            Reduction::Enchantments,
            after_armor - magic.mob_effects - magic.damage,
        );
        return magic.damage;
    }
    let after_effects = container.reduce(
        Reduction::MobEffects,
        after_armor,
        magic.mob_effects,
        magic.after_effects,
        magic.resisted,
    );
    let Some(protection) = magic.protection else {
        container.record_reduction(Reduction::Enchantments, 0.0);
        return after_effects;
    };
    let enchanted = if after_effects > 0.0 && protection > 0.0 {
        combat_rules::after_magic(after_effects, protection)
    } else {
        after_effects
    };
    container.reduce(
        Reduction::Enchantments,
        after_effects,
        after_effects - enchanted,
        enchanted,
        true,
    )
}
pub(crate) fn actually_hurt_after_armor<W: DamageWorld>(
    world: &mut W,
    container: &mut DamageContainer,
    after_armor: f32,
    player: bool,
) -> Result<(), W::Error> {
    let magic = world.after_magic(after_armor)?;
    let magic_damage = after_magic_reductions(container, after_armor, magic);
    container.enter_pre(magic_damage);
    world.damage_pre(container)?;
    let damage = container.capture_inflicted();
    let mut remaining = positive(damage - world.absorption()?);
    let mut absorption = world.absorption()? - (damage - remaining);
    if container.has_reduction_modifier(Reduction::Absorption) {
        let hearts = world.absorption()?;
        if let Some(modified) = container.modified_reduction(
            Reduction::Absorption,
            damage - remaining,
            damage.min(hearts),
        ) {
            remaining = damage - modified;
            absorption = hearts - modified;
        }
    }
    world.set_absorption(absorption)?;
    let absorbed = damage - remaining;
    container.record_reduction(Reduction::Absorption, absorbed);
    if absorbed > 0.0 && absorbed < STAT_LIMIT {
        if player {
            world.award_self(DamageStat::Absorbed, stat_amount(absorbed))?;
        } else if let Some(attacker) = world.server_player_attacker()? {
            world.award_attacker(attacker, DamageStat::DealtAbsorbed, stat_amount(absorbed))?;
        }
    }
    if remaining == 0.0 {
        return Ok(());
    }
    if player {
        let exhaustion = world.source_food_exhaustion()?;
        world.cause_food_exhaustion(exhaustion)?;
    }
    world.record_damage(remaining)?;
    let health = world.health()?;
    world.set_health(health - remaining)?;
    if player {
        if remaining < STAT_LIMIT {
            world.award_self(DamageStat::Taken, stat_amount(remaining))?;
        }
    } else {
        let absorption = world.absorption()?;
        world.set_absorption(absorption - remaining)?;
    }
    world.damage_event()?;
    container.record_health_damage(remaining);
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    /// A living entity whose steps are simple enough to compute by hand: armor halves the
    /// hit, Resistance takes a quarter of what is left, protection takes half of that.
    struct Target {
        health: f32,
        absorption: f32,
        bypass_armor: bool,
        pre_damage: Vec<f32>,
    }
    impl Target {
        fn new(absorption: f32) -> Self {
            Self {
                health: 100.0,
                absorption,
                bypass_armor: false,
                pre_damage: Vec::new(),
            }
        }
    }
    impl DamageWorld for Target {
        type Error = ();
        type Attacker = ();
        fn invulnerable(&mut self) -> Result<bool, ()> {
            Ok(false)
        }
        fn after_armor(&mut self, damage: f32) -> Result<f32, ()> {
            Ok(if self.bypass_armor {
                damage
            } else {
                damage * 0.5
            })
        }
        fn bypasses_armor(&mut self) -> Result<bool, ()> {
            Ok(self.bypass_armor)
        }
        fn after_magic(&mut self, damage: f32) -> Result<MagicAbsorb, ()> {
            let after_effects = damage * 0.75;
            Ok(MagicAbsorb {
                damage: after_effects * 0.5,
                mob_effects: damage - after_effects,
                resisted: true,
                after_effects,
                // `combat_rules::after_magic`: 1 - 12.5 / 25 is one half.
                protection: Some(12.5),
            })
        }
        fn absorption(&mut self) -> Result<f32, ()> {
            Ok(self.absorption)
        }
        fn set_absorption(&mut self, value: f32) -> Result<(), ()> {
            self.absorption = value;
            Ok(())
        }
        fn server_player_attacker(&mut self) -> Result<Option<()>, ()> {
            Ok(None)
        }
        fn award_attacker(&mut self, _: (), _: DamageStat, _: i32) -> Result<(), ()> {
            Ok(())
        }
        fn award_self(&mut self, _: DamageStat, _: i32) -> Result<(), ()> {
            Ok(())
        }
        fn source_food_exhaustion(&mut self) -> Result<f32, ()> {
            Ok(0.0)
        }
        fn cause_food_exhaustion(&mut self, _: f32) -> Result<(), ()> {
            Ok(())
        }
        fn record_damage(&mut self, _: f32) -> Result<(), ()> {
            Ok(())
        }
        fn health(&mut self) -> Result<f32, ()> {
            Ok(self.health)
        }
        fn set_health(&mut self, value: f32) -> Result<(), ()> {
            self.health = value;
            Ok(())
        }
        fn damage_event(&mut self) -> Result<(), ()> {
            Ok(())
        }
        fn damage_pre(&mut self, container: &mut DamageContainer) -> Result<(), ()> {
            self.pre_damage.push(container.new_damage());
            Ok(())
        }
    }
    fn hit(target: &mut Target, container: &mut DamageContainer, damage: f32) {
        actually_hurt(target, container, damage, true).unwrap();
    }
    fn reductions(c: &DamageContainer) -> [f32; 5] {
        [
            c.reduction(Reduction::Armor),
            c.reduction(Reduction::MobEffects),
            c.reduction(Reduction::Enchantments),
            c.reduction(Reduction::Absorption),
            c.reduction(Reduction::Invulnerability),
        ]
    }
    #[test]
    fn a_hit_with_no_modifier_takes_each_reduction_by_hand() {
        // 20: armor takes 10, Resistance 2.5 of the 10, protection half of the 7.5 left.
        let (mut target, mut c) = (Target::new(0.0), DamageContainer::new(20.0));
        hit(&mut target, &mut c, 20.0);
        assert_eq!(reductions(&c), [10.0, 2.5, 3.75, 0.0, 0.0]);
        assert_eq!(target.pre_damage, [3.75]);
        assert_eq!(c.health_damage(), 3.75);
        assert_eq!(target.health, 96.25);
        assert_eq!(c.original(), 20.0);
    }
    #[test]
    fn several_modifiers_on_one_reduction_apply_in_the_order_added() {
        // Armor took 10: x0.5 then +2 is 7, +2 then x0.5 is 6.
        let (mut target, mut c) = (Target::new(0.0), DamageContainer::new(20.0));
        c.add_reduction_modifier(Reduction::Armor, ReductionChange::Scale(0.5));
        c.add_reduction_modifier(Reduction::Armor, ReductionChange::Add(2.0));
        hit(&mut target, &mut c, 20.0);
        // 13 goes on: Resistance takes 3.25, protection half of 9.75.
        assert_eq!(reductions(&c), [7.0, 3.25, 4.875, 0.0, 0.0]);
        assert_eq!(c.health_damage(), 4.875);
        let (mut target, mut c) = (Target::new(0.0), DamageContainer::new(20.0));
        c.add_reduction_modifier(Reduction::Armor, ReductionChange::Add(2.0));
        c.add_reduction_modifier(Reduction::Armor, ReductionChange::Scale(0.5));
        hit(&mut target, &mut c, 20.0);
        // 14 goes on: Resistance takes 3.5, protection half of 10.5.
        assert_eq!(reductions(&c), [6.0, 3.5, 5.25, 0.0, 0.0]);
        assert_eq!(c.health_damage(), 5.25);
    }
    #[test]
    fn modifiers_on_different_reductions_each_change_their_own() {
        let (mut target, mut c) = (Target::new(3.0), DamageContainer::new(20.0));
        c.add_reduction_modifier(Reduction::Armor, ReductionChange::AtMost(4.0));
        c.add_reduction_modifier(Reduction::MobEffects, ReductionChange::Add(1.0));
        c.add_reduction_modifier(Reduction::Enchantments, ReductionChange::Add(1.0));
        c.add_reduction_modifier(Reduction::Absorption, ReductionChange::Scale(0.5));
        hit(&mut target, &mut c, 20.0);
        // Armor 4 leaves 16; Resistance 4 + 1 leaves 11; protection half of 11 is 5.5, + 1 is
        // 6.5, leaving 4.5; the 3 hearts' 3 halved is 1.5, so 3 reaches the health.
        assert_eq!(reductions(&c), [4.0, 5.0, 6.5, 1.5, 0.0]);
        assert_eq!(target.pre_damage, [4.5]);
        assert_eq!(c.inflicted(), 4.5);
        assert_eq!(c.health_damage(), 3.0);
        assert_eq!(target.health, 97.0);
        assert_eq!(target.absorption, 1.5);
        assert_eq!(c.original(), 20.0);
    }
    #[test]
    fn a_changed_resistance_changes_what_the_enchantments_start_from() {
        // Resistance took 2.5 of 10: x2 is 5, so 5 goes on and protection takes half of it.
        let (mut target, mut c) = (Target::new(0.0), DamageContainer::new(20.0));
        c.add_reduction_modifier(Reduction::MobEffects, ReductionChange::Scale(2.0));
        hit(&mut target, &mut c, 20.0);
        assert_eq!(reductions(&c), [10.0, 5.0, 2.5, 0.0, 0.0]);
        assert_eq!(c.health_damage(), 2.5);
    }
    #[test]
    fn a_modified_amount_stays_within_what_there_was_to_take() {
        // Armor could take at most the 20, so nothing is left; a negative result takes none.
        let (mut target, mut c) = (Target::new(0.0), DamageContainer::new(20.0));
        c.add_reduction_modifier(Reduction::Armor, ReductionChange::Add(100.0));
        hit(&mut target, &mut c, 20.0);
        assert_eq!(c.reduction(Reduction::Armor), 20.0);
        assert_eq!(c.health_damage(), 0.0);
        assert_eq!(target.health, 100.0);
        let (mut target, mut c) = (Target::new(0.0), DamageContainer::new(20.0));
        c.add_reduction_modifier(Reduction::Armor, ReductionChange::Add(-100.0));
        hit(&mut target, &mut c, 20.0);
        // Armor took none, so 20 goes on: Resistance 5, protection half of 15.
        assert_eq!(reductions(&c), [0.0, 5.0, 7.5, 0.0, 0.0]);
        // A modified absorption never takes more than the hearts hold.
        let (mut target, mut c) = (Target::new(2.0), DamageContainer::new(20.0));
        c.add_reduction_modifier(Reduction::Absorption, ReductionChange::Add(50.0));
        hit(&mut target, &mut c, 20.0);
        assert_eq!(c.reduction(Reduction::Absorption), 2.0);
        assert_eq!(target.absorption, 0.0);
        assert_eq!(c.health_damage(), 1.75);
    }
    #[test]
    fn a_modifier_that_is_not_a_number_changes_nothing() {
        let (mut target, mut c) = (Target::new(0.0), DamageContainer::new(20.0));
        c.add_reduction_modifier(Reduction::Armor, ReductionChange::Scale(f32::NAN));
        hit(&mut target, &mut c, 20.0);
        assert_eq!(reductions(&c), [10.0, 2.5, 3.75, 0.0, 0.0]);
    }
    #[test]
    fn a_skipped_step_runs_no_modifier() {
        let mut target = Target::new(0.0);
        target.bypass_armor = true;
        let mut c = DamageContainer::new(20.0);
        c.add_reduction_modifier(Reduction::Armor, ReductionChange::Add(5.0));
        hit(&mut target, &mut c, 20.0);
        // Armor took nothing and the 20 went on: Resistance 5, protection half of 15.
        assert_eq!(reductions(&c), [0.0, 5.0, 7.5, 0.0, 0.0]);
    }
    #[test]
    fn modifiers_last_for_their_own_hit() {
        let mut target = Target::new(0.0);
        let mut first = DamageContainer::new(20.0);
        first.add_reduction_modifier(Reduction::Armor, ReductionChange::Add(5.0));
        hit(&mut target, &mut first, 20.0);
        assert_eq!(first.reduction(Reduction::Armor), 15.0);
        let mut second = DamageContainer::new(20.0);
        hit(&mut target, &mut second, 20.0);
        assert_eq!(reductions(&second), [10.0, 2.5, 3.75, 0.0, 0.0]);
    }
    #[test]
    fn modifiers_cross_as_plain_values() {
        let mut c = DamageContainer::new(8.0);
        c.add_reduction_modifier(Reduction::Armor, ReductionChange::Scale(2.0));
        c.add_reduction_modifier(Reduction::Absorption, ReductionChange::AtLeast(1.0));
        let values = DamageValues::from(&c);
        assert_eq!(
            values.reduction_modifiers,
            [
                (Reduction::Armor, ReductionChange::Scale(2.0)),
                (Reduction::Absorption, ReductionChange::AtLeast(1.0)),
            ]
        );
        assert_eq!(DamageContainer::from(values), c);
    }
}
