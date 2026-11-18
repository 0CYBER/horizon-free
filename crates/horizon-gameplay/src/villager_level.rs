//! What villager code calls on its level beyond [`MobLevel`]: the shared tables, the
//! few services of the game that stay behind, and the spawns and events a villager causes.
use crate::mob_level::MobLevel;
use horizon_client::{
    items::{ItemStack, Items},
    mobs::{MobType, MobWorld, Seen, spawning::BiomeFacts},
    stations::StationData,
};
use horizon_core::{geometry::Vec3, position::BlockPos};
use horizon_world::level::LevelEvent;
use std::{collections::HashSet, sync::Arc};

/// The services villager code calls on its level beyond its data.
pub trait VillagerLevel: MobLevel {
    /// The item table the world plays with, shared (`parts().items` needs the level borrowed mutably).
    fn items(&self) -> &Items;
    /// The station tables the world plays with, shared.
    fn stations(&self) -> &StationData;
    /// The `minecraft:villager_picks_up` item tag of the game's own resources, cloned.
    fn villager_picks_up(&self) -> Option<HashSet<String>>;
    /// The mob type `id` of the game's resources.
    fn mob_type(&self, id: &str) -> Option<Arc<MobType>>;
    /// The level as mob rules read it, with `others` as the mobs it has seen.
    fn mob_world_seen<'s>(&'s self, others: &'s [Seen]) -> MobWorld<'s>;
    /// Stores the last id handed out. `MobParts::next_entity` holds the same value, and the next
    /// entity takes that value plus one (`try_spawn_iron_golem` reads it, adds one and stores the result).
    fn set_next_entity(&mut self, id: u32);
    /// The mod hooks for the mob at `from` having joined the level (`natural`: a natural spawn).
    fn mob_joined(&mut self, from: usize, natural: bool);
    /// Biome facts at a position for `finalizeSpawn` and variant rules.
    fn biome_facts(&self, x: i32, y: i32, z: i32) -> BiomeFacts;
    /// A mob of `kind` created without `finalizeSpawn`'s rolls: its id, unless it did not join.
    fn spawn_mob_raw(&mut self, kind: &str, at: [f64; 3], persistent: bool) -> Option<u32>;
    /// `Level.broadcastEntityEvent` of an event that draws particles about mob `id`.
    fn mob_event_particles(&mut self, id: u32, event: i8);
    /// The `BonemealEvent` for villager bone meal from `stack` on `pos` and the growth. The first
    /// value is the effective success: the answer a mod gave in vanilla's place when it canceled
    /// the event, the crop's growth otherwise. The second is that answer (`Some(successful)`) or
    /// `None` when no mod canceled the event.
    fn bonemeal_grow(&mut self, pos: BlockPos, stack: &mut ItemStack) -> (bool, Option<bool>);
    /// Queues a level event and plays what it asks for.
    fn level_event(&mut self, event: LevelEvent);
    /// The fill level of the composter at `pos`, if there is one.
    fn composter_level(&self, pos: [i32; 3]) -> Option<i32>;
    /// `WorkAtComposter.compostItems` for the villager at `mob`.
    fn compost_items(&mut self, mob: usize, pos: [i32; 3]);
    /// The player behind the last hurt of the mob `id`, as a UUID.
    fn hurt_by_player_uuid(&self, id: u32) -> Option<[i32; 4]>;
    /// The UUID of a player by connection (None for the host).
    fn player_uuid_of(&self, who: Option<u32>) -> Option<[i32; 4]>;
    /// The connection of the guest acting now, if any.
    fn lan_acting(&self) -> Option<u32>;
    /// `LivingEntity.stopSleeping` for the villager `id`, outside its own tick.
    fn wake_villager(&mut self, id: u32);
    /// The index of the villager `id` in the level's mobs.
    fn villager_index(&self, id: u32) -> Option<usize>;
    /// The largest stack of `item` (64 for an unknown item).
    fn item_max_stack(&self, item: &str) -> u32;
    /// `WorkAtComposter.makeBread` for the villager at `i`.
    fn villager_make_bread(&mut self, i: usize);
    /// `BehaviorUtils.throwItem`: toss a stack from a villager's hand toward a point.
    fn villager_throw(&mut self, from: Vec3, eye: f64, to: Vec3, stack: ItemStack);
    /// `Villager.spawnGolemIfNeeded` for the villager `id`.
    fn villager_golem_check(&mut self, id: u32, needed: usize);
    /// `SpawnUtil.trySpawnMob(IRON_GOLEM, ...)` near `start`.
    fn try_spawn_iron_golem(&mut self, start: [i32; 3]) -> bool;
    /// `Villager.getPlayerReputation` of the player `uuid`.
    fn villager_reputation_of(&self, id: u32, uuid: [i32; 4]) -> i32;
}
