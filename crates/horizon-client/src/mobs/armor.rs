//! Effective mob armor and toughness at the damage boundary.
use super::Mob;
use super::finalize::{self, Attr};
use horizon_world::attribute_map::ValueKind;
use horizon_world::attributes::entity::{Catalog, EntityAttributes};
use horizon_world::attributes::{Instance, Observer};
use std::convert::Infallible;
use std::sync::Arc;

/// Restore saved shulker armor at admission and after invalidating writes,
/// before damage can read it without an attribute query or equipment tick.
pub fn prepare(mob: &Mob, catalog: &Arc<Catalog>) {
    if mob.species() == super::Species::Shulker {
        mob.extra
            .live
            .read(
                catalog,
                &mob.kind.id,
                "minecraft:armor",
                ValueKind::Computed,
                || {
                    crate::entity_nbt::attribute_entries(mob)
                        .into_iter()
                        .map(|(_, entry)| entry)
                        .collect()
                },
            )
            .expect("a shulker has an armor attribute");
    }
}

struct Calculation;
impl Observer for Calculation {
    type Error = Infallible;
    fn dirty(&mut self, _: &mut Instance) -> Result<(), Infallible> {
        Ok(())
    }
}

/// The attribute's value on the mob's own map, with `bonus` counted in the
/// additive phase; none without a map, or for an attribute its catalog lacks.
fn effective(map: Option<&EntityAttributes>, name: &str, base: f64, bonus: f64) -> Option<f64> {
    let map = map?;
    let key = map.catalog().id(name)?;
    if let Some(source) = map.instance(key) {
        // Equipment and the covered shell contribute in the additive
        // phase, before multiplicative modifiers and effective clamping.
        let mut value = source.clone();
        value
            .set_base_value(source.base_value() + bonus, &mut Calculation)
            .unwrap();
        return Some(value.value());
    }
    let definition = map.catalog().attribute(key).definition;
    Some(definition.sanitize(map.base_value(key).unwrap_or(base) + bonus))
}

/// LivingEntity.getArmorValue floors the effective double before converting
/// its integer result to float. Toughness converts its effective double directly.
///
/// The live attribute map answers first (arismoko/horizon#941): the type's base, what
/// `/attribute` and the save left, a mod's modifiers, and every worn or held
/// stack's own `attribute_modifiers`, components included — the same map a
/// `LivingRef` reads, which the tick's equipment pass keeps current. A
/// shulker's `minecraft:covered` lives on that map too. Without a map yet
/// (nothing equipped, nothing written), the type's own armor fields and the
/// `/attribute` armor map answer, as before.
pub fn values(mob: &Mob) -> (f32, f32) {
    if let Some(live) = mob.extra.live.built() {
        let toughness = live.key("minecraft:armor_toughness").and_then(|key| {
            live.value(Some(key), horizon_world::attribute_map::ValueKind::Computed)
                .ok()
        });
        let armor = live.key("minecraft:armor").and_then(|key| {
            live.value(Some(key), horizon_world::attribute_map::ValueKind::Computed)
                .ok()
        });
        // The computed values are already inside the catalog's bounds.
        if let (Some(armor), Some(toughness)) = (armor, toughness) {
            return (armor.floor() as i32 as f32, toughness as f32);
        }
    }
    let map = mob.extra.armor_attributes.as_deref();
    let armor_base =
        finalize::base_value_with(mob, Attr::Armor, || Some(f64::from(mob.kind.armor)));
    let armor = effective(map, "minecraft:armor", armor_base, 0.0)
        .unwrap_or_else(|| mob.kind.attribute_bounds.sanitize(Attr::Armor, armor_base));
    // Without a map, the type's own toughness: its row's value, which nothing
    // writes past the bounds.
    let toughness_base = f64::from(mob.kind.armor_toughness);
    let toughness =
        effective(map, "minecraft:armor_toughness", toughness_base, 0.0).unwrap_or(toughness_base);
    (armor.floor() as i32 as f32, toughness as f32)
}

/// Absorption's `max_absorption` modifier (`MobEffects.ABSORPTION`:
/// +4 per level, `ADD_VALUE`) for the effects a mob has.
pub fn absorption_bonus(effects: &[super::StoredEffect]) -> f64 {
    effects
        .iter()
        .find(|e| e.id == "minecraft:absorption")
        .map_or(0.0, |e| 4.0 * f64::from(e.amplifier + 1))
}
/// `getMaxAbsorption` from a mob's attribute map (its base and explicit
/// modifiers, `/attribute` keeps them there) and the Absorption effect's
/// `ADD_VALUE` modifier. `AttributeInstance.calculateValue` sums `ADD_VALUE`
/// into the base before the multiplying operations, so the effect counts in
/// that phase; the stored base and modifiers are not changed.
/// None for a catalog without the attribute.
pub fn max_absorption_of(map: &EntityAttributes, default: f64, bonus: f64) -> Option<f64> {
    effective(Some(map), "minecraft:max_absorption", default, bonus)
}
/// A mob's `getMaxAbsorption` (see [`max_absorption_of`]).
pub fn max_absorption(mob: &Mob) -> f32 {
    let default = mob
        .kind
        .attributes
        .get("minecraft:max_absorption")
        .copied()
        .unwrap_or(0.0);
    let bonus = absorption_bonus(&mob.extra.effects);
    let map = mob.extra.armor_attributes.as_deref();
    map.and_then(|map| max_absorption_of(map, default, bonus))
        .unwrap_or_else(|| {
            mob.kind
                .attribute_bounds
                .sanitize_max_absorption(default + bonus)
        }) as f32
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mobs::{Damage, DamageKind, Species};
    use horizon_core::geometry::Vec3;
    use horizon_world::attributes::{Modifier, Operation, entity::Catalog};
    use std::sync::Arc;

    /// A pig whose live attribute map holds `armor` and `toughness` bases, as
    /// `/attribute` writes and the save leave them, with its catalog for edits.
    fn pig(armor: f64, toughness: f64) -> (Mob, Arc<Catalog>) {
        let bits = |v: f64| format!("{:016x}", v.to_bits());
        let catalog = Arc::new(Catalog::from_json(&serde_json::json!({
            "data_version": horizon_world::DATA_VERSION,
            "attributes": [
                {"id":"minecraft:armor", "default":bits(0.0), "min":bits(0.0), "max":bits(30.0), "syncable":true},
                {"id":"minecraft:armor_toughness", "default":bits(0.0), "min":bits(0.0), "max":bits(20.0), "syncable":true}
            ],
            "entities": {"minecraft:pig":{"minecraft:armor":bits(0.0), "minecraft:armor_toughness":bits(0.0)}}
        })).unwrap());
        let mut live = crate::living_attributes::LivingAttributes::default();
        for (name, base) in [
            ("minecraft:armor", armor),
            ("minecraft:armor_toughness", toughness),
        ] {
            live.edit(
                &catalog,
                "minecraft:pig",
                name,
                true,
                Vec::new,
                |value, observer| value.set_base_value(base, observer),
            )
            .unwrap();
        }
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let ty = Arc::make_mut(&mut kind);
        ty.id = "minecraft:pig".into();
        ty.species = Species::Pig;
        let mut mob = Mob::new(1, kind, Vec3::ZERO, 0.0);
        mob.extra.live = live;
        mob.attrs.max_health = 1000.0;
        mob.health = 1000.0;
        (mob, catalog)
    }

    #[test]
    fn effective_mob_armor_controls_damage_and_fractional_armor_floors() {
        let mut test_level = crate::health::TestLevel::new(0);
        let level = &mut test_level.level();
        for (armor, toughness, source, expected) in [
            (20.0, 8.0, DamageKind::Cactus, 999039),
            (0.0, 8.0, DamageKind::Cactus, 996000),
            (0.5, 8.0, DamageKind::Cactus, 996000),
            (20.0, 8.0, DamageKind::SonicBoom, 996000),
        ] {
            let (mut mob, _) = pig(armor, toughness);
            assert!(mob.damage(level, 4.0, Damage::of(source)));
            assert_eq!((f64::from(mob.health) * 1000.0) as i32, expected);
        }
    }

    /// A worn stack's modifiers are the live map's transient rows, whatever
    /// changed them: a mod's, `/item`'s or `set_equipment`'s (arismoko/horizon#941).
    fn wear(mob: &mut Mob, catalog: &Arc<Catalog>, amount: f64, operation: Operation) {
        mob.extra
            .live
            .edit(
                catalog,
                "minecraft:pig",
                "minecraft:armor",
                true,
                Vec::new,
                |value, observer| {
                    value.add_transient(
                        Modifier::new("minecraft:armor.helmet", amount, operation).unwrap(),
                        observer,
                    )
                },
            )
            .unwrap();
    }

    #[test]
    fn equipment_is_counted_once_before_effective_limits() {
        let (mut mob, catalog) = pig(20.0, 8.0);
        wear(&mut mob, &catalog, 3.0, Operation::AddValue);
        assert_eq!(values(&mob), (23.0, 8.0));
        let (mut mob, catalog) = pig(10.0, 8.0);
        wear(&mut mob, &catalog, 0.5, Operation::AddMultipliedTotal);
        assert_eq!(values(&mob), (15.0, 8.0));
        let (mob, _) = pig(100.0, 100.0);
        assert_eq!(values(&mob), (30.0, 20.0));
    }

    /// A mob that never built a map answers from its type's armor fields, and
    /// one whose type lacks the attributes from the range's floor.
    #[test]
    fn a_mob_without_a_map_answers_its_types_fields() {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let ty = Arc::make_mut(&mut kind);
        ty.armor = 7.0;
        ty.armor_toughness = 2.0;
        let mob = Mob::new(1, kind, Vec3::ZERO, 0.0);
        assert_eq!(values(&mob), (7.0, 2.0));
        assert!(mob.extra.live.built().is_none());
    }
    /// The effect bonus counted in the `ADD_VALUE` phase gives what an
    /// `Instance` carrying it as one more `ADD_VALUE` modifier gives, under
    /// the multiplying operations and the range clamp (the cases the pinned
    /// 26.3 server answered 36, 28, 60, 2042 and 2048 for), and the stored
    /// base and modifiers stay as they were.
    #[test]
    fn absorption_bonus_is_an_add_value_modifier_under_every_operation() {
        use horizon_world::attributes::{Modifier, Operation};
        let bits = |v: f64| format!("{:016x}", v.to_bits());
        let catalog = Arc::new(Catalog::from_json(&serde_json::json!({
            "data_version": horizon_world::DATA_VERSION,
            "attributes": [
                {"id":"minecraft:max_absorption", "default":bits(0.0), "min":bits(0.0), "max":bits(2048.0), "syncable":true}
            ],
            "entities": {"minecraft:pig":{"minecraft:max_absorption":bits(0.0)}}
        })).unwrap());
        let key = catalog.id("minecraft:max_absorption").unwrap();
        let cases: [(f64, &[(f64, Operation)], f64, f64); 5] = [
            (20.0, &[(0.5, Operation::AddMultipliedBase)], 4.0, 36.0),
            (10.0, &[(1.0, Operation::AddMultipliedTotal)], 4.0, 28.0),
            (
                10.0,
                &[
                    (2.0, Operation::AddValue),
                    (0.5, Operation::AddMultipliedBase),
                    (1.0, Operation::AddMultipliedTotal),
                ],
                8.0,
                60.0,
            ),
            (2048.0, &[(-10.0, Operation::AddValue)], 4.0, 2042.0),
            (2046.0, &[], 4.0, 2048.0),
        ];
        for (base, mods, bonus, java) in cases {
            let mut map = catalog.create("minecraft:pig").unwrap();
            map.edit(key, |value, observer| {
                value.set_base_value(base, observer).unwrap();
                for (n, (amount, op)) in mods.iter().enumerate() {
                    let m = Modifier::new(&format!("h203:m{n}"), *amount, *op).unwrap();
                    value.add_permanent(m, observer).unwrap();
                }
            });
            let mut with_effect = map.instance(key).unwrap().clone();
            with_effect
                .add_permanent(
                    Modifier::new("minecraft:effect.absorption", bonus, Operation::AddValue)
                        .unwrap(),
                    &mut Calculation,
                )
                .unwrap();
            let got = max_absorption_of(&map, 0.0, bonus).unwrap();
            assert_eq!(got, with_effect.value(), "base {base}");
            assert_eq!(got, java, "base {base}: the pinned Java value");
            assert_eq!(
                map.base_value(key).unwrap(),
                base,
                "the stored base is unchanged"
            );
            assert_eq!(
                max_absorption_of(&map, 0.0, 0.0).unwrap(),
                map.value(key).unwrap()
            );
        }
        // No map yet: the type default with the bonus, in the bounds its
        // type took from the catalog.
        let bounds = finalize::AttrBounds::of(&catalog);
        assert_eq!(bounds.sanitize_max_absorption(8.0), 8.0);
        assert_eq!(bounds.sanitize_max_absorption(-4.0), 0.0);
        assert_eq!(bounds.sanitize_max_absorption(2052.0), 2048.0);
    }

    fn magma_cube(size: i32) -> Mob {
        let mut kind = crate::mobs::sensor_scan_tests::kind();
        let ty = Arc::make_mut(&mut kind);
        ty.id = "minecraft:magma_cube".into();
        ty.species = Species::MagmaCube;
        let mut mob = Mob::new(1, kind, Vec3::ZERO, 0.0);
        crate::mobs::cube::set_size(&mut mob, size);
        mob
    }

    /// MagmaCube.setSize writes `size * 3` into `Attributes.ARMOR`, and
    /// `LivingEntity.getArmorValue` reads that base back (no armor map needed).
    #[test]
    fn magma_cube_armor_is_its_size_times_three_and_softens_hits() {
        for size in [2, 4] {
            assert_eq!(
                values(&magma_cube(size)),
                ((size * 3) as f32, 0.0),
                "size {size}"
            );
        }
        let mut small = magma_cube(1);
        let mut big = magma_cube(4);
        let hit = Damage::of(DamageKind::Cactus);
        let small_before = small.health;
        let big_before = big.health;
        assert!(small.damage(&mut crate::health::TestLevel::new(0).level(), 0.5, hit));
        assert!(big.damage(&mut crate::health::TestLevel::new(0).level(), 0.5, hit));
        assert!(
            big_before - big.health < small_before - small.health,
            "size 4 lost {} and size 1 lost {}",
            big_before - big.health,
            small_before - small.health
        );
    }
}
