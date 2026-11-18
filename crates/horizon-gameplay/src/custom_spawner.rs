//! What the custom spawners (`CustomSpawner`: cats, the wandering trader, the
//! village siege) call on `ServerLevel`. They run once a tick at most; their
//! code is compiled here over this trait, which the game implements once.
use horizon_client::{
    blocks::BlockFacts,
    join_seq::JoinList,
    mobs::{Mob, Species, raider, spawn_rules},
    world::World,
};
use horizon_core::geometry::Vec3;
use std::collections::HashSet;

/// The level's data as a custom spawner reads it: borrows only.
pub struct SpawnerParts<'a> {
    pub world: &'a mut World,
    pub mobs: &'a mut JoinList<Mob>,
    pub facts: &'a BlockFacts,
}

/// A player of the level (`ServerLevel.players()`).
pub struct SpawnerPlayer {
    pub uuid: u128,
    pub position: Vec3,
    pub alive: bool,
    pub spectator: bool,
}

/// The services the custom spawners call on their level beyond its data.
pub trait CustomSpawnerLevel {
    fn parts(&mut self) -> SpawnerParts<'_>;
    /// Whether this level is the overworld, the only one with custom spawners.
    fn overworld(&self) -> bool;
    /// `spawn_mobs` and `spawn_monsters` are both on.
    fn spawn_mobs_and_monsters(&self) -> bool;
    /// `ServerLevel.players()`: the local player and the guests in the level.
    fn players(&self) -> Vec<SpawnerPlayer>;
    /// The sections that are village centres (`PoiManager`'s distance tracker).
    fn village_centers(&self) -> HashSet<(i32, i32, i32)>;
    /// `Level.getSkyDarken`.
    fn sky_darken(&self) -> i32;
    /// `Heightmap.Types.WORLD_SURFACE` at a column.
    fn surface_height(&self, x: i32, z: i32) -> i32;
    /// `Heightmap.Types.MOTION_BLOCKING_NO_LEAVES` at a column.
    fn motion_top(&self, x: i32, z: i32) -> i32;
    /// `Biome.is(tag)` at a block, through the spawn rules' biome zoom.
    fn biome_tag(&self, tag: &str, pos: [i32; 3]) -> bool;
    /// The chunk's own biome at a block is in the mob data's biome tag; false unloaded.
    fn chunk_biome_has(&self, tag: &str, pos: [i32; 3]) -> bool;
    /// The structures a structure tag lists.
    fn structure_tag(&self, tag: &str) -> Vec<String>;
    /// The structures whose pieces hold `pos`.
    fn structures_at(&self, pos: [i32; 3]) -> Vec<String>;
    /// `SpawnPlacements.isSpawnPositionOk`.
    fn is_spawn_position_ok(&self, entity: &str, species: Species, pos: [i32; 3]) -> bool;
    /// `SpawnPlacements.checkSpawnRules`, drawing from the level random.
    fn check_spawn_rules(
        &mut self,
        entity: &str,
        species: Species,
        reason: spawn_rules::Reason,
        pos: [i32; 3],
    ) -> bool;
    /// `EntityType.spawn`: the new mob finalized with `reason`; its id.
    fn spawn_mob(
        &mut self,
        kind: &str,
        at: [f64; 3],
        persistent: bool,
        reason: spawn_rules::Reason,
    ) -> Option<u32>;
    /// [`spawn_mob`](Self::spawn_mob) for a creation that snaps the mob to a
    /// yaw (`snapTo(pos, yaw, 0)`): the facing takes no draw from the level.
    fn spawn_mob_facing(
        &mut self,
        kind: &str,
        at: [f64; 3],
        persistent: bool,
        reason: spawn_rules::Reason,
        yaw: f32,
    ) -> Option<u32>;
    /// `EntityType.create` and `finalizeSpawn` for a patrol or an event.
    fn spawn_event_mob(
        &mut self,
        kind: &str,
        at: [f64; 3],
        leader: bool,
        reason: raider::Reason,
    ) -> Option<u32>;
    /// Mods hear of a village siege's spawn position; true when one cancelled it.
    fn village_siege_cancelled(&mut self, player: u128, attempted: [f64; 3]) -> bool;
}

/// `SectionPos` of a block position.
pub fn section(p: [i32; 3]) -> (i32, i32, i32) {
    (p[0] >> 4, p[1] >> 4, p[2] >> 4)
}

/// `PoiManager.DistanceTracker`: sections from the nearest village centre,
/// 7 when farther than 6.
pub fn sections_to_village(centers: &HashSet<(i32, i32, i32)>, at: (i32, i32, i32)) -> i32 {
    centers
        .iter()
        .map(|c| {
            (c.0 - at.0)
                .abs()
                .max((c.1 - at.1).abs())
                .max((c.2 - at.2).abs())
        })
        .min()
        .map_or(7, |d| d.min(7))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sections_to_village_is_a_chebyshev_distance_capped_at_seven() {
        let mut centers = HashSet::new();
        assert_eq!(sections_to_village(&centers, (0, 0, 0)), 7);
        centers.insert(section([40, 64, 40]));
        assert_eq!(sections_to_village(&centers, (2, 4, 2)), 0);
        assert_eq!(sections_to_village(&centers, (4, 4, 2)), 2);
        assert_eq!(sections_to_village(&centers, (-10, 4, 2)), 7);
        assert_eq!(section([-1, -17, 15]), (-1, -2, 0));
    }
}
