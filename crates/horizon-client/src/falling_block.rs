//! FallingBlockEntity's impact damage, anvil wear and saved impact settings.
use crate::{
    mobs::{Damage as MobDamage, DamageKind, Mob},
    save::{float, number, put},
};
use horizon_core::geometry::{Aabb, Vec3};
use horizon_world::{chunk::BlockState, nbt::Tag};

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Damage {
    pub hurt_entities: bool,
    pub cancel_drop: bool,
    pub per_distance: f32,
    pub max: i32,
}
impl Default for Damage {
    fn default() -> Self {
        Self {
            hurt_entities: false,
            cancel_drop: false,
            per_distance: 0.0,
            max: 40,
        }
    }
}
impl Damage {
    pub fn anvil() -> Self {
        Self {
            hurt_entities: true,
            per_distance: 2.0,
            ..Self::default()
        }
    }
    pub fn from_nbt(tag: &Tag, anvil: bool) -> Self {
        Self {
            hurt_entities: number(tag.get("HurtEntities")).map_or(anvil, |v| v != 0),
            cancel_drop: number(tag.get("CancelDrop")).is_some_and(|v| v != 0),
            per_distance: float(tag.get("FallHurtAmount")).unwrap_or(0.0) as f32,
            max: number(tag.get("FallHurtMax")).unwrap_or(40) as i32,
        }
    }
    pub fn save(self, tag: &mut Tag) {
        for (key, value) in [
            ("HurtEntities", Tag::Byte(i8::from(self.hurt_entities))),
            ("CancelDrop", Tag::Byte(i8::from(self.cancel_drop))),
            ("FallHurtAmount", Tag::Float(self.per_distance.to_bits())),
            ("FallHurtMax", Tag::Int(self.max)),
        ] {
            put(tag, key, value);
        }
    }
    pub fn impact(self, fall_distance: f64) -> Option<Impact> {
        if !self.hurt_entities {
            return None;
        }
        let steps = (fall_distance - 1.0).ceil() as i32;
        if steps < 0 {
            return None;
        }
        let amount = ((steps as f32 * self.per_distance).floor() as i32).min(self.max) as f32;
        Some(Impact {
            amount,
            wear_chance: if amount > 0.0 {
                0.05 + steps as f32 * 0.05
            } else {
                0.0
            },
        })
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Impact {
    pub amount: f32,
    pub wear_chance: f32,
}

/// AnvilBlock.damage: advance wear without changing facing; false means broken.
pub fn wear_anvil(state: &mut BlockState) -> bool {
    state.name = match state.name.as_str() {
        "minecraft:anvil" => "minecraft:chipped_anvil",
        "minecraft:chipped_anvil" => "minecraft:damaged_anvil",
        _ => return false,
    }
    .into();
    true
}

pub fn bounds(position: Vec3) -> Aabb {
    let size = f64::from(0.98_f32);
    Aabb::new(
        Vec3::new(position.x - size / 2.0, position.y, position.z - size / 2.0),
        Vec3::new(
            position.x + size / 2.0,
            position.y + size,
            position.z + size / 2.0,
        ),
    )
}
pub fn hurt_mobs(mobs: &mut [Mob], position: Vec3, amount: f32) {
    if amount <= 0.0 {
        return;
    }
    let bounds = bounds(position);
    let source = MobDamage {
        kind: DamageKind::FallingBlock,
        attacker: None,
        from: Some([position.x, position.y, position.z]),
    };
    for mob in mobs
        .iter_mut()
        .filter(|m| m.alive() && bounds.intersects(m.bounds()))
    {
        mob.damage(amount, source);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn impact_uses_ceil_distance_capped_damage_and_positive_damage_wear() {
        let damage = Damage::anvil();
        assert!(Damage::default().impact(50.0).is_none());
        assert!(damage.impact(0.0).is_none());
        let short = damage.impact(1.0).unwrap();
        assert_eq!((short.amount, short.wear_chance), (0.0, 0.0));
        assert_eq!(damage.impact(1.01).unwrap().amount, 2.0);
        assert_eq!(damage.impact(5.0).unwrap().amount, 8.0);
        assert_eq!(damage.impact(100.0).unwrap().amount, 40.0);
        assert!(damage.impact(21.0).unwrap().wear_chance > 1.0);
    }
    #[test]
    fn damage_settings_round_trip_and_keep_vanilla_load_defaults() {
        let empty = Tag::Compound(Default::default());
        assert_eq!(Damage::from_nbt(&empty, false), Damage::default());
        let loaded_anvil = Damage::from_nbt(&empty, true);
        assert!(loaded_anvil.hurt_entities);
        assert_eq!(loaded_anvil.per_distance, 0.0);
        let damage = Damage {
            hurt_entities: true,
            cancel_drop: true,
            per_distance: 3.5,
            max: 17,
        };
        let mut tag = empty;
        damage.save(&mut tag);
        assert_eq!(Damage::from_nbt(&tag, false), damage);
    }
    #[test]
    fn anvil_wears_twice_then_breaks_preserving_facing() {
        for facing in ["north", "south", "east", "west"] {
            let mut state = BlockState {
                name: "minecraft:anvil".into(),
                properties: [("facing".into(), facing.into())].into(),
            };
            for name in ["minecraft:chipped_anvil", "minecraft:damaged_anvil"] {
                assert!(wear_anvil(&mut state));
                assert_eq!(state.name, name);
                assert_eq!(state.properties["facing"], facing);
            }
            assert!(!wear_anvil(&mut state));
        }
    }
    #[test]
    #[ignore = "requires prepared entity models/data in HORIZON_TEST_RESOURCES"]
    fn impact_hurts_only_living_intersecting_noninvulnerable_mobs() {
        use crate::mobs::{Mob, MobTypes};
        use std::{path::Path, sync::Arc};
        let root = std::env::var_os("HORIZON_TEST_RESOURCES").expect("HORIZON_TEST_RESOURCES");
        let types = MobTypes::load(Path::new(&root)).unwrap();
        let cow = Arc::new(types.types["minecraft:cow"].clone());
        let mut mobs = vec![
            Mob::new(1, cow.clone(), Vec3::ZERO, 0.0),
            Mob::new(2, cow.clone(), Vec3::new(3.0, 0.0, 0.0), 0.0),
            Mob::new(3, cow.clone(), Vec3::ZERO, 0.0),
            Mob::new(4, cow, Vec3::ZERO, 0.0),
        ];
        mobs[2].invulnerable_flag = true;
        mobs[3].extra.head = Some("minecraft:carved_pumpkin".into());
        let initial = mobs.iter().map(|m| m.health).collect::<Vec<_>>();
        hurt_mobs(
            &mut mobs,
            Vec3::ZERO,
            Damage::anvil().impact(5.0).unwrap().amount,
        );
        assert_eq!(mobs[0].health, initial[0] - 8.0);
        assert_eq!(mobs[1].health, initial[1]);
        assert_eq!(mobs[2].health, initial[2]);
        assert_eq!(mobs[3].health, initial[3] - 6.0);
    }
}
