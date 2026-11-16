//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The handles' reads of the `raids` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait RaidsBackend {
    /// Side: server.
    /// `Raid.getCenter`.
    #[access(WorldOnly)]
    fn raid_center(&self, raid: RaidRef) -> Option<[i32; 3]>;
    /// Side: server.
    /// `Raid.getRaidOmenLevel`: 0 to 5 in play, any value a mod set.
    #[access(WorldOnly)]
    fn raid_omen_level(&self, raid: RaidRef) -> Option<i32>;
    /// Side: server.
    /// `Raid.getGroupsSpawned`: the waves spawned so far.
    #[access(WorldOnly)]
    fn raid_groups_spawned(&self, raid: RaidRef) -> Option<i32>;
    /// Side: server.
    /// `Raid.numGroups`, set from `getNumGroups(difficulty)` when the raid
    /// began: 0, 3, 5 or 7. A raid of omen level 2 or more adds a bonus wave
    /// (`hasBonusWave`).
    #[access(WorldOnly)]
    fn raid_num_groups(&self, raid: RaidRef) -> Option<i32>;
    /// Side: server.
    /// Whether the omen level is above 1, which adds a wave after the last
    /// (`Raid.hasBonusWave`).
    #[access(WorldOnly)]
    fn raid_has_bonus_wave(&self, raid: RaidRef) -> Option<bool>;
    /// Side: server.
    /// `Raid.isStarted`.
    #[access(WorldOnly)]
    fn raid_is_started(&self, raid: RaidRef) -> bool;
    /// Side: server.
    /// `Raid.isActive`: false once stopped.
    #[access(WorldOnly)]
    fn raid_is_active(&self, raid: RaidRef) -> bool;
    /// Side: server.
    /// `Raid.RaidStatus`.
    #[access(WorldOnly)]
    fn raid_status(&self, raid: RaidRef) -> Option<RaidStatus>;
    /// Side: server.
    /// `Raid.getTotalHealth`: the health of every raider that joined, which
    /// the boss bar divides by.
    #[access(WorldOnly)]
    fn raid_total_health(&self, raid: RaidRef) -> Option<f32>;
    /// Side: server.
    /// `Raid.getHealthOfLivingRaiders`.
    #[access(WorldOnly)]
    fn raid_living_health(&self, raid: RaidRef) -> Option<f32>;
    /// Side: server.
    /// The boss bar's progress, 0 to 1 (`Raid.raidEvent.getProgress`).
    #[access(WorldOnly)]
    fn raid_bar_progress(&self, raid: RaidRef) -> Option<f32>;
    /// Side: server.
    /// `Raid.getAllRaiders`: the living raiders that joined, in mob-list
    /// order, as entity handles.
    #[access(WorldOnly)]
    fn raid_raiders(&self, raid: RaidRef) -> Option<Vec<EntityRef>>;
    /// Side: server.
    /// The living raiders of one wave (`groupRaiderMap.get(wave)`).
    #[access(WorldOnly)]
    fn raid_wave_raiders(&self, raid: RaidRef, wave: i32) -> Option<Vec<EntityRef>>;
    /// Side: server.
    /// `Raid.heroesOfTheVillage`: the UUIDs of the players who got Hero of
    /// the Village, online or not, in the order vanilla's set iterates them.
    #[access(WorldOnly)]
    fn raid_heroes(&self, raid: RaidRef) -> Option<Vec<u128>>;
}
/// The handles' writes of the `raids` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait RaidsBackendMut {
    /// Side: server.
    /// `Raid.setRaidOmenLevel`: any value, as vanilla assigns it (only
    /// `absorbRaidOmen` clamps, to 0 to 5). The raid is saved with it.
    #[access(WorldOnly)]
    fn raid_set_omen_level(&mut self, raid: RaidRef, level: i32) -> bool;
    /// Side: server.
    /// `Raid.stop`: the raid ends as `stopped` and its boss bar leaves the
    /// players; the `raid_state_change` hook fires. False if the raid is
    /// already stopped.
    #[access(WorldOnly)]
    fn raid_stop(&mut self, raid: RaidRef) -> bool;
}
