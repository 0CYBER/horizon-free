//! A server's mobs as their synched data has them
//! (ClientPacketListener.handleSetEntityData, then each entity's
//! `onSyncedDataUpdated`): what a client draws from it rather than from its
//! own simulation, which a client of a server doesn't run.
use super::Game;
use horizon_client::mobs::render::{PROFESSIONS, VILLAGER_TYPES};
use horizon_client::mobs::{Mob, Species, tame};
use horizon_protocol::entity_data::Value;
use horizon_protocol::registry::Builtin;

/// The values `apply` takes from one SetEntityData packet, by vanilla's entry
/// indices (Entity 0..=7, LivingEntity 8..=14, Mob 15, then the subclasses).
/// `builtin` names the ids of built-in registries.
fn apply(m: &mut Mob, values: &[(u8, i32, Value)], builtin: &Builtin) {
    let species = m.species();
    let baby = super::lan::baby_index(&m.kind.id);
    for (index, _, value) in values {
        match (*index, value) {
            // Entity.DATA_SHARED_FLAGS_ID's FLAG_ONFIRE: Entity.isOnFire on a
            // client, which draws the flames. A client doesn't count the
            // fire down: the flag goes off when the server's fire is out.
            (0, Value::Byte(flags)) => m.fire_ticks = i32::from(flags & 1),
            // LivingEntity.DATA_LIVING_ENTITY_FLAGS: using an item 1, in the
            // off hand 2. The flag starts and stops the use; the client ticks
            // the count itself (a bow's pull follows it).
            (8, Value::Byte(flags)) => {
                m.extra.using_off_hand = flags & 2 != 0;
                if flags & 1 == 0 {
                    m.extra.using_item = 0;
                } else if m.extra.using_item == 0 {
                    m.extra.using_item = 1;
                }
            }
            // LivingEntity.DATA_HEALTH_ID: the boss bars and the death
            // animation follow it.
            (9, Value::Float(health)) => m.health = *health,
            // Mob.DATA_MOB_FLAGS_ID: no AI 1, left-handed 2, aggressive 4.
            // Mob.isAggressive draws the illagers' and skeletons' attack
            // poses and the vindicator's axe.
            (15, Value::Byte(flags)) => {
                m.no_ai = flags & 1 != 0;
                m.extra.left_handed = flags & 2 != 0;
                m.ai.aggressive = flags & 4 != 0;
            }
            (i, Value::Bool(b)) if Some(i) == baby => m.extra.age = if *b { -1 } else { 0 },
            _ => {}
        }
        match (species, *index, value) {
            // Sheep.DATA_WOOL_ID: the colour in the low nibble, 16 sheared.
            (Species::Sheep, 18, Value::Byte(wool)) => {
                m.extra.color = (*wool & 15) as u8;
                m.extra.sheared = wool & 16 != 0;
            }
            // TamableAnimal.DATA_FLAGS_ID: sitting 1, tame 4.
            (Species::Wolf | Species::Cat | Species::Parrot, 18, Value::Byte(flags)) => {
                m.extra.sitting = flags & 1 != 0;
                m.extra.tame = flags & 4 != 0;
            }
            // Wolf.DATA_COLLAR_COLOR and Cat's.
            (Species::Wolf, 21, Value::Int(c)) | (Species::Cat, 23, Value::Int(c)) => {
                m.extra.color = (*c & 15) as u8;
            }
            // AbstractCubeMob.ID_SIZE: its size and box (refreshDimensions).
            (Species::Slime | Species::MagmaCube, 18, Value::Int(size)) => {
                m.extra.size = (*size).clamp(1, 127);
            }
            // Pillager.IS_CHARGING_CROSSBOW: the charge pose (Raider's
            // IS_CELEBRATING takes 16).
            (Species::Pillager, 17, Value::Bool(charging)) => m.extra.flag = *charging,
            // SpellcasterIllager.DATA_SPELL_CASTING_ID: the casting pose.
            (Species::Evoker | Species::Illusioner, 17, Value::Byte(spell)) => {
                m.extra.variant2 = (*spell).max(0) as u8;
            }
            // Villager.DATA_VILLAGER_DATA and ZombieVillager's (after
            // Zombie's three): what VillagerProfessionLayer draws.
            (Species::Villager, 19, Value::Ids(data))
            | (Species::ZombieVillager, 20, Value::Ids(data)) => villager_data(m, data, builtin),
            // Mannequin.DATA_PROFILE (after Avatar's two): whose skin it
            // wears; update_remote_skins resolves it like a tab player's.
            (Species::Mannequin, 17, Value::Profile(profile)) => {
                m.profile = Some(profile.clone());
            }
            _ => {}
        }
    }
}

/// VillagerData as the network sends it: the type and the profession by
/// their built-in registry ids, then the level.
fn villager_data(m: &mut Mob, data: &[i32], builtin: &Builtin) {
    let &[kind, profession, level] = data else {
        return;
    };
    let index = |registry: &str, id: i32, names: &[&str]| {
        let name = builtin
            .get(registry)?
            .name(id)?
            .strip_prefix("minecraft:")?;
        names.iter().position(|n| *n == name).map(|i| i as u8)
    };
    if let Some(t) = index("minecraft:villager_type", kind, &VILLAGER_TYPES) {
        m.extra.variant = t;
    }
    if let Some(p) = index("minecraft:villager_profession", profession, &PROFESSIONS) {
        m.extra.variant2 = p;
    }
    m.extra.variant3 = level.clamp(0, i32::from(u8::MAX)) as u8;
}

impl Game<'_> {
    /// SetEntityData for a mob of the server's.
    pub(super) fn remote_mob_data(&mut self, id: i32, values: &[(u8, i32, Value)]) {
        let Some(r) = &self.remote else {
            return;
        };
        if let Some(m) = self.mobs.iter_mut().find(|m| m.id == id as u32) {
            apply(m, values, &r.builtin);
        }
    }
}

/// A mob as a client constructs it before its data arrives: the synched
/// defaults the server leaves out (`getNonDefaultValues`) that differ from
/// what `Mob::new` gives, such as a wolf's and a cat's red collar.
pub(super) fn client_defaults(m: &mut Mob) {
    if matches!(m.species(), Species::Wolf | Species::Cat) {
        m.extra.color = tame::RED;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_protocol::entity_data::serializer as s;

    /// A mob of the prepared resources' type `kind`, as a client spawns it
    /// (None without the resources).
    fn root() -> std::path::PathBuf {
        std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("../../.local/horizon-client")
            .join(horizon_core::VANILLA_VERSION)
    }

    /// The prepared resources' built-in registries (empty without them).
    fn builtin() -> Builtin {
        Builtin::load(&root().join("registries.json")).unwrap_or_default()
    }

    fn apply(m: &mut Mob, values: &[(u8, i32, Value)]) {
        super::apply(m, values, &builtin());
    }

    fn mob(kind: &str) -> Option<Mob> {
        let types = horizon_client::mobs::MobTypes::load(&root()).ok()?;
        let t = std::sync::Arc::new(types.types.get(kind)?.clone());
        let mut m = Mob::new(1, t, horizon_core::geometry::Vec3::ZERO, 0.0);
        client_defaults(&mut m);
        Some(m)
    }

    #[test]
    fn a_sheep_takes_its_wool_and_a_calf_its_size() {
        let Some(mut m) = mob("minecraft:sheep") else {
            return;
        };
        apply(&mut m, &[(18, s::BYTE, Value::Byte(14 | 16))]);
        assert_eq!((m.extra.color, m.extra.sheared), (14, true));
        apply(&mut m, &[(18, s::BYTE, Value::Byte(0))]);
        assert_eq!((m.extra.color, m.extra.sheared), (0, false));
        let mut calf = mob("minecraft:cow").unwrap();
        apply(&mut calf, &[(16, s::BOOLEAN, Value::Bool(true))]);
        assert!(calf.is_baby());
        apply(&mut calf, &[(16, s::BOOLEAN, Value::Bool(false))]);
        assert!(!calf.is_baby());
    }

    #[test]
    fn a_wolf_takes_its_owner_flags_and_collar() {
        let Some(mut m) = mob("minecraft:wolf") else {
            return;
        };
        assert_eq!(m.extra.color, tame::RED);
        apply(
            &mut m,
            &[
                (18, s::BYTE, Value::Byte(4 | 1)),
                (21, s::INT, Value::Int(11)),
            ],
        );
        assert!(m.extra.tame && m.extra.sitting);
        assert_eq!(m.extra.color, 11);
    }

    #[test]
    fn fire_health_and_size_follow_the_server() {
        let Some(mut m) = mob("minecraft:slime") else {
            return;
        };
        apply(
            &mut m,
            &[
                (0, s::BYTE, Value::Byte(1)),
                (9, s::FLOAT, Value::Float(12.5)),
                (18, s::INT, Value::Int(4)),
            ],
        );
        assert_eq!((m.fire_ticks, m.health, m.extra.size), (1, 12.5, 4));
        apply(&mut m, &[(0, s::BYTE, Value::Byte(0))]);
        assert_eq!(m.fire_ticks, 0);
    }

    #[test]
    fn mob_flags_turn_aggression_on_and_off() {
        let Some(mut m) = mob("minecraft:vindicator") else {
            return;
        };
        apply(&mut m, &[(15, s::BYTE, Value::Byte(4 | 2))]);
        assert!(m.ai.aggressive && m.extra.left_handed && !m.no_ai);
        assert!(horizon_client::mobs::illager::shows_item(&m));
        apply(&mut m, &[(15, s::BYTE, Value::Byte(0))]);
        assert!(!m.ai.aggressive && !m.extra.left_handed);
        assert!(!horizon_client::mobs::illager::shows_item(&m));
    }

    #[test]
    fn a_villager_takes_its_type_profession_and_level() {
        let Some(mut m) = mob("minecraft:villager") else {
            return;
        };
        let registries = builtin();
        let id = |registry: &str, name: &str| registries.get(registry)?.id(name);
        let (Some(desert), Some(farmer), Some(librarian)) = (
            id("minecraft:villager_type", "minecraft:desert"),
            id("minecraft:villager_profession", "minecraft:farmer"),
            id("minecraft:villager_profession", "minecraft:librarian"),
        ) else {
            return;
        };
        let name = |m: &Mob| {
            (
                VILLAGER_TYPES[m.extra.variant as usize],
                PROFESSIONS[m.extra.variant2 as usize],
                m.extra.variant3,
            )
        };
        apply(
            &mut m,
            &[(19, s::VILLAGER_DATA, Value::Ids(vec![desert, farmer, 2]))],
        );
        assert_eq!(name(&m), ("desert", "farmer", 2));
        // A job taken later: only the profession and the level change.
        apply(
            &mut m,
            &[(19, s::VILLAGER_DATA, Value::Ids(vec![desert, librarian, 5]))],
        );
        assert_eq!(name(&m), ("desert", "librarian", 5));
        // A zombie villager's data comes after Zombie's three entries.
        let mut z = mob("minecraft:zombie_villager").unwrap();
        apply(
            &mut z,
            &[(20, s::VILLAGER_DATA, Value::Ids(vec![desert, farmer, 3]))],
        );
        assert_eq!(name(&z), ("desert", "farmer", 3));
    }

    #[test]
    fn the_mob_flags_drive_the_aggressive_poses() {
        let Some(mut m) = mob("minecraft:skeleton") else {
            return;
        };
        apply(&mut m, &[(15, s::BYTE, Value::Byte(2 | 4))]);
        assert!(m.ai.aggressive && m.extra.left_handed && !m.no_ai);
        apply(&mut m, &[(15, s::BYTE, Value::Byte(0))]);
        assert!(!m.ai.aggressive && !m.extra.left_handed);
    }

    #[test]
    fn the_living_flags_start_and_stop_a_mob_using_an_item() {
        let Some(mut m) = mob("minecraft:skeleton") else {
            return;
        };
        apply(&mut m, &[(8, s::BYTE, Value::Byte(1))]);
        assert_eq!(m.extra.using_item, 1);
        assert!(!m.extra.using_off_hand);
        // Using in the off hand.
        apply(&mut m, &[(8, s::BYTE, Value::Byte(3))]);
        assert!(m.extra.using_off_hand);
        apply(&mut m, &[(8, s::BYTE, Value::Byte(0))]);
        assert_eq!(m.extra.using_item, 0);
    }

    #[test]
    fn a_pillager_charges_and_a_spellcaster_casts() {
        let Some(mut p) = mob("minecraft:pillager") else {
            return;
        };
        apply(&mut p, &[(17, s::BOOLEAN, Value::Bool(true))]);
        assert!(p.extra.flag);
        let mut e = mob("minecraft:evoker").unwrap();
        apply(&mut e, &[(17, s::BYTE, Value::Byte(2))]);
        assert_eq!(e.extra.variant2, 2);
    }

    #[test]
    fn a_mannequin_exists_and_takes_its_profile() {
        let Some(mut m) = mob("minecraft:mannequin") else {
            return;
        };
        assert_eq!(&*m.kind.id, "minecraft:mannequin");
        let profile = horizon_protocol::entity_data::Profile {
            name: Some("Gommunity".into()),
            uuid: Some(1),
            ..Default::default()
        };
        apply(
            &mut m,
            &[(17, s::RESOLVABLE_PROFILE, Value::Profile(profile.clone()))],
        );
        assert_eq!(m.profile, Some(profile));
    }
}
