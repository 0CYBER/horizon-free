//! Scalar-only damage snapshots at the subscribed combat boundary.
use super::{TryIntoApi, TryIntoEngine};
use horizon_world::living_damage as engine;
use mod_api::{DamageContainer, DamageValues, Reduction};
impl TryIntoApi for engine::DamageContainer {
    type Api = DamageContainer;
    type Error = std::convert::Infallible;
    fn try_into_api(&self) -> Result<Self::Api, Self::Error> {
        #[cfg(test)]
        CONVERSIONS.with(|count| count.set(count.get() + 1));
        let v = engine::DamageValues::from(self);
        let mut values = DamageValues::from(&DamageContainer::new(v.original));
        values.reduction_modifiers = v
            .reduction_modifiers
            .iter()
            .map(|(r, c)| {
                (
                    match r {
                        engine::Reduction::Invulnerability => Reduction::Invulnerability,
                        engine::Reduction::Armor => Reduction::Armor,
                        engine::Reduction::Enchantments => Reduction::Enchantments,
                        engine::Reduction::MobEffects => Reduction::MobEffects,
                        engine::Reduction::Absorption => Reduction::Absorption,
                        engine::Reduction::InnateResistance => Reduction::InnateResistance,
                        _ => unreachable!("unknown engine reduction"),
                    },
                    match c {
                        engine::ReductionChange::Add(v) => mod_api::ReductionChange::Add(*v),
                        engine::ReductionChange::Scale(v) => mod_api::ReductionChange::Scale(*v),
                        engine::ReductionChange::AtLeast(v) => {
                            mod_api::ReductionChange::AtLeast(*v)
                        }
                        engine::ReductionChange::AtMost(v) => mod_api::ReductionChange::AtMost(*v),
                    },
                )
            })
            .collect();
        values.original = v.original;
        values.new_damage = v.new_damage;
        values.blocked = v.blocked;
        values.shield_damage = v.shield_damage;
        values.post_attack_invulnerability_ticks = v.post_attack_invulnerability_ticks;
        values.should_cause_side_effects = v.should_cause_side_effects;
        values.entered_pre = v.entered_pre;
        values.inflicted = v.inflicted;
        values.health_damage = v.health_damage;
        values.set_reduction(
            Reduction::Invulnerability,
            v.reductions[engine::Reduction::Invulnerability as usize],
        );
        values.set_reduction(
            Reduction::Armor,
            v.reductions[engine::Reduction::Armor as usize],
        );
        values.set_reduction(
            Reduction::Enchantments,
            v.reductions[engine::Reduction::Enchantments as usize],
        );
        values.set_reduction(
            Reduction::MobEffects,
            v.reductions[engine::Reduction::MobEffects as usize],
        );
        values.set_reduction(
            Reduction::Absorption,
            v.reductions[engine::Reduction::Absorption as usize],
        );
        values.set_reduction(
            Reduction::InnateResistance,
            v.reductions[engine::Reduction::InnateResistance as usize],
        );
        Ok(values.into())
    }
}
impl TryIntoEngine<()> for DamageContainer {
    type Engine = engine::DamageContainer;
    type Error = std::convert::Infallible;
    fn try_into_engine(&self, _: &()) -> Result<Self::Engine, Self::Error> {
        let v = DamageValues::from(self);
        Ok(engine::DamageValues {
            reduction_modifiers: v
                .reduction_modifiers
                .iter()
                .map(|(r, c)| {
                    (
                        match r {
                            Reduction::Invulnerability => engine::Reduction::Invulnerability,
                            Reduction::Armor => engine::Reduction::Armor,
                            Reduction::Enchantments => engine::Reduction::Enchantments,
                            Reduction::MobEffects => engine::Reduction::MobEffects,
                            Reduction::Absorption => engine::Reduction::Absorption,
                            Reduction::InnateResistance => engine::Reduction::InnateResistance,
                            _ => unreachable!("unknown API reduction"),
                        },
                        match c {
                            mod_api::ReductionChange::Add(v) => engine::ReductionChange::Add(*v),
                            mod_api::ReductionChange::Scale(v) => {
                                engine::ReductionChange::Scale(*v)
                            }
                            mod_api::ReductionChange::AtLeast(v) => {
                                engine::ReductionChange::AtLeast(*v)
                            }
                            mod_api::ReductionChange::AtMost(v) => {
                                engine::ReductionChange::AtMost(*v)
                            }
                            _ => unreachable!("unknown API modifier"),
                        },
                    )
                })
                .collect(),
            original: v.original,
            new_damage: v.new_damage,
            blocked: v.blocked,
            shield_damage: v.shield_damage,
            post_attack_invulnerability_ticks: v.post_attack_invulnerability_ticks,
            should_cause_side_effects: v.should_cause_side_effects,
            entered_pre: v.entered_pre,
            inflicted: v.inflicted,
            health_damage: v.health_damage,
            reductions: [
                v.reduction(Reduction::Invulnerability),
                v.reduction(Reduction::Armor),
                v.reduction(Reduction::Enchantments),
                v.reduction(Reduction::MobEffects),
                v.reduction(Reduction::Absorption),
                v.reduction(Reduction::InnateResistance),
            ],
        }
        .into())
    }
}

#[cfg(test)]
thread_local! { pub(in crate::game) static CONVERSIONS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) }; }

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn snapshot_preserves_reductions_and_engine_normalization() {
        let mut engine = engine::DamageContainer::new(17.0);
        engine.record_reduction(engine::Reduction::Invulnerability, 2.0);
        engine.record_reduction(engine::Reduction::Armor, 4.0);
        engine.record_reduction(engine::Reduction::Enchantments, 0.5);
        engine.record_reduction(engine::Reduction::MobEffects, 0.75);
        engine.record_reduction(engine::Reduction::Absorption, 1.0);
        engine.record_reduction(engine::Reduction::InnateResistance, 1.5);
        engine.enter_pre(11.5);
        engine.record_block(2.0);
        engine.record_shield_damage(3.0);
        engine.record_should_cause_side_effects(true);
        engine.add_reduction_modifier(
            engine::Reduction::Armor,
            engine::ReductionChange::Scale(0.5),
        );
        let mut api = engine.try_into_api().unwrap();
        assert_eq!(
            api.reduction_modifiers().collect::<Vec<_>>(),
            vec![(Reduction::Armor, mod_api::ReductionChange::Scale(0.5))]
        );
        api.add_reduction_modifier(Reduction::Armor, mod_api::ReductionChange::Add(2.0));
        let adjusted = api.try_into_engine(&()).unwrap();
        assert_eq!(
            engine::DamageValues::from(&adjusted).reduction_modifiers,
            vec![
                (
                    engine::Reduction::Armor,
                    engine::ReductionChange::Scale(0.5)
                ),
                (engine::Reduction::Armor, engine::ReductionChange::Add(2.0))
            ]
        );
        assert_eq!(api.reduction(Reduction::Invulnerability), 2.0);
        assert_eq!(api.reduction(Reduction::Armor), 4.0);
        assert_eq!(api.reduction(Reduction::Enchantments), 0.5);
        assert_eq!(api.reduction(Reduction::MobEffects), 0.75);
        assert_eq!(api.reduction(Reduction::Absorption), 1.0);
        assert_eq!(api.reduction(Reduction::InnateResistance), 1.5);
        assert_eq!((api.blocked(), api.shield_damage()), (2.0, 3.0));
        assert!(api.should_cause_side_effects());
        api.set_new_damage(f32::NAN);
        api.set_post_attack_invulnerability_ticks(9);
        let mut engine = api.try_into_engine(&()).unwrap();
        assert!(engine.new_damage().is_nan());
        assert_eq!(engine.capture_inflicted(), 11.5);
        assert_eq!(engine.post_attack_invulnerability_ticks(), 9);
        assert_eq!(engine.reduction(engine::Reduction::Armor), 4.0);
        api.set_new_damage(-5.0);
        let mut engine = api.try_into_engine(&()).unwrap();
        assert_eq!(engine.new_damage(), -5.0);
        assert_eq!(engine.capture_inflicted(), 0.0);
    }
}
