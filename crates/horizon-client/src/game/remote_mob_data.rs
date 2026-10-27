//! A server's mobs as their synched data has them
//! (ClientPacketListener.handleSetEntityData, then each entity's
//! `onSyncedDataUpdated`): what a client draws from it rather than from its
//! own simulation, which a client of a server doesn't run.
use super::Game;
use horizon_client::mobs::render::{PROFESSIONS, VILLAGER_TYPES};
use horizon_client::mobs::{Mob, Species, Who, bat, tame};
use horizon_client::particles::{ParticleOptions, ParticleType};
use horizon_protocol::entity_data::{Particle, ParticleData, Value};
use horizon_protocol::registry::Builtin;

/// The values `apply` takes from one SetEntityData packet, by vanilla's entry
/// indices (Entity 0..=7, LivingEntity 8..=14, Mob 15, then the subclasses).
/// `builtin` names the ids of built-in registries.
fn apply(m: &mut Mob, values: &[(u8, i32, Value)], builtin: &Builtin, local_entity: i32) {
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
            // LivingEntity.DATA_EFFECT_PARTICLES and DATA_EFFECT_AMBIENCE_ID:
            // the swirls of its effects (`tickEffects` draws them).
            (10, Value::Particles(list)) => m.extra.effect_particles = swirls(list, builtin),
            (11, Value::Bool(ambient)) => m.extra.effect_ambient = *ambient,
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
            (Species::Guardian | Species::ElderGuardian, 16, Value::Bool(moving)) => {
                m.extra.flag2 = *moving;
            }
            (Species::Guardian | Species::ElderGuardian, 17, Value::Int(target)) => {
                // Guardian.onSyncedDataUpdated resets the visual charge clock.
                m.extra.counter2 = 0;
                m.extra.flag = *target != 0;
                m.ai.target = (*target != 0).then_some(if *target == local_entity {
                    Who::Player
                } else {
                    Who::Mob(*target as u32)
                });
            }
            // PolarBear.DATA_STANDING_ID: the client animates the rearing.
            (Species::PolarBear, 18, Value::Bool(standing)) => m.extra.flag = *standing,
            // Bat.DATA_ID_FLAGS: resting in bit 0.
            (Species::Bat, 16, Value::Byte(flags)) => bat::set_resting(m, flags & 1 != 0),
            // Sheep.DATA_WOOL_ID: the colour in the low nibble, 16 sheared.
            (Species::Sheep, 18, Value::Byte(wool)) => {
                m.extra.color = (*wool & 15) as u8;
                m.extra.sheared = wool & 16 != 0;
            }
            // SnowGolem.DATA_PUMPKIN_ID: 16 while the pumpkin is on.
            (Species::SnowGolem, 16, Value::Byte(flags)) => m.extra.sheared = flags & 16 == 0,
            // MushroomCow.DATA_TYPE: 0 red, 1 brown.
            (Species::Mooshroom, 17, Value::Int(variant)) => {
                m.extra.variant = (*variant == 1) as u8
            }
            // Piglin.DATA_IS_DANCING: the dance pose.
            (Species::Piglin, 19, Value::Bool(dancing)) => m.extra.scratch[0] = i32::from(*dancing),
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

/// The options of a synched particle list: an entity effect carries its ARGB
/// colour, the omens none.
fn swirls(list: &[Particle], builtin: &Builtin) -> Vec<ParticleOptions> {
    let channel = |argb: i32, shift: u32| ((argb >> shift) & 255) as f32 / 255.0;
    list.iter()
        .filter_map(|p| {
            let name = builtin.get("minecraft:particle_type")?.name(p.id)?;
            let ty = ParticleType::from_name(name)?;
            Some(match p.data {
                Some(ParticleData::Color(argb)) => ParticleOptions::colored(
                    ty,
                    [
                        channel(argb, 16),
                        channel(argb, 8),
                        channel(argb, 0),
                        channel(argb, 24),
                    ],
                ),
                _ => ParticleOptions::simple(ty),
            })
        })
        .collect()
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
            apply(m, values, &r.builtin, r.entity_id);
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
    if m.species() == Species::Bat {
        // Bat's constructor sets resting only on the server; the data default is 0.
        bat::set_resting(m, false);
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
        super::apply(m, values, &builtin(), 99);
    }

    fn mob(kind: &str) -> Option<Mob> {
        let types = horizon_client::mobs::MobTypes::load(&root()).ok()?;
        let t = std::sync::Arc::new(types.types.get(kind)?.clone());
        let mut m = Mob::new(1, t, horizon_core::geometry::Vec3::ZERO, 0.0);
        client_defaults(&mut m);
        Some(m)
    }

    #[test]
    fn guardian_charge_follows_target_updates_and_caps_at_its_duration() {
        use horizon_client::mobs::guardian::{beam, client_tick};
        for (kind, duration) in [("guardian", 80), ("elder_guardian", 60)] {
            let Some(mut m) = mob(&format!("minecraft:{kind}")) else {
                return;
            };
            assert!(beam(&m, 0.0).is_none());
            apply(
                &mut m,
                &[
                    (16, s::BOOLEAN, Value::Bool(true)),
                    (17, s::INT, Value::Int(99)),
                ],
            );
            assert!(m.extra.flag2);
            assert_eq!(m.ai.target, Some(Who::Player));
            assert_eq!(beam(&m, 0.0), Some((0.0, 0.0)));
            for _ in 0..duration + 10 {
                client_tick(&mut m);
            }
            assert_eq!(beam(&m, 0.0), Some((duration as f32, 1.0)));
            // Another player or a mob retains its network entity id.
            apply(&mut m, &[(17, s::INT, Value::Int(42))]);
            assert_eq!(m.ai.target, Some(Who::Mob(42)));
            assert_eq!(m.extra.counter2, 0);
            client_tick(&mut m);
            assert_eq!(m.extra.counter2, 1);
            apply(
                &mut m,
                &[
                    (16, s::BOOLEAN, Value::Bool(false)),
                    (17, s::INT, Value::Int(0)),
                ],
            );
            client_tick(&mut m);
            assert!(!m.extra.flag2);
            assert_eq!(m.ai.target, None);
            assert_eq!(m.extra.counter2, 0);
            assert!(beam(&m, 0.0).is_none());
        }
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
    fn bat_flags_select_continuous_client_clocks_without_changing_motion() {
        use horizon_client::mobs::{MobTypes, anim, keyframes::Animation};
        use horizon_core::geometry::Vec3;
        use serde_json::json;
        use std::{collections::HashMap, fs, sync::Arc};

        // Original minimal resources keep this regression runnable without vanilla data.
        let root =
            std::env::temp_dir().join(format!("horizon-bat-animation-{}", std::process::id()));
        fs::create_dir(&root).unwrap();
        fs::write(root.join("entity-types.json"), r#"{"minecraft:bat": {}}"#).unwrap();
        fs::write(
            root.join("entity-models.json"),
            r#"{"minecraft:bat#main": {"children": {"right_wing": {}, "head": {}}}}"#,
        )
        .unwrap();
        let mut types = MobTypes::load(&root).unwrap();
        fs::remove_dir_all(root).unwrap();
        let position = Vec3::new(1.0, 2.25, 3.0);
        let velocity = Vec3::new(0.2, 0.3, 0.4);
        let mut m = Mob::new(
            1,
            Arc::new(types.types.remove("minecraft:bat").unwrap()),
            position,
            0.0,
        );
        assert!(bat::resting(&m));
        client_defaults(&mut m);
        assert!(!bat::resting(&m));
        m.velocity = velocity;
        m.age = 1;
        bat::setup_animation_states(&mut m);
        m.age = 4;
        apply(&mut m, &[(16, s::BYTE, Value::Byte(2))]);
        bat::setup_animation_states(&mut m);
        assert_eq!(m.anim.states[0].started, Some(1.0));

        let curve = Animation::parse(&json!({"length": 1, "looping": true,
            "bones": {"right_wing": [{"target": "rotation", "keyframes":
                [[0,[0,0,0],[0,0,0],"linear"],[1,[0,1,0],[0,1,0],"linear"]]}]}}));
        let animations = HashMap::from([("BatAnimation.BAT_FLYING".into(), curve)]);
        let wing = |partial| {
            let mut poses = anim::Poses::new(&m.kind.model);
            anim::setup(&m, &anim::state(&m, partial), &mut poses, &animations);
            poses.get("right_wing")[4]
        };
        assert!(wing(0.5) > wing(0.0));

        m.age = 5;
        apply(&mut m, &[(16, s::BYTE, Value::Byte(3))]);
        bat::setup_animation_states(&mut m);
        assert!(bat::resting(&m));
        assert_eq!(m.anim.states[0].started, None);
        assert_eq!(m.anim.states[1].started, Some(5.0));
        m.age = 7;
        apply(&mut m, &[(16, s::BYTE, Value::Byte(0))]);
        bat::setup_animation_states(&mut m);
        assert_eq!(m.anim.states[0].started, Some(7.0));
        assert_eq!(m.anim.states[1].started, None);
        assert_eq!((m.position, m.velocity), (position, velocity));
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
