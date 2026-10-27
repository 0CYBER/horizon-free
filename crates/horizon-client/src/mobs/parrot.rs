//! What `Parrot.aiStep` adds: imitating the mobs around it (`imitateNearbyMobs`,
//! the ambient sound of `getAmbient`) and dancing near a playing jukebox.
use super::{Mob, MobEvent, MobWorld, Species};
use horizon_core::{
    geometry::Vec3,
    random::{LegacyRandom, RandomSource},
};

/// `Parrot.MOB_SOUND_MAP` (the happy ghast's is `SoundEvents.EMPTY`: "").
const IMITATIONS: [(Species, &str); 41] = [
    (Species::Blaze, "entity.parrot.imitate.blaze"),
    (Species::Bogged, "entity.parrot.imitate.bogged"),
    (Species::Breeze, "entity.parrot.imitate.breeze"),
    (Species::CamelHusk, "entity.parrot.imitate.camel_husk"),
    (Species::CaveSpider, "entity.parrot.imitate.spider"),
    (Species::Creaking, "entity.parrot.imitate.creaking"),
    (Species::Creeper, "entity.parrot.imitate.creeper"),
    (Species::Drowned, "entity.parrot.imitate.drowned"),
    (
        Species::ElderGuardian,
        "entity.parrot.imitate.elder_guardian",
    ),
    (Species::Endermite, "entity.parrot.imitate.endermite"),
    (Species::Evoker, "entity.parrot.imitate.evoker"),
    (Species::Ghast, "entity.parrot.imitate.ghast"),
    (Species::HappyGhast, ""),
    (Species::Guardian, "entity.parrot.imitate.guardian"),
    (Species::Hoglin, "entity.parrot.imitate.hoglin"),
    (Species::Husk, "entity.parrot.imitate.husk"),
    (Species::Illusioner, "entity.parrot.imitate.illusioner"),
    (Species::MagmaCube, "entity.parrot.imitate.magma_cube"),
    (Species::Parched, "entity.parrot.imitate.parched"),
    (Species::Phantom, "entity.parrot.imitate.phantom"),
    (Species::Piglin, "entity.parrot.imitate.piglin"),
    (Species::PiglinBrute, "entity.parrot.imitate.piglin_brute"),
    (Species::Pillager, "entity.parrot.imitate.pillager"),
    (Species::Ravager, "entity.parrot.imitate.ravager"),
    (Species::Shulker, "entity.parrot.imitate.shulker"),
    (Species::Silverfish, "entity.parrot.imitate.silverfish"),
    (Species::Skeleton, "entity.parrot.imitate.skeleton"),
    (Species::Slime, "entity.parrot.imitate.slime"),
    (Species::Spider, "entity.parrot.imitate.spider"),
    (Species::Stray, "entity.parrot.imitate.stray"),
    (Species::Vex, "entity.parrot.imitate.vex"),
    (Species::Vindicator, "entity.parrot.imitate.vindicator"),
    (Species::Warden, "entity.parrot.imitate.warden"),
    (Species::Witch, "entity.parrot.imitate.witch"),
    (Species::Wither, "entity.parrot.imitate.wither"),
    (
        Species::WitherSkeleton,
        "entity.parrot.imitate.wither_skeleton",
    ),
    (Species::Zoglin, "entity.parrot.imitate.zoglin"),
    (Species::Zombie, "entity.parrot.imitate.zombie"),
    (Species::ZombieHorse, "entity.parrot.imitate.zombie_horse"),
    (
        Species::ZombieNautilus,
        "entity.parrot.imitate.zombie_nautilus",
    ),
    (
        Species::ZombieVillager,
        "entity.parrot.imitate.zombie_villager",
    ),
];

fn imitation(species: Species) -> Option<&'static str> {
    IMITATIONS.iter().find(|i| i.0 == species).map(|i| i.1)
}

/// `Parrot.getPitch`.
fn pitch(random: &mut LegacyRandom) -> f32 {
    (random.next_float() - random.next_float()) * 0.2 + 1.0
}

/// The start of `Parrot.aiStep`: a jukebox that is gone or farther than
/// 3.46 blocks stops the dance, and one tick in 400 the parrot imitates a
/// mob nearby.
pub fn ai_step(
    m: &mut Mob,
    w: &MobWorld,
    level_random: &mut LegacyRandom,
    events: &mut Vec<MobEvent>,
) {
    if let Some(at) = m.extra.party_jukebox {
        let d = |i: usize, v: f64| f64::from(at[i]) + 0.5 - v;
        let (x, y, z) = (d(0, m.position.x), d(1, m.position.y), d(2, m.position.z));
        let jukebox = &*w.facts.get(w.world.block(at[0], at[1], at[2])).name == "minecraft:jukebox";
        if x * x + y * y + z * z >= 3.46 * 3.46 || !jukebox {
            m.extra.party_jukebox = None;
        }
    }
    if level_random.next_int_bound(400).unwrap_or(1) == 0 {
        imitate_nearby_mobs(m, w, level_random, events);
    }
}

/// `Parrot.imitateNearbyMobs`: half the time, a random imitable mob within 20
/// blocks is heard through the parrot's own mouth.
fn imitate_nearby_mobs(
    m: &Mob,
    w: &MobWorld,
    random: &mut LegacyRandom,
    events: &mut Vec<MobEvent>,
) {
    if !m.alive() || m.silent || random.next_int_bound(2).unwrap_or(1) != 0 {
        return;
    }
    let area = m.bounds().inflate(Vec3::new(20.0, 20.0, 20.0));
    let mobs: Vec<_> = w
        .others
        .iter()
        .filter(|s| s.alive && imitation(s.species).is_some() && s.bounds.intersects(area))
        .collect();
    if mobs.is_empty() {
        return;
    }
    let mob = mobs[random.next_int_bound(mobs.len() as i32).unwrap_or(0) as usize];
    if mob.silent {
        return;
    }
    if let Some(sound) = imitation(mob.species).filter(|s| !s.is_empty()) {
        let at = [m.position.x, m.position.y, m.position.z];
        events.push(MobEvent::Sound(sound, at, 0.7, pitch(random)));
    }
}

/// `Parrot.getAmbient`: now and then (outside peaceful) the voice of a random
/// imitable mob instead of the parrot's own.
pub fn ambient(m: &mut Mob, w: &MobWorld) -> &'static str {
    if w.difficulty.effective > 0.0 && m.random.next_int_bound(1000).unwrap_or(1) == 0 {
        let i = m
            .random
            .next_int_bound(IMITATIONS.len() as i32)
            .unwrap_or(0);
        return IMITATIONS[i as usize].1;
    }
    "entity.parrot.ambient"
}
