//! `WanderingTraderSpawner`'s clock and the `WanderingTraderData` saved with the world.
use crate::custom_spawner::CustomSpawnerLevel;
use horizon_client::{
    blocks::BlockFacts,
    mobs::{Species, leash::Holder, spawn_rules, trader},
    world::World,
};
use horizon_core::{geometry::Vec3, nbt::Tag, random::RandomSource};

const TICK_DELAY: i32 = 1200;
const SPAWN_DELAY: i32 = 24000;
const MIN_CHANCE: i32 = 25;
const MAX_CHANCE: i32 = 75;
const ATTEMPTS: i32 = 10;

/// What the spawner calls on its level beyond [`CustomSpawnerLevel`]: the game's own trader
/// methods, which mods target. Each runs the function of this module of the same name.
pub trait TraderLevel: CustomSpawnerLevel {
    /// `WanderingTraderSpawner.spawn` ([`spawn`]).
    fn spawn_trader(&mut self) -> bool;
    /// `tryToSpawnLlamaFor` ([`spawn_llama`]).
    fn spawn_trader_llama(&mut self, trader: u32, radius: i32);
    /// `findSpawnPositionNear` ([`find_spawn_near`]).
    fn find_trader_spawn_near(&mut self, reference: [i32; 3], radius: i32) -> Option<[i32; 3]>;
    /// `hasEnoughSpace` ([`has_space`]).
    fn trader_has_space(&self, pos: [i32; 3]) -> bool;
    /// The closest bell within `range` blocks ([`nearest_bell`]).
    fn nearest_bell(&self, from: [i32; 3], range: i32) -> Option<[i32; 3]>;
}

/// The spawner's clock and the `WanderingTraderData` saved with the world.
pub struct Spawner {
    tick_delay: i32,
    spawn_delay: i32,
    chance: i32,
    loaded: bool,
    dirty: bool,
}
impl Default for Spawner {
    fn default() -> Self {
        Self {
            tick_delay: TICK_DELAY,
            spawn_delay: SPAWN_DELAY,
            chance: MIN_CHANCE,
            loaded: false,
            dirty: false,
        }
    }
}
impl Spawner {
    pub fn is_loaded(&self) -> bool {
        self.loaded
    }
    /// Reads the file's `data` compound (`None` when there is no file).
    pub fn load(&mut self, data: Option<&Tag>) {
        let get = |k: &str, d: i32| match data.and_then(|d| d.get(k)) {
            Some(Tag::Int(v)) => *v,
            _ => d,
        };
        self.spawn_delay = get("spawn_delay", SPAWN_DELAY);
        self.chance = get("spawn_chance", MIN_CHANCE);
        self.loaded = true;
    }
    /// The file to write, `None` unless loaded and changed.
    pub fn to_save(&self) -> Option<Tag> {
        if !self.loaded || !self.dirty {
            return None;
        }
        Some(Tag::compound([
            (
                "data",
                Tag::compound([
                    ("spawn_delay", Tag::Int(self.spawn_delay)),
                    ("spawn_chance", Tag::Int(self.chance)),
                ]),
            ),
            ("DataVersion", Tag::Int(horizon_core::version::DATA_VERSION)),
        ]))
    }
    /// The file was written.
    pub fn saved(&mut self) {
        self.dirty = false;
    }
    /// One tick of the countdowns: the chance to roll against once the spawn delay runs out.
    pub fn due(&mut self) -> Option<i32> {
        self.tick_delay -= 1;
        if self.tick_delay > 0 {
            return None;
        }
        self.tick_delay = TICK_DELAY;
        self.spawn_delay -= TICK_DELAY;
        self.dirty = true;
        if self.spawn_delay > 0 {
            return None;
        }
        self.spawn_delay = SPAWN_DELAY;
        let chance = self.chance;
        self.chance = (chance + 25).clamp(MIN_CHANCE, MAX_CHANCE);
        Some(chance)
    }
    /// A trader appeared: the chance starts over.
    pub fn spawned(&mut self) {
        self.chance = MIN_CHANCE;
    }
    /// `WanderingTraderSpawner.tick` once its data is loaded: the clock,
    /// the chance, then the spawn.
    pub fn tick(&mut self, l: &mut impl TraderLevel) {
        let Some(chance) = self.due() else {
            return;
        };
        if l.parts().world.random.next_int_bound(100).unwrap_or(100) <= chance && l.spawn_trader() {
            self.spawned();
        }
    }
}

/// `WanderingTraderSpawner.spawn`.
pub fn spawn(l: &mut impl TraderLevel) -> bool {
    // ServerLevel.getRandomPlayer: one of the living players, drawn
    // from the level random.
    let alive: Vec<Vec3> = l
        .players()
        .into_iter()
        .filter(|p| p.alive)
        .map(|p| p.position)
        .collect();
    if alive.is_empty() {
        return true;
    }
    let random = &mut l.parts().world.random;
    let pick = random.next_int_bound(alive.len() as i32).unwrap_or(0);
    let p = alive[pick as usize];
    if random.next_int_bound(10).unwrap_or(1) != 0 {
        return false;
    }
    let player_pos = [p.x.floor() as i32, p.y.floor() as i32, p.z.floor() as i32];
    let reference = l.nearest_bell(player_pos, 48).unwrap_or(player_pos);
    let Some(pos) = l.find_trader_spawn_near(reference, 48) else {
        return false;
    };
    if !l.trader_has_space(pos) {
        return false;
    }
    if l.chunk_biome_has("minecraft:without_wandering_trader_spawns", pos) {
        return false;
    }
    let at = [
        f64::from(pos[0]) + 0.5,
        f64::from(pos[1]),
        f64::from(pos[2]) + 0.5,
    ];
    let Some(id) = l.spawn_mob(
        "minecraft:wandering_trader",
        at,
        true,
        spawn_rules::Reason::Event,
    ) else {
        return false;
    };
    for _ in 0..2 {
        l.spawn_trader_llama(id, 4);
    }
    if let Some(m) = l.parts().mobs.iter_mut().find(|m| m.id == id) {
        trader::start_visit(m, reference);
    }
    true
}

/// `tryToSpawnLlamaFor`: a trader llama near the trader, on its lead.
pub fn spawn_llama(l: &mut impl TraderLevel, trader: u32, radius: i32) {
    let Some(at) = l.parts().mobs.iter().find(|m| m.id == trader).map(|m| {
        [
            m.position.x.floor() as i32,
            m.position.y.floor() as i32,
            m.position.z.floor() as i32,
        ]
    }) else {
        return;
    };
    let Some(pos) = l.find_trader_spawn_near(at, radius) else {
        return;
    };
    let spot = [
        f64::from(pos[0]) + 0.5,
        f64::from(pos[1]),
        f64::from(pos[2]) + 0.5,
    ];
    if let Some(id) = l.spawn_mob(
        "minecraft:trader_llama",
        spot,
        false,
        spawn_rules::Reason::Event,
    ) && let Some(m) = l.parts().mobs.iter_mut().find(|m| m.id == id)
    {
        // EntitySpawnReason.EVENT: grown up.
        m.extra.age = 0;
        m.extra.leash = Some(Holder::Mob(trader));
    }
}

/// `findSpawnPositionNear`: ten tries at a column near the reference
/// where the trader could stand.
pub fn find_spawn_near(
    l: &mut impl CustomSpawnerLevel,
    reference: [i32; 3],
    radius: i32,
) -> Option<[i32; 3]> {
    for _ in 0..ATTEMPTS {
        let world = l.parts().world;
        let x = reference[0] + world.random.next_int_bound(radius * 2).unwrap_or(0) - radius;
        let z = reference[2] + world.random.next_int_bound(radius * 2).unwrap_or(0) - radius;
        if !world.loaded(x, z) {
            continue;
        }
        let y = l.motion_top(x, z);
        if l.is_spawn_position_ok(
            "minecraft:wandering_trader",
            Species::WanderingTrader,
            [x, y, z],
        ) {
            return Some([x, y, z]);
        }
    }
    None
}

/// `hasEnoughSpace`: no collision in the 2 by 3 by 2 box.
pub fn has_space(world: &World, facts: &BlockFacts, pos: [i32; 3]) -> bool {
    (0..=1).all(|dx| {
        (0..=2).all(|dy| {
            (0..=1).all(|dz| {
                !facts
                    .get(world.block(pos[0] + dx, pos[1] + dy, pos[2] + dz))
                    .full_collision
            })
        })
    })
}

/// The closest bell (`PoiTypes.MEETING`) within `range` blocks.
pub fn nearest_bell(
    world: &World,
    facts: &BlockFacts,
    from: [i32; 3],
    range: i32,
) -> Option<[i32; 3]> {
    let mut best: Option<([i32; 3], i32)> = None;
    for x in from[0] - range..=from[0] + range {
        for z in from[2] - range..=from[2] + range {
            if !world.loaded(x, z) {
                continue;
            }
            for y in (from[1] - range).max(world.bottom_y())..=from[1] + range {
                if &*facts.get(world.block(x, y, z)).name != "minecraft:bell" {
                    continue;
                }
                let d = (x - from[0]).pow(2) + (y - from[1]).pow(2) + (z - from[2]).pow(2);
                if best.is_none_or(|b| d < b.1) {
                    best = Some(([x, y, z], d));
                }
            }
        }
    }
    best.map(|b| b.0)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ticks_until_due(s: &mut Spawner, n: u32) -> Vec<(u32, i32)> {
        (1..=n).filter_map(|t| s.due().map(|c| (t, c))).collect()
    }

    #[test]
    fn the_chance_grows_by_25_each_delay_up_to_75_and_a_spawn_resets_it() {
        let mut s = Spawner::default();
        s.load(Some(&Tag::compound([
            ("spawn_delay", Tag::Int(1200)),
            ("spawn_chance", Tag::Int(75)),
        ])));
        assert_eq!(ticks_until_due(&mut s, 1200), vec![(1200, 75)]);
        assert_eq!(s.chance, 75);
        s.spawned();
        assert_eq!(s.chance, 25);
        // 24000 ticks to the next roll, 25 then 50.
        assert_eq!(ticks_until_due(&mut s, 24000), vec![(24000, 25)]);
        assert_eq!(s.chance, 50);
    }

    #[test]
    fn only_a_loaded_and_changed_spawner_saves() {
        let mut s = Spawner::default();
        assert!(s.to_save().is_none());
        s.load(None);
        assert!(s.to_save().is_none());
        ticks_until_due(&mut s, 1200);
        let saved = s.to_save().expect("changed");
        let data = saved.get("data").expect("data");
        assert_eq!(data.get("spawn_delay"), Some(&Tag::Int(22800)));
        assert_eq!(data.get("spawn_chance"), Some(&Tag::Int(25)));
        s.saved();
        assert!(s.to_save().is_none());
    }
}
