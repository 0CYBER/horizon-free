//! A server's levels, one per dimension (`MinecraftServer.levels`).
//!
//! The game's tick code reads one level: the one in place, held in `Game`'s
//! own fields (the world, its streaming and save, block ticks, entities).
//! A server (the dedicated server and single player's server thread) keeps
//! the other dimensions' levels here and ticks each one with players or forced
//! chunks by swapping it into place (`enter_level`), the way `as_guest` swaps a player
//! in, so the tick code runs unchanged, one level at a time.
//!
//! Shared by every level, as vanilla's derived levels share the
//! Overworld's level data: the game time, the day time and the weather, the
//! game rules, the scoreboard, the maps and the entities waiting to arrive
//! through a portal.

use super::commands::Target;
use super::entity_residency::{define_resident_access, resident_owners};
use super::{
    Dim, ExperienceOrb, FallingBlock, Game, ItemEntity, Mob, boats, commands,
    creaking::CreakingState, dragon, fireworks, lightning, machines, minecarts,
    ominous_item_spawner, persist, raids::Raiding, sculk::SculkState, tnt, trident,
};
use horizon_client::join_seq::{
    Joined,
    residency::{
        Journal, Kind, Owner,
        table::{EntityLookup, Location, SourceEntity},
    },
};
use horizon_client::spans::time as span;
use horizon_client::{
    Result,
    block_entity_anim::BlockEntityAnims,
    random_tick_sections::RandomTickSections,
    save::SaveDir,
    streaming::Streaming,
    tick_chunk_membership::TickChunkMembership,
    ticket_levels::{ChunkTickets, PortalTicket},
    vanillagen::VanillaTerrain,
    world::World,
};
use horizon_world::level::{LevelEvent, Ticks};
use mod_api::DimensionKey;
use std::{
    collections::HashMap,
    sync::Arc,
    time::{Duration, Instant},
};

/// A borrowed counting view: no resident-table scans and no allocation for gauges.
pub(super) struct EntityCountView<'a> {
    pub(super) fixed: [(&'static str, usize); 19],
    pub(super) mobs: &'a [Mob],
    pub(super) projectiles: &'a [horizon_client::projectiles::Projectile],
    pub(super) general: &'a super::general::GeneralRows,
    pub(super) others: &'a [commands::OtherEntity],
}
impl EntityCountView<'_> {
    pub(super) fn total(&self) -> usize {
        self.fixed.iter().map(|(_, n)| n).sum::<usize>()
            + self.projectiles.len()
            + self.mobs.len()
            + self.general.len()
            + self.others.len()
    }
    pub(super) fn registry_counts(&self) -> Vec<(String, u32)> {
        let mut counts = std::collections::BTreeMap::<String, u32>::new();
        for &(id, n) in &self.fixed {
            if n > 0 {
                *counts.entry(id.to_owned()).or_default() += n as u32;
            }
        }
        for projectile in self.projectiles {
            *counts.entry(projectile.kind.id().to_owned()).or_default() += 1;
        }
        for mob in self.mobs.iter().filter(|mob| !mob.removed) {
            *counts.entry(mob.kind.id.clone()).or_default() += 1;
        }
        for row in self.general.iter() {
            *counts
                .entry(
                    super::general::general_type_catalog()
                        .key(row.type_slot)
                        .to_owned(),
                )
                .or_default() += 1;
        }
        for row in self.others {
            *counts.entry(row.kind.clone()).or_default() += 1;
        }
        counts.into_iter().collect()
    }
}

// Both Game and Parked own these typed families. Keep their instrumentation in one list.
macro_rules! entity_count_view {
    ($level:expr, $projectiles:expr, $general:expr, $knots:expr) => {
        crate::game::levels::EntityCountView {
            fixed: [
                ("minecraft:item", $level.dropped.len()),
                ("minecraft:experience_orb", $level.orbs.len()),
                ("minecraft:minecart", $level.carts.len()),
                ("minecraft:boat", $level.boats.len()),
                ("minecraft:falling_block", $level.falling.len()),
                ("minecraft:painting", $level.paintings.len()),
                ("minecraft:item_frame", $level.item_frames.len()),
                ("minecraft:armor_stand", $level.armor_stands.len()),
                ("minecraft:trident", $level.tridents.len()),
                ("minecraft:firework_rocket", $level.rockets.len()),
                ("minecraft:tnt", $level.tnts.len()),
                ("minecraft:end_crystal", $level.crystals.len()),
                ("minecraft:leash_knot", $knots.len()),
                (
                    "minecraft:ominous_item_spawner",
                    $level.ominous_spawners.len(),
                ),
                ("minecraft:cushion", $level.cushions.len()),
                ("minecraft:area_effect_cloud", $level.breath_clouds.len()),
                ("minecraft:lightning_bolt", $level.bolts.len()),
                ("minecraft:evoker_fangs", $level.fangs.perf_count()),
                // Multipart hitboxes are not independently admitted world entities.
                (
                    "minecraft:ender_dragon",
                    usize::from($level.dragon.as_ref().is_some()),
                ),
            ],
            mobs: &$level.mobs,
            projectiles: $projectiles,
            general: $general,
            others: &$level.others,
        }
    };
}
pub(super) use entity_count_view;

/// Server ticks a level other than the Overworld stays loaded with no player
/// in it, before it is saved and closed (vanilla unloads a level's chunks
/// once no ticket holds them; the Overworld stays loaded).
const IDLE_TICKS: u32 = 300;

/// A level that is not in place: every per-level field of `Game`.
pub(super) struct Parked {
    pub(super) world: World,
    streaming: Streaming,
    locator: Option<Arc<VanillaTerrain>>,
    save: Arc<SaveDir>,
    block_ticks: Ticks,
    tick_chunks: TickChunkMembership,
    tickets: ChunkTickets,
    random_tick_sections: RandomTickSections,
    level_events: Vec<LevelEvent>,
    /// Entity portal processes and cooldowns belong to this level's entities.
    pub(super) portal_states: HashMap<u32, horizon_client::nether_portal::PortalState>,
    /// Which of this level's chunks may hold a portal (`Game::find_portals`), kept per level:
    /// one cache asked by two levels in turn read every palette of both again each tick.
    pub(super) portal_sections: horizon_client::portal_sections::PortalSections,
    falling: horizon_client::join_seq::JoinList<FallingBlock, false>,
    paintings: horizon_client::join_seq::JoinList<horizon_client::entities::Painting>,
    item_frames: horizon_client::join_seq::JoinList<horizon_client::entities::ItemFrame>,
    armor_stands: horizon_client::join_seq::JoinList<horizon_client::entities::ArmorStand>,
    cushions: horizon_client::join_seq::JoinList<horizon_client::entities::Cushion>,
    others: horizon_client::join_seq::ResidentList<commands::OtherEntity>,
    fangs: super::fangs::FangState,
    pub(super) boats: horizon_client::join_seq::JoinList<boats::Boat, false>,
    pub(super) carts: horizon_client::join_seq::JoinList<minecarts::Minecart, false>,
    jukebox_sounds: HashMap<(i32, i32, i32), u64>,
    bolts: Vec<lightning::Bolt>,
    crystals: horizon_client::join_seq::JoinList<dragon::Crystal>,
    dragon: super::native_entity_refs::dragon_parts_table::DragonSlot,
    breath_clouds: horizon_client::join_seq::ResidentList<dragon::Breath>,
    tridents: horizon_client::join_seq::JoinList<trident::ThrownTrident>,
    rockets: horizon_client::join_seq::JoinList<fireworks::Rocket>,
    ominous_spawners:
        horizon_client::join_seq::ResidentList<ominous_item_spawner::OminousItemSpawner>,
    tnts: horizon_client::join_seq::JoinList<tnt::PrimedTnt>,
    /// Its orbs', TNT's and falling blocks' place in the tick order beyond
    /// their joins (`ordered_tick::TickRestarts`).
    tick_restarts: super::ordered_tick::TickRestarts,
    general: super::general::GeneralStore,
    hoppers: HashMap<(i32, i32, i32), machines::HopperState>,
    pub(super) mobs: horizon_client::join_seq::JoinList<Mob>,
    /// The entity ids of the level's fence knots (`knot_ids`).
    knot_ids: HashMap<[i32; 3], u32>,
    knots: horizon_client::join_seq::JoinList<super::leads::Knot>,
    projectiles: horizon_client::join_seq::JoinList<horizon_client::projectiles::Projectile, false>,
    dropped: horizon_client::join_seq::JoinList<ItemEntity>,
    orbs: horizon_client::join_seq::JoinList<ExperienceOrb>,
    persist: persist::EntityMemory,
    lectern_pulses: Vec<((i32, i32, i32), i32)>,
    lid_openers: HashMap<(i32, i32, i32), i32>,
    lan_events: Vec<super::QueuedLevelEvent>,
    lan_block_events: Vec<([i32; 3], i32, i32, u32)>,
    lan_entity_events: Vec<(i32, i8)>,
    lan_critical_hits: Vec<(Option<u32>, i32, bool)>,
    /// The hub's tracked entities of this level (`LanHost::states`), as its last tick described
    /// them: what its players' moves are checked against before this tick describes them again.
    lan_states: horizon_client::lan::entity::States,
    /// Only the per-level parts of these are used (`swap_level`).
    anims: BlockEntityAnims,
    pub(super) raiding: Raiding,
    sculk: SculkState,
    creaking: CreakingState,
    /// Natural spawning's candidate cache: each level keeps its own players' squares.
    spawn_scratch: super::natural_spawning::SpawnScratch,
    /// Server ticks since a player was last in the level.
    idle: u32,
    /// Its save is queued, and its `SaveDir` is held until the level leaves memory: dropping
    /// that folder joins the writer thread (`close_idle_levels`).
    closing: bool,
    /// The level's own world border (`ServerLevel.getWorldBorder`), and what its players were
    /// last told of it (`LanHost::border_sent`).
    border: horizon_client::worldborder::WorldBorder,
    border_sent: Option<horizon_client::lan::border::Sent>,
}

impl Parked {
    resident_owners!(
        define_resident_access,
        self,
        projectiles,
        knots,
        general.rows
    );

    /// A write reached this level: it is not on its way out, and the idle count starts again
    /// (`close_idle_levels`). What the write left must not be dropped with a close already
    /// queued, and a level a mod keeps writing to is not closed every tick.
    fn not_closing(&mut self) {
        self.closing = false;
        self.idle = 0;
    }
}

impl Game<'_> {
    /// Resolve through the exact owner, never by trying dimension swaps.
    pub(super) fn locate_server_resident_owner(
        &self,
        at: Location,
    ) -> Option<SourceEntity<(Target, bool)>> {
        if matches!(
            at.owner.kind,
            horizon_client::join_seq::residency::Kind::Player
                | horizon_client::join_seq::residency::Kind::Hook
        ) {
            return self.locate_specialized_resident_owner(at);
        }
        if at.owner.dimension == self.dim.key() {
            // An open transaction's removal reads as gone already.
            return self.locate_resident_owner(at).map(|mut source| {
                source.value.1 &= !self.removal_pending(&source.value.0);
                source
            });
        }
        let dim = match at.owner.dimension {
            DimensionKey::OVERWORLD => Dim::Overworld,
            DimensionKey::NETHER => Dim::Nether,
            DimensionKey::END => Dim::End,
            key => Dim::Extra(key),
        };
        // The transactions stay with the game when a level is parked: its pending removals too.
        self.levels
            .parked
            .get(&dim)?
            .locate_resident_owner(at)
            .map(|mut source| {
                source.value.1 &= !self.removal_pending_in(dim, &source.value.0);
                source
            })
    }

    /// Whether `target` counts as removed in the level in place: a mod removed it, or an open
    /// transaction did and its root commit has not run the removal yet.
    pub(in crate::game) fn target_removed(&self, target: &Target) -> bool {
        self.persist.removals.contains(target)
            || self.persist.destroying_vehicles.contains(target)
            || self.removal_pending(target)
    }

    pub(in crate::game) fn resident_source_target_visible(
        &self,
        dimension: DimensionKey,
        target: Target,
    ) -> bool {
        let player = match target {
            Target::Guest(conn) => Some(conn),
            Target::Player => self.lan_acting,
            _ => None,
        };
        if player
            .and_then(|conn| self.command_guest(conn))
            .is_some_and(|guest| !guest.body_present())
        {
            return false;
        }
        if dimension == self.dim.key() {
            return !self.target_removed(&target);
        }
        let dim = match dimension {
            DimensionKey::OVERWORLD => Dim::Overworld,
            DimensionKey::NETHER => Dim::Nether,
            DimensionKey::END => Dim::End,
            key => Dim::Extra(key),
        };
        self.levels.parked.get(&dim).is_none_or(|parked| {
            !parked.persist.removals.contains(&target)
                && !parked.persist.destroying_vehicles.contains(&target)
                && !self.removal_pending_in(dim, &target)
        })
    }

    /// Visit exactly the actual physical lifetime slot. Resolution releases the
    /// table and its journal before lending storage; no lift, swap or replacement
    /// occurs, including when the visitor unwinds. Logical visibility is a caller
    /// policy, just as for the underlying owner resolver. The visitor must not
    /// call entity lookup or recursively enter this seam.
    pub(super) fn with_resident_slot<R>(
        &mut self,
        dimension: &mod_api::DimensionKey,
        id: std::num::NonZeroU32,
        visit: impl FnOnce(&mut horizon_mod_runtime::attachments::AttachmentLifetime) -> R,
    ) -> super::entity_residency::SlotAccess<R> {
        use super::entity_residency::SlotAccess;
        let resolved = {
            self.resident_entities
                .table
                .borrow_mut()
                .entity_lookup(dimension, id.get(), |at| {
                    self.locate_server_resident_owner(at)
                })
        };
        let location = match resolved {
            EntityLookup::Found { location, .. } => location,
            EntityLookup::Taken => return SlotAccess::Taken,
            EntityLookup::Missing => return SlotAccess::Missing,
        };
        debug_assert!(self.resident_entities.table.try_borrow_mut().is_ok());
        if matches!(
            location.owner.kind,
            horizon_client::join_seq::residency::Kind::Player
                | horizon_client::join_seq::residency::Kind::Hook
        ) {
            if *dimension != self.dim.key() {
                let dim = match *dimension {
                    DimensionKey::OVERWORLD => Dim::Overworld,
                    DimensionKey::NETHER => Dim::Nether,
                    DimensionKey::END => Dim::End,
                    key => Dim::Extra(key),
                };
                if !self.levels.parked.contains_key(&dim) {
                    return SlotAccess::Missing;
                }
            }
            return self.with_specialized_resident_slot(location, visit);
        }
        if *dimension == self.dim.key() {
            self.with_resident_owner_slot(location, visit)
        } else {
            let dim = match *dimension {
                mod_api::DimensionKey::OVERWORLD => Dim::Overworld,
                mod_api::DimensionKey::NETHER => Dim::Nether,
                mod_api::DimensionKey::END => Dim::End,
                key => Dim::Extra(key),
            };
            self.levels
                .parked
                .get_mut(&dim)
                .map_or(SlotAccess::Missing, |parked| {
                    parked.with_resident_owner_slot(location, visit)
                })
        }
    }

    pub(in crate::game) fn server_resident_entity(&self, id: u32) -> EntityLookup<Target> {
        let found = self
            .resident_entities
            .table
            .borrow_mut()
            .server_entity_lookup(id, |at| self.locate_server_resident_owner(at));
        match found {
            EntityLookup::Found {
                location,
                value: (value, true),
            } => EntityLookup::Found { location, value },
            EntityLookup::Found {
                value: (_, false), ..
            }
            | EntityLookup::Missing => EntityLookup::Missing,
            EntityLookup::Taken => EntityLookup::Taken,
        }
    }
    /// Find the canonical level without swapping or probing physical families.
    pub(super) fn resident_target_dimension(&self, target: Target) -> Option<Dim> {
        let id = self.resident_target_id(target)?;
        let found = match self.server_resident_entity(id) {
            EntityLookup::Found { location, value }
                if value == target
                    || matches!(
                        (value, target),
                        (Target::Player, Target::Guest(_)) | (Target::Guest(_), Target::Player)
                    ) =>
            {
                location
            }
            _ => return None,
        };
        Some(match found.owner.dimension {
            DimensionKey::OVERWORLD => Dim::Overworld,
            DimensionKey::NETHER => Dim::Nether,
            DimensionKey::END => Dim::End,
            key => Dim::Extra(key),
        })
    }

    /// A new epoch invalidates every old producer before rebinding retained levels.
    pub(super) fn reset_entity_residency(&mut self) {
        let table = self.resident_entities.table.get_mut();
        table.reset();
        let journal = table.producer();
        self.bind_resident_owners(&journal, self.dim.key(), self.is_logical_server());
        self.bind_specialized_resident_owners(&journal);
        for (dim, parked) in &mut self.levels.parked {
            parked.bind_resident_owners(&journal, dim.key(), true);
        }
    }
}

/// Which passes of the server tick the level in place runs: the first
/// reads the network (`Hub::poll`), the last ends the tick for every player
/// (`Hub::end_tick`). A tick with one level is both. `running` is the
/// tick rate manager's answer for this server tick, asked once before the
/// levels (`MinecraftServer.tickServer`); `None` outside `tick_server`.
#[derive(Clone, Copy)]
pub(super) struct Pass {
    pub first: bool,
    pub last: bool,
    pub running: Option<bool>,
    /// Set once this level's player packets were handled before block ticks.
    pub packets_drained: bool,
}

impl Default for Pass {
    fn default() -> Self {
        Self {
            first: true,
            last: true,
            running: None,
            packets_drained: false,
        }
    }
}

/// How long a server tick took and what it spent the time in.
pub(super) struct TickTiming {
    pub wall_ms: f32,
    pub cpu_ms: Option<f32>,
    /// Spans recorded inside the tick only, in observation order.
    pub raw: Vec<horizon_client::spans::Span>,
}

/// The levels not in place, by dimension.
#[derive(Default)]
pub(super) struct Levels {
    pub(super) parked: HashMap<Dim, Parked>,
    /// The levels whose open the mods heard (`native_level_load`), each to
    /// be told closed once.
    heard_open: Vec<Dim>,
    /// `max-chained-neighbor-updates` of a dedicated server, for the levels it opens;
    /// `None` is the default every other server runs with.
    max_chained_neighbor_updates: Option<i32>,
    /// The levels a mod asked to have saved (`level_request_save`), each once, saved at the start
    /// of the next server tick.
    requested_saves: Vec<Dim>,
    /// The levels mods opened while the server runs, and the requests waiting to be applied.
    pub(super) runtime: super::runtime_levels::RuntimeTable,
}

impl Levels {
    /// Sets the neighbour update chain limit of the levels made from now on.
    pub(super) fn set_max_chained_neighbor_updates(&mut self, max: i32) {
        self.max_chained_neighbor_updates = Some(max);
    }

    /// The block ticks of a new level, with the server's neighbour update chain limit.
    pub(super) fn new_ticks(&self, random_tick_seed: i32) -> Ticks {
        let mut ticks = Ticks::new(random_tick_seed);
        if let Some(max) = self.max_chained_neighbor_updates {
            ticks.set_max_chained_neighbor_updates(max);
        }
        ticks
    }

    /// Queue `dim` to be saved at the start of the next server tick; a level already queued is
    /// not queued twice.
    pub(in crate::game) fn request_save(&mut self, dim: Dim) {
        if !self.requested_saves.contains(&dim) {
            self.requested_saves.push(dim);
        }
    }

    /// Whether a save of `dim` is still waiting for a tick that admits
    /// saves (the arismoko/horizon#2488 gate holds it back).
    #[cfg_attr(not(test), allow(dead_code))]
    pub(in crate::game) fn save_pending(&self, dim: Dim) -> bool {
        self.requested_saves.contains(&dim)
    }

    /// Drop a save of `dim` still waiting for a tick: its level is gone.
    pub(super) fn cancel_requested_save(&mut self, dim: Dim) {
        self.requested_saves.retain(|saved| *saved != dim);
    }

    /// Clear every parked level and dimensions recorded as heard open.
    pub(super) fn clear(&mut self) {
        self.parked.clear();
        self.heard_open.clear();
        self.requested_saves.clear();
        self.runtime.clear();
    }

    /// A parked level, without opening or loading it.
    pub(super) fn parked_world(&self, dim: Dim) -> Option<&World> {
        self.parked.get(&dim).map(|level| &level.world)
    }

    /// The world of the level not in place whose dimension is `dimension`
    /// (`minecraft:the_nether`), found without collecting the parked levels.
    pub(super) fn parked_world_of(&self, dimension: &str) -> Option<&World> {
        self.parked
            .iter()
            .find(|(dim, _)| dim.id() == dimension)
            .map(|(_, level)| &level.world)
    }

    /// The world of the level not in place whose dimension is `dimension`, to
    /// change. A change reaching a level whose close is queued cancels that close: otherwise it
    /// would be dropped with the level (`close_idle_levels`).
    pub(super) fn parked_world_mut_of(&mut self, dimension: &str) -> Option<&mut World> {
        self.parked
            .iter_mut()
            .find(|(dim, _)| dim.id() == dimension)
            .map(|(_, level)| {
                level.not_closing();
                &mut level.world
            })
    }

    /// The streaming of the level not in place whose dimension is `dimension`.
    pub(super) fn parked_streaming_of(&self, dimension: &str) -> Option<&Streaming> {
        self.parked
            .iter()
            .find(|(dim, _)| dim.id() == dimension)
            .map(|(_, level)| &level.streaming)
    }

    /// A parked level's existing generator query worker, without switching levels.
    pub(super) fn parked_locator_of(&self, dimension: &str) -> Option<Arc<VanillaTerrain>> {
        self.parked_of(dimension)?.locator.clone()
    }

    /// The streaming of the level not in place whose dimension is `dimension`, to change: a
    /// ticket it holds cancels a queued close, as a world change does (`close_idle_levels`).
    pub(super) fn parked_streaming_of_mut(&mut self, dimension: &str) -> Option<&mut Streaming> {
        self.parked
            .iter_mut()
            .find(|(dim, _)| dim.id() == dimension)
            .map(|(_, level)| {
                level.not_closing();
                &mut level.streaming
            })
    }

    /// `ServerLevel.setChunkForced`: force a column in an already open level without changing the acting
    /// player's level; the same queue and load barrier serve the active level.
    pub(super) fn force_parked_chunk(
        &mut self,
        dimension: &str,
        x: i32,
        z: i32,
        forced: bool,
    ) -> Option<bool> {
        let level = self.parked_mut_of(dimension)?;
        Some(
            level
                .streaming
                .set_forced_loaded(&mut level.world, x, z, forced),
        )
    }

    pub(super) fn parked_anims(&self, dim: Dim) -> Option<&BlockEntityAnims> {
        self.parked.get(&dim).map(|level| &level.anims)
    }

    /// The worlds of the levels not in place.
    /// A parked level's world border.
    pub(super) fn parked_border(
        &self,
        dim: Dim,
    ) -> Option<&horizon_client::worldborder::WorldBorder> {
        self.parked.get(&dim).map(|level| &level.border)
    }

    pub(super) fn install_profession_save_snapshots(
        &self,
        table: Arc<horizon_client::mobs::villager::ResolvedProfessions>,
    ) {
        for level in self.parked.values() {
            level.save.install_profession_snapshot(table.clone());
        }
    }

    pub(super) fn install_mob_map_snapshots(
        &self,
        snapshot: &Arc<horizon_client::neoforge_mob_maps::NeoForgeMobMaps>,
    ) {
        for level in self.parked.values() {
            if let Some(terrain) = &level.locator {
                terrain.set_mob_maps(snapshot);
            }
        }
    }

    pub(super) fn parked_worlds(&self) -> impl Iterator<Item = &World> {
        self.parked.values().map(|level| &level.world)
    }

    /// The streamers of the levels not in place.
    pub(super) fn parked_streamings(
        &self,
    ) -> impl Iterator<Item = &horizon_client::streaming::Streaming> {
        self.parked.values().map(|level| &level.streaming)
    }

    /// The level not in place, in `dim`; a change through it cancels a queued close
    /// (`close_idle_levels`).
    pub(super) fn parked_mut(&mut self, dim: Dim) -> Option<&mut Parked> {
        let level = self.parked.get_mut(&dim)?;
        level.not_closing();
        Some(level)
    }

    /// A parked level by its exact dimension identity, without swapping it in.
    pub(super) fn parked_of(&self, dimension: &str) -> Option<&Parked> {
        self.parked
            .iter()
            .find(|(dim, _)| dim.id() == dimension)
            .map(|(_, level)| level)
    }
    pub(super) fn parked_mut_of(&mut self, dimension: &str) -> Option<&mut Parked> {
        self.parked
            .iter_mut()
            .find(|(dim, _)| dim.id() == dimension)
            .map(|(_, level)| {
                level.not_closing();
                level
            })
    }

    /// Whether a parked level's general rows hold any of these UUIDs (arismoko/horizon#792's
    /// dimension-transfer check: a destination may not already hold a row
    /// of the travelling tree's UUIDs).
    pub(in crate::game) fn parked_general_holds_any_uuid(
        &self,
        dim: Dim,
        uuids: &std::collections::HashSet<[i32; 4]>,
    ) -> bool {
        self.parked.get(&dim).is_some_and(|parked| {
            parked
                .general
                .rows
                .iter()
                .any(|row| uuids.contains(&row.uuid))
        })
    }

    /// The entities of a parked level, counted as `Game::entity_count` counts the one in place.
    pub(super) fn parked_entity_count(&self, dim: Dim) -> Option<usize> {
        let l = self.parked.get(&dim)?;
        Some(
            l.mobs.iter().filter(|m| !m.removed).count()
                + l.dropped.len()
                + l.orbs.len()
                + l.projectiles.len()
                + l.carts.len()
                + l.boats.len()
                + l.falling.len()
                + l.paintings.len()
                + l.item_frames.len()
                + l.armor_stands.len()
                + l.cushions.len()
                + l.tridents.len()
                + l.rockets.len()
                + l.tnts.len(),
        )
    }

    /// The mobs of a parked level.
    pub(super) fn parked_mobs(&self, dim: Dim) -> Option<&[Mob]> {
        self.parked.get(&dim).map(|level| &*level.mobs)
    }

    pub(super) fn parked_perf_views(
        &self,
    ) -> impl Iterator<Item = (Dim, &World, EntityCountView<'_>)> {
        self.parked.iter().map(|(&dim, level)| {
            (
                dim,
                &level.world,
                entity_count_view!(level, &level.projectiles, &level.general.rows, &level.knots),
            )
        })
    }

    /// A parked level's loose entities, for `horizon-report`: items, experience orbs,
    /// projectiles, minecarts and boats.
    pub(super) fn parked_entity_counts(&self, dim: Dim) -> Option<[usize; 5]> {
        self.parked.get(&dim).map(|l| {
            [
                l.dropped.len(),
                l.orbs.len(),
                l.projectiles.len(),
                l.carts.len(),
                l.boats.len(),
            ]
        })
    }

    /// The levels that are not in place, by dimension.
    /// The forced chunks of every level not in place, released.
    pub(super) fn release_parked_forced_chunks(&mut self) {
        for level in self.parked.values_mut() {
            level.streaming.clear_forced();
        }
    }

    pub(super) fn parked_dims(&self) -> Vec<Dim> {
        self.parked.keys().copied().collect()
    }
}

impl Game<'_> {
    /// A new level for `dim`, opened from the save (`MinecraftServer.createLevels`).
    pub(super) fn open_level(&self, dim: Dim) -> Result<Parked> {
        // A level a mod opened has the seed and generator it asked for.
        let (generate, seed) = match self.runtime_terrain(dim) {
            Some((generate, seed)) => (Some(generate), seed),
            None => (None, self.level.seed),
        };
        let (streaming, save, locator) = super::world_loading::make_streaming(
            &self.res,
            &self.pack,
            &self.options,
            &self.settings,
            &self.world_root,
            dim,
            self.level_shapes.get(dim.id()),
            seed,
            &self.level.enabled_features,
            self.world_biomes
                .as_ref()
                .unwrap_or_else(|| self.res.biomes())
                .clone(),
            generate.as_ref(),
        )?;
        save.install_profession_snapshot(self.stations().professions.clone());
        if let Some(terrain) = &locator {
            terrain.set_mob_maps(self.mob_map_snapshot());
        }
        let mut world = Game::world_for_mods(self.res.facts().air, seed);
        world.attachments = save.load_attachments();
        world.shape = self.level_shapes.get(dim.id());
        world.fast_lava = self.dimension_type_of(dim).fast_lava;
        world.infiniburn = Some(self.resolve_level_infiniburn(self.dimension_type_of(dim)));
        let tickets = ChunkTickets::with_saved_portal(save.load_portal_tickets());
        Ok(Parked {
            world,
            streaming,
            locator,
            save,
            block_ticks: self.levels.new_ticks(seed as i32),
            tick_chunks: Default::default(),
            tickets,
            random_tick_sections: Default::default(),
            level_events: Vec::new(),
            portal_states: HashMap::new(),
            portal_sections: Default::default(),
            falling: Default::default(),
            paintings: Default::default(),
            item_frames: Default::default(),
            armor_stands: Default::default(),
            cushions: Default::default(),
            others: Default::default(),
            fangs: Default::default(),
            boats: Default::default(),
            carts: Default::default(),
            jukebox_sounds: HashMap::new(),
            bolts: Vec::new(),
            crystals: Default::default(),
            dragon: Default::default(),
            breath_clouds: Default::default(),
            tridents: Default::default(),
            rockets: Default::default(),
            ominous_spawners: Default::default(),
            tnts: Default::default(),
            tick_restarts: Default::default(),
            general: Default::default(),
            hoppers: HashMap::new(),
            mobs: Default::default(),
            knot_ids: HashMap::new(),
            knots: Default::default(),
            projectiles: Default::default(),
            dropped: Default::default(),
            orbs: Default::default(),
            persist: Default::default(),
            lectern_pulses: Vec::new(),
            lid_openers: HashMap::new(),
            lan_events: Vec::new(),
            lan_block_events: Vec::new(),
            lan_entity_events: Vec::new(),
            lan_critical_hits: Vec::new(),
            lan_states: Default::default(),
            anims: Default::default(),
            raiding: Default::default(),
            sculk: Default::default(),
            creaking: Default::default(),
            spawn_scratch: Default::default(),
            idle: 0,
            closing: false,
            border: self.load_border(dim),
            border_sent: None,
        })
    }

    /// `MinecraftServer.prepareLevels` activates every level's saved tickets:
    /// a level holding saved `PORTAL` tickets opens with the world and ticks
    /// while they hold it (`shouldKeepDimensionActive`).
    pub(super) fn open_saved_portal_levels(&mut self) -> Result<()> {
        if self.options.execution_role.is_client() {
            return Ok(());
        }
        let back = self.dim;
        for dim in [Dim::Overworld, Dim::Nether, Dim::End] {
            if dim == back || self.levels.parked.contains_key(&dim) {
                continue;
            }
            let save = SaveDir::new(
                horizon_client::save::dimension_dir(&self.world_root, dim.id()),
                self.level_shapes.get(dim.id()),
            );
            if !save.load_portal_tickets().is_empty() {
                self.enter_level(dim)?;
                self.enter_level(back)?;
            }
        }
        Ok(())
    }

    /// `MinecraftServer.prepareLevels`: before the first tick, the chunks the
    /// open levels' restored tickets hold (forced chunks, a native mod's) are
    /// loaded (`ChunkLoadCounter`). The first tick then finds them ticking
    /// and unpacks their saved block and fluid ticks at the game time they
    /// were saved at (`startTickingChunk`), not at whichever tick they finish
    /// streaming in.
    pub(super) fn prepare_levels(&mut self) -> Result<()> {
        if self.options.execution_role.is_client() {
            return Ok(());
        }
        let deadline = Instant::now() + Duration::from_secs(600);
        self.wait_for_ticket_chunks(deadline);
        let back = self.dim;
        for dim in [Dim::Overworld, Dim::Nether, Dim::End] {
            if dim != back && self.levels.parked.contains_key(&dim) {
                self.enter_level(dim)?;
                self.wait_for_ticket_chunks(deadline);
            }
        }
        if self.dim != back {
            self.enter_level(back)?;
        }
        Ok(())
    }

    /// Stream the level in place until each chunk its tickets make block
    /// ticking is a ticking chunk (lit, with its neighbours loaded).
    fn wait_for_ticket_chunks(&mut self, deadline: Instant) {
        // Forced chunks are checked two chunks out (their ticket block ticks one).
        // A `PORTAL` ticket's chunks are streamed as an area of their own (the
        // network tick does it, which hasn't run yet), whose chunks out to its
        // radius can become ticking chunks: those are waited for.
        let portal = self.portal_ticket_areas();
        for area in &portal {
            if !self.streaming.extra.contains(area) {
                self.streaming.extra.push(*area);
            }
        }
        loop {
            self.update_chunk_tickets();
            let levels = self.tickets.levels();
            let streaming = &self.streaming;
            let ready = streaming
                .forced
                .iter()
                .map(|&c| (c, 2))
                .chain(portal.iter().copied())
                .all(|((cx, cz), reach)| {
                    (cz - reach..=cz + reach).all(|z| {
                        (cx - reach..=cx + reach).all(|x| {
                            !levels.loaded_ticking(x, z) || super::ticking_chunk(&self.world, x, z)
                        })
                    })
                });
            if ready {
                return;
            }
            if Instant::now() >= deadline {
                log::warn!("Horizon: the chunks of the saved tickets did not load");
                return;
            }
            self.update_world();
            self.streaming.wait(Duration::from_millis(5));
        }
    }

    /// Exchange the level in place with `p`.
    fn swap_level(&mut self, p: &mut Parked) {
        use std::mem::swap;
        swap(&mut self.world, &mut p.world);
        // The carrier first-use gate belongs to the world, not the
        // level: a level swap keeps it.
        self.world.carrier_data_existed_at_open = p.world.carrier_data_existed_at_open;
        swap(&mut self.streaming, &mut p.streaming);
        let mut locator = self.end.locator();
        swap(&mut locator, &mut p.locator);
        self.end.set_locator(locator);
        self.share_spawn_conditions();
        swap(&mut self.save, &mut p.save);
        swap(&mut self.block_ticks, &mut p.block_ticks);
        swap(&mut self.tick_chunks, &mut p.tick_chunks);
        swap(&mut self.tickets, &mut p.tickets);
        swap(&mut self.random_tick_sections, &mut p.random_tick_sections);
        swap(&mut self.level_events, &mut p.level_events);
        swap(&mut self.portals.entities, &mut p.portal_states);
        swap(&mut self.portals.sections, &mut p.portal_sections);
        swap(&mut self.falling, &mut p.falling);
        swap(&mut self.paintings, &mut p.paintings);
        swap(&mut self.item_frames, &mut p.item_frames);
        swap(&mut self.armor_stands, &mut p.armor_stands);
        swap(&mut self.cushions, &mut p.cushions);
        swap(&mut self.others, &mut p.others);
        swap(&mut self.fangs, &mut p.fangs);
        swap(&mut self.boats, &mut p.boats);
        swap(&mut self.carts, &mut p.carts);
        swap(&mut self.jukebox_sounds, &mut p.jukebox_sounds);
        swap(&mut self.bolts, &mut p.bolts);
        swap(&mut self.crystals, &mut p.crystals);
        swap(&mut self.dragon, &mut p.dragon);
        swap(&mut self.breath_clouds, &mut p.breath_clouds);
        swap(&mut self.tridents, &mut p.tridents);
        swap(&mut self.rockets, &mut p.rockets);
        swap(&mut self.ominous_spawners, &mut p.ominous_spawners);
        swap(&mut self.tnts, &mut p.tnts);
        swap(&mut self.ordered.restarts, &mut p.tick_restarts);
        swap(&mut self.hoppers, &mut p.hoppers);
        swap(&mut self.mobs, &mut p.mobs);
        swap(&mut self.knot_ids, &mut p.knot_ids);
        swap(&mut self.resident_entities.knots, &mut p.knots);
        swap(&mut self.resident_entities.general, &mut p.general);
        swap(&mut self.creatures.projectiles, &mut p.projectiles);
        swap(&mut self.dropped, &mut p.dropped);
        swap(&mut self.orbs, &mut p.orbs);
        swap(&mut self.persist, &mut p.persist);
        swap(&mut self.lectern_pulses, &mut p.lectern_pulses);
        swap(&mut self.lid_openers, &mut p.lid_openers);
        swap(&mut self.lan_events, &mut p.lan_events);
        swap(&mut self.lan_block_events, &mut p.lan_block_events);
        swap(&mut self.lan_entity_events, &mut p.lan_entity_events);
        swap(&mut self.lan_critical_hits, &mut p.lan_critical_hits);
        let (a, b) = (&mut self.be_anims, &mut p.anims);
        swap(&mut a.chests, &mut b.chests);
        swap(&mut a.books, &mut b.books);
        swap(&mut a.shulkers, &mut b.shulkers);
        swap(&mut a.bells, &mut b.bells);
        swap(&mut a.conduits, &mut b.conduits);
        swap(&mut a.skulls, &mut b.skulls);
        swap(&mut a.pots, &mut b.pots);
        swap(&mut a.gateways, &mut b.gateways);
        swap(&mut a.gateway_servers, &mut b.gateway_servers);
        swap(&mut a.conduit_servers, &mut b.conduit_servers);
        self.raiding.swap_level(&mut p.raiding);
        self.sculk.swap_level(&mut p.sculk);
        self.creaking.swap_level(&mut p.creaking);
        swap(&mut self.spawn_scratch, &mut p.spawn_scratch);
        swap(&mut self.border, &mut p.border);
        // What only the client draws starts over.
        self.clear_client_particles();
    }

    /// Put `target`'s level in place, opening it first if it is not loaded.
    /// The level that was in place is kept with the others; the shared
    /// clock and weather go with the swap.
    pub(super) fn enter_level(&mut self, target: Dim) -> Result<()> {
        // A client holds no levels.
        if self.options.execution_role.is_client() {
            return Ok(());
        }
        if target != self.dim {
            let (mut p, opened) = match self.levels.parked.remove(&target) {
                // A level in use is not on its way out: a close it had started is abandoned,
                // and only the writes it queued before still land.
                Some(mut p) => {
                    p.closing = false;
                    (p, false)
                }
                None => (self.open_level(target)?, true),
            };
            p.world.game_time = self.world.game_time;
            // The level's day time is its default clock's; a level without one follows the
            // Overworld's.
            let clock = self.level_clock_name(target);
            p.world.day_time = self.switch_clock(&clock);
            p.world.end_clock = self.world.end_clock;
            p.world.weather = self.world.weather;
            // The game rules and the difficulty are the server's, not a
            // level's: natural regeneration holds in every dimension.
            p.world.health_rules = self.world.health_rules;
            // The sculk tables' watched blocks are built once per game.
            p.world.watched = std::mem::take(&mut self.world.watched);
            self.swap_level(&mut p);
            // The tracked entities go with their level: a level's moves, read before its tick
            // describes its entities again, would otherwise meet the last level's (the anticheat
            // missed the boat under a player while anyone was in another dimension).
            let states = self.lan.as_mut().map(|h| &mut h.states);
            if let Some(states) = states {
                std::mem::swap(states, &mut p.lan_states);
            }
            if let Some(h) = self.lan.as_mut() {
                std::mem::swap(&mut h.border_sent, &mut p.border_sent);
            }
            p.idle = 0;
            let left = std::mem::replace(&mut self.dim, target);
            self.levels.parked.insert(left, p);
            if opened {
                // The new streaming's radius and meshing options.
                self.apply_settings();
                // ServerLevel's constructor reads the level's raids.
                self.read_raids();
                self.native_level_opened(target);
            }
        }
        if let Some(h) = self.lan.as_mut() {
            h.hub.set_dimension(target.id());
        }
        self.sync_hub_clocks();
        Ok(())
    }

    /// Follow the direct mount of a travelling player through complete source objects.
    /// A later player in the same travelling stack finds no source root: the first
    /// arrival already moved it, keeping every id that the guests' mounts name.
    pub(super) fn parked_vehicle_root(&self, from: Dim, mut id: u32) -> Option<u32> {
        let p = self.levels.parked.get(&from)?;
        for _ in 0..=p.mobs.len() + p.persist.off_level.len() + 1 {
            let mob = p
                .mobs
                .iter()
                .find(|m| m.id == id)
                .or_else(|| {
                    p.persist
                        .off_level
                        .get(&id)
                        .and_then(persist::OffLevel::as_mob)
                })
                .filter(|m| !m.removed);
            if let Some(parent) = mob.and_then(|m| m.extra.mount) {
                id = parent;
                continue;
            }
            let present = mob.is_some()
                || p.boats.iter().any(|b| b.id == id)
                || p.carts.iter().any(|c| c.id == id);
            let removed = p.persist.removals.iter().chain(p.persist.destroying_vehicles.iter()).any(|target| matches!(
                target, commands::Target::Mob(n) | commands::Target::Boat(n) | commands::Target::Cart(n) if *n == id
            ));
            return (present && !removed).then_some(id);
        }
        None
    }

    /// Move the source tree's object memory and take its Mob descendants in
    /// canonical postorder. The caller owns the extracted root and admits it last.
    /// Transfer removal preserves lifetimes; residency is decided in the destination.
    pub(super) fn move_entity_memory(&mut self, from: Dim, root: u32) -> Vec<Mob> {
        let Some(p) = self.levels.parked.get_mut(&from) else {
            return Vec::new();
        };
        let mut edges: Vec<_> = p
            .mobs
            .iter()
            .chain(
                p.persist
                    .off_level
                    .values()
                    .filter_map(persist::OffLevel::as_mob),
            )
            .filter(|mob| !mob.removed)
            .map(|mob| (mob.extra.mount, mob.extra.vehicle_order, mob.id))
            .collect();
        edges.sort_by_key(|&(_, order, id)| (order, id));
        let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
        for (parent, _, id) in edges {
            if let Some(parent) = parent {
                children.entry(parent).or_default().push(id);
            }
        }
        let mut pending = vec![(root, false)];
        let mut seen = std::collections::HashSet::new();
        let mut ordered = Vec::new();
        while let Some((id, done)) = pending.pop() {
            if done {
                ordered.push(id);
            } else if seen.insert(id) {
                pending.push((id, true));
                if let Some(riders) = children.get(&id) {
                    pending.extend(riders.iter().rev().map(|&rider| (rider, false)));
                }
            }
        }
        let mut moved = Vec::new();
        for id in ordered {
            p.persist.move_object_to(id, &mut self.persist);
            if id == root {
                continue;
            }
            if let Some(state) = p.portal_states.remove(&id) {
                self.portals.entities.insert(id, state);
            }
            let mob = if let Some(index) = p.mobs.iter().position(|m| m.id == id) {
                Some(p.mobs.remove_for_transfer(index))
            } else {
                // Boats and carts are only roots, which the caller takes.
                match p.persist.off_level.remove(&id) {
                    Some(persist::OffLevel::Mob(mob)) => Some(mob),
                    _ => None,
                }
            };
            if let Some(mut mob) = mob {
                mob.join = horizon_client::join_seq::JoinSeq::UNJOINED;
                moved.push(mob);
            }
        }
        moved
    }

    /// An already-owned body changes level, never construction identity.
    /// `PersistentEntitySectionManager.addEntityUuid` decides its residency:
    /// a body whose UUID an earlier arrival took is retained off-level.
    pub(super) fn admit_transferred_object(&mut self, mut object: persist::OffLevel) {
        let id = match &mut object {
            persist::OffLevel::Mob(mob) => {
                self.rebind_transferred_entity(mob.id, mob);
                mob.id
            }
            persist::OffLevel::Boat(boat) => {
                self.rebind_transferred_entity(boat.id, boat);
                boat.id
            }
            persist::OffLevel::Cart(cart) => {
                self.rebind_transferred_entity(cart.id, cart);
                cart.id
            }
        };
        self.journal_arrival(id);
        if self.uuid_admission_refused(self.persist.uuids.get(&id).copied()) {
            self.persist.off_level.insert(id, object);
            return;
        }
        match object {
            persist::OffLevel::Mob(mob) => self.mobs.push(mob),
            persist::OffLevel::Boat(boat) => self.boats.push(boat),
            persist::OffLevel::Cart(cart) => self.carts.push(cart),
        }
    }

    /// Far terrain for a client in `dim`: the level in place may be another
    /// one, the server ticking its levels in turn.
    pub(super) fn lod_source_in(
        &self,
        dim: Dim,
    ) -> Option<Arc<dyn horizon_client::streaming::LodSource>> {
        if dim == self.dim {
            return Some(self.streaming.lod_source());
        }
        let parked = self.levels.parked.get(&dim)?;
        Some(parked.streaming.lod_source())
    }

    /// Put `dim`'s level in place for a command whose source is there
    /// (`execute in`, `execute at` a player in another level), opening it if
    /// nobody is in it; returns the level to put back afterwards. Only a
    /// server holds other levels.
    pub(super) fn enter_command_level(&mut self, dim: Dim) -> Option<Dim> {
        if self.options.execution_role.is_client() || dim == self.dim {
            return None;
        }
        let back = self.dim;
        match self.enter_level(dim) {
            Ok(()) => Some(back),
            Err(e) => {
                log::error!("could not open {}: {e}", dim.id());
                None
            }
        }
    }

    /// One server tick (`MinecraftServer.tickServer`): each level with
    /// players or forced chunks in turn (`tickChildren`), its chunks streamed
    /// and loaded and its world ticked. The first pass reads the network and runs the
    /// tick's shared parts (the clock, the weather, functions, the autosave);
    /// the next ones only their level. While `paused`, only the connections
    /// run. The native server tick hook brackets it (`ServerTickStart` before
    /// the first level, `ServerTickEnd` after the last).
    pub(super) fn tick_server(&mut self, paused: bool) -> Result<()> {
        self.with_residency_tick(|game| game.tick_server_body(paused))
    }

    fn tick_server_body(&mut self, paused: bool) -> Result<()> {
        // `tickRateManager.tick()` once per server tick, before any level
        // (each `ServerLevel.tick` only reads `runsNormally`), and the native
        // `ServerTickStart` before that level work (NeoForge's
        // `ServerTickEvent.Pre`). A paused pass and a held server tick nothing.
        let running = if !paused && !self.agent.hold_server {
            self.clock.check_sprint_completion();
            if let Some(report) = self.clock.take_sprint_report() {
                // ServerTickRateManager.finishTickSprint: the server's sendSuccess.
                self.console_broadcast = Some(true);
                self.sprint_report(&report);
                self.console_broadcast = None;
                self.send_admin_feedback(None);
                self.admin_feedback.clear();
                // Then onTickRateChanged, its mods outside the server's broadcast.
                self.on_tick_rate_changed();
            }
            Some(self.clock.tick_world_runs())
        } else {
            None
        };
        let server = self.serving.is_some() || self.options.dedicated.is_some();
        if server {
            self.run_requested_saves();
        }
        // A config edited while the server is paused (empty) still reloads.
        // The parts outside every level get spans of their own, so that a slow
        // tick's line and `horizon-report` say when a mod's tick hook took it.
        if server {
            let _span = horizon_client::spans::guard("mod_configs_poll");
            self.native_configs_poll();
        }
        if let Some(running) = running
            && server
        {
            let _span = horizon_client::spans::guard("mods_server_tick_start");
            self.native_lifecycle_server_tick_start(running);
        }
        let observe_level =
            running.is_some() && (self.serving.is_some() || self.options.dedicated.is_some());
        let dims = self.levels_to_tick();
        let order = dims.clone();
        let n = dims.len();
        let mut result = Ok(());
        for (k, dim) in dims.into_iter().enumerate() {
            // Each level's share of the tick (the first one also reads the network and runs the
            // tick's shared parts): `horizon-report`'s `level_*` spans.
            horizon_threads::work::set_dim(match dim {
                Dim::Overworld => 1,
                Dim::Nether => 2,
                Dim::End => 3,
                Dim::Extra(_) => 0,
            });
            let _level_span = horizon_client::spans::guard(match dim {
                Dim::Overworld => "level_overworld",
                Dim::Nether => "level_the_nether",
                Dim::End => "level_the_end",
                Dim::Extra(_) => "level_extra",
            });
            if let Err(e) = self.enter_level(dim) {
                result = Err(e);
                break;
            }
            self.level_pass = Pass {
                first: k == 0,
                last: k + 1 == n,
                running,
                packets_drained: false,
            };
            if observe_level && let Some(level) = self.level(dim.id()) {
                self.native_level_tick_pre(level);
            }
            horizon_client::spans::time("dedicated_streaming", || {
                self.update_dedicated_streaming();
            });
            horizon_client::spans::time("update_world", || self.update_world());
            let tick_span = horizon_client::spans::guard("level_tick");
            result = horizon_client::block_contact::in_level(
                (self.serving.is_some() || self.options.dedicated.is_some()).then_some(dim.id()),
                || {
                    if paused {
                        self.tick_lan(true);
                        Ok(())
                    } else if k == 0 {
                        self.tick()
                    } else {
                        let running = running.unwrap_or_else(|| self.clock.world_runs());
                        // ClientLevel.tick: the sky flash fades in a frozen level too.
                        self.tick_weather_effects();
                        self.tick_world(running)
                    }
                },
            );
            drop(tick_span);
            self.drain_entity_inside();
            if result.is_err() {
                break;
            }
            if observe_level && let Some(level) = self.level(dim.id()) {
                self.native_level_tick_post(level);
            }
            #[cfg(feature = "state-digest")]
            self.state_digest_tick(dim, running);
            horizon_client::spans::time("server_debug", || self.publish_server_debug(dim));
            // An entity that crossed into a level that already ticked this
            // server tick is in it when the tick ends (`addDuringTeleport`).
            if running == Some(true) {
                let _span = horizon_client::spans::guard("late_portal_arrivals");
                let mut moved = false;
                for &earlier in &order[..k] {
                    if !self.arrival_chunks(earlier).is_empty() && self.enter_level(earlier).is_ok()
                    {
                        self.land_portal_arrivals();
                        moved = true;
                    }
                }
                if moved && k + 1 < n {
                    self.enter_level(dim)?;
                }
            }
        }
        // The levels only `Loaded` areas name: streamed and sent, never ticked.
        if result.is_ok() {
            result = self.run_view_passes(&order);
        }
        if let Some(&first) = order.first()
            && self.dim != first
        {
            let _ = self.enter_level(first);
        }
        self.level_pass = Pass::default();
        if result.is_err() {
            self.native_lifecycle.cancel_server_tick();
        }
        result?;
        // The levels mods asked to open or close: now that no level is in the middle of a tick.
        self.apply_runtime_levels();
        let closed = {
            let _span = horizon_client::spans::guard("close_idle_levels");
            self.close_idle_levels()
        };
        if let Err(error) = closed {
            self.native_lifecycle.cancel_server_tick();
            return Err(error);
        }
        // `ServerTickEnd`: an errored tick or an idle-close failure (at
        // the `?` above) never reaches this site.
        {
            let _span = horizon_client::spans::guard("mods_server_tick_end");
            self.native_lifecycle_server_tick_end(paused);
        }
        Ok(())
    }

    /// `tick_server` as a server thread runs it, timed whenever spans are on.
    pub(super) fn tick_server_timed(&mut self, paused: bool) -> Result<()> {
        self.tick_server_spans(paused, |_| {})
    }

    /// `tick_server_timed`, handing the tick's timing to `record` while spans are on.
    pub(super) fn tick_server_spans(
        &mut self,
        paused: bool,
        record: impl FnOnce(TickTiming),
    ) -> Result<()> {
        let started = Instant::now();
        // The game carried on past any turn a caught panic interrupted before this tick.
        self.lan_interrupted_turns.clear();
        horizon_threads::work::tick_start();
        let result = self.tick_server_recorded(paused, record);
        horizon_threads::work::tick_end(started.elapsed().as_nanos() as u64);
        result
    }

    fn tick_server_recorded(
        &mut self,
        paused: bool,
        record: impl FnOnce(TickTiming),
    ) -> Result<()> {
        if self.options.spikes.is_none() && !horizon_client::spans::recording_here() {
            return self.tick_server(paused);
        }
        // Discard between-tick work (such as the chunk pump) before timing this tick.
        horizon_client::spans::take_raw();
        let (started, cpu) = (Instant::now(), horizon_client::spans::thread_cpu());
        let result = self.tick_server(paused);
        let wall = started.elapsed().as_secs_f32() * 1000.0;
        let cpu = horizon_client::spans::thread_cpu()
            .zip(cpu)
            .map(|(end, start)| (end - start).as_secs_f32() * 1000.0);
        // Taken every tick: nothing else takes this thread's spans.
        let raw = horizon_client::spans::take_raw();
        // The spans go to the slow ring; whose packets or events took the tick stays in the owner's log.
        let notes = horizon_client::spans::take_notes();
        if let Some(threshold) = self.options.spikes
            && wall > threshold
            && !notes.is_empty()
        {
            log::info!(
                "Horizon: {wall:.1} ms tick was busy with: {}",
                slowest_steps(&notes)
            );
        }
        record(TickTiming {
            wall_ms: wall,
            cpu_ms: cpu,
            raw,
        });
        result
    }

    /// What `Game::tick` runs for a server's first level, after the window's lead.
    pub(super) fn tick_first_level(&mut self) -> Result<()> {
        // Only a server ticks its own world from here: its players are guests, swapped in for
        // their turns (`as_guest`), and the window's player is a client of it.
        self.ticks += 1;
        self.repro_tick();
        let window = !self.is_logical_server();
        if window {
            self.track_experience_display();
            self.tick_enchanting_book();
        }
        // `/tick freeze` stops the world but not the player.
        let running = match self.level_pass.running {
            Some(running) => running,
            None => self.clock.world_runs(),
        };
        if window {
            // GameRenderer.tick: a wither's bar darkens the world, then fades;
            // the activated item counts down.
            self.boss_overlay.tick_darken(running);
            self.tick_item_activation();
        }
        span("mods_inventory_ticks", || self.native_inventory_ticks());
        if running {
            // `MinecraftServer.tickChildren`: `commandFunctions` runs before
            // any level ticks, so a tick function reads the game time and the
            // day clock from before this tick. Vanilla's SavedData is loaded
            // long before this point; the lazy scoreboard/storage read has to
            // be done by then too, or a load tag writes over what it restores.
            self.load_scoreboard();
            span("tick_functions", || self.tick_functions());
            self.world.game_time += 1;
            if self
                .ui
                .rules
                .bool_at(horizon_client::gamerules::vanilla::ADVANCE_WEATHER)
            {
                self.world.weather.tick_timers(&mut self.world.random);
            }
            self.world.weather.tick_levels();
        } else if self.clock.still() {
            // Under `HORIZON_STILL` a frozen world's rain and thunder sit at their levels, as a
            // shader pack's smoothed values do, so `/weather rain` shows full rain without
            // stepping the world (and its mobs) through the fade.
            self.world.weather.settle_levels();
        }
        // ClientLevel.tick: the sky flash fades in a frozen level too.
        self.tick_weather_effects();
        let _ = span("player_autosaves", || self.finish_player_autosaves(false));
        // MinecraftServer.tickServer's countdown, then autoSave: three
        // hundred seconds of ticks at the rate the server runs at now.
        self.ticks_until_autosave -= 1;
        self.autosaving = self.ticks_until_autosave <= 0;
        if let Some(serving) = &self.serving {
            serving.set_saving(self.autosaving);
        }
        if self.autosaving {
            span("autosave", || self.save_game_with(false))?;
            self.ticks_until_autosave = self.next_autosave_interval();
            if let Some(serving) = &self.serving {
                serving.set_saving(false);
            }
        }
        if running {
            self.advance_clock();
            // `ServerLevel.tickTime`, once the clock has moved on.
            span("scheduled_functions", || self.run_scheduled_functions());
        }
        if window && self.use_delay > 0 {
            self.use_delay -= 1;
        }
        self.tick_totem();
        self.update_totem_ready();
        span("tick_command_extras", || self.tick_command_extras());
        self.tick_world(running)
    }

    /// The end of `MinecraftServer.runServer`'s loop for a server thread:
    /// the next tick starts one tick length after this one, or straight away
    /// while the sprint runs (`checkShouldSprintThisTick`). A server more
    /// than two seconds behind skips the ticks it missed: that returns how far
    /// behind it was, for the overload warning.
    pub(super) fn wait_for_next_server_tick(
        &mut self,
        next: &mut Instant,
        paused: bool,
    ) -> Option<Duration> {
        let now = Instant::now();
        if !paused && self.clock.sprinting() {
            *next = now;
            return None;
        }
        *next += self.clock.tick_duration();
        if *next > now {
            // The chunks loading (the players', force-loaded ones, a
            // `/pregen` task) keep the workers fed until the next tick.
            self.pump_chunks(*next, paused);
            let now = Instant::now();
            if *next > now {
                std::thread::sleep(*next - now);
            }
        } else if now - *next > Duration::from_secs(2) {
            let behind = now - *next;
            *next = now;
            return Some(behind);
        }
        None
    }

    /// The levels this server tick runs, in `MinecraftServer.levels` order
    /// (the Overworld, the Nether, the End): every level with a player in
    /// it, forced chunks or a `/pregen` task, or the Overworld while idle.
    pub(super) fn levels_to_tick(&self) -> Vec<Dim> {
        let guests = self
            .lan
            .as_ref()
            .map_or(&[][..], |h| h.hub.guests.as_slice());
        let mut dims: Vec<Dim> = Dim::all()
            .into_iter()
            .filter(|d| {
                // `/forceload`ed chunks, or a native mod's tickets.
                let held = |s: &Streaming| !s.forced.is_empty();
                let forced = if *d == self.dim {
                    held(&self.streaming)
                } else {
                    self.levels
                        .parked
                        .get(d)
                        .is_some_and(|p| held(&p.streaming))
                };
                guests.iter().any(|g| g.dimension == d.id() && g.participates_in_player_simulation())
                    // A viewer count not yet told to 0 (`tick_mod_container_viewers`).
                    || self.mod_container_viewers.keys().any(|(k, _)| *k == d.id())
                    || self.player_bed_levels().any(|b| b == *d)
                    || forced
                    // A game test runs in its own level, players or not.
                    || self.game_tests.has_tests_in(*d)
                    || self.pregen_level() == Some(*d)
                    // A pending portal arrival keeps its destination level
                    // running so it can land and tick (the portal ticket).
                    || !self.arrival_chunks(*d).is_empty()
                    // A portal ticket keeps its dimension active
                    // (`TicketType.shouldKeepDimensionActive`).
                    || !self.portal_tickets_in(*d).is_empty()
                    // So does a viewed `Ticking` area (arismoko/horizon#3716), for its chunks only.
                    || self.viewed_level(*d) == Some(true)
            })
            .collect();
        if dims.is_empty() {
            dims.push(Dim::Overworld);
        }
        dims
    }

    /// The portal tickets of `dim`'s level (`placePortalTicket`, or saved).
    pub(super) fn portal_tickets_in(&self, dim: Dim) -> &[PortalTicket] {
        if dim == self.dim {
            self.tickets.portal_tickets()
        } else {
            self.levels
                .parked
                .get(&dim)
                .map_or(&[], |p| p.tickets.portal_tickets())
        }
    }

    /// The areas the level in place's portal tickets keep streamed: around each
    /// exit its block-ticking chunks and their neighbours, the ring vanilla
    /// keeps `FULL` (level 33), so the outer block-ticking ring is a ticking
    /// chunk too; and at least the chunks its expiry waits for
    /// (`portal_ticket_can_expire`).
    pub(super) fn portal_ticket_areas(&self) -> Vec<((i32, i32), i32)> {
        let block_ticking = i32::from(horizon_client::ticket_levels::BLOCK_TICKING);
        self.tickets
            .portal_tickets()
            .iter()
            .map(|t| (t.chunk, (block_ticking - t.level).max(1)))
            .collect()
    }

    /// Count the parked levels' empty ticks, and save and close the ones
    /// (other than the Overworld) nobody has been in for a while.
    ///
    /// A close takes two ticks. The first queues the level's save and does not wait for the
    /// disk (`save_level_queued`): waiting for it here made every idle close a p99 spike in
    /// the tick (`SaveDir::flush`). The level leaves memory on a later tick, once
    /// `SaveDir::writes_finished` says the writer has done everything that save queued.
    pub(super) fn close_idle_levels(&mut self) -> Result<()> {
        let mut occupied = self.levels_to_tick();
        // A level a viewed area names, ticked or not, or one a client still holds chunks of.
        occupied.extend(Dim::all().into_iter().filter(|d| self.needs_view_pass(*d)));
        // A vehicle waiting there for its rider's arrival in another level.
        let waiting: Vec<Dim> = self
            .levels
            .parked
            .keys()
            .copied()
            .filter(|d| self.vehicle_waits_in(*d))
            .collect();
        // A level at its idle limit, and one whose queued save may be on disk.
        let (mut to_save, mut to_finish) = (Vec::new(), Vec::new());
        for (dim, p) in &mut self.levels.parked {
            if occupied.contains(dim) || waiting.contains(dim) {
                p.not_closing();
                continue;
            }
            if p.closing {
                to_finish.push(*dim);
                continue;
            }
            p.idle += 1;
            if *dim != Dim::Overworld && p.idle >= IDLE_TICKS {
                to_save.push(*dim);
            }
        }
        for dim in to_save {
            self.begin_closing_parked_level(dim)?;
        }
        for dim in to_finish {
            self.finish_closing_parked_level(dim)?;
        }
        Ok(())
    }

    /// The first half of a close: queue the level's save without waiting for the disk, and mark
    /// the level `closing`. It stays parked, with its `SaveDir`, until the second half.
    fn begin_closing_parked_level(&mut self, dim: Dim) -> Result<()> {
        let back = self.dim;
        self.enter_level(dim)?;
        // Vanilla saves the level's chunks before their unload hooks, and before the entities
        // `save_entities` brings back hear their load.
        self.native_chunks_loaded();
        let saved = self.save_level_queued();
        // Whatever the save did, the level in place is the one before.
        self.enter_level(back)?;
        saved?;
        if let Some(p) = self.levels.parked.get_mut(&dim) {
            p.closing = true;
        }
        Ok(())
    }

    /// The second half of a close: the pass that finds the level's queued save done ends the
    /// close and its chunks and entities leave memory. A level that came back into use keeps
    /// open, and the writes it queued land on their own; so does a level whose close a write
    /// cancelled after the pass listed it.
    fn finish_closing_parked_level(&mut self, dim: Dim) -> Result<()> {
        let written = match self.levels.parked.get(&dim) {
            None => return Ok(()),
            Some(p) => p.save.writes_finished(),
        };
        match written {
            Ok(true) => {}
            Ok(false) => return Ok(()),
            Err(why) => {
                // Not saved, as a save that fails: the level stays parked. The error stays on
                // its folder, so it is not asked for again on every tick.
                if let Some(p) = self.levels.parked.get_mut(&dim) {
                    p.not_closing();
                }
                log::warn!("Horizon: closing {} was not saved: {why}", dim.id());
                return Err(why);
            }
        }
        // Back in use while its save was queued: a ticket or a viewed area came with it, or a
        // vehicle waits there. It stays open, and the writes it queued land on their own.
        if self.level_in_use(dim) {
            if let Some(p) = self.levels.parked.get_mut(&dim) {
                p.not_closing();
            }
            return Ok(());
        }
        // The pass listed this level for the drop on its `closing` flag, but the two happen
        // apart in the pass: another level's save hooks run between them
        // (`begin_closing_parked_level`), and a write they make through a parked accessor
        // cancels this close. What such a write left must not go with the save already on
        // disk: the level stays parked, and is saved again on its next close.
        if !self.levels.parked.get(&dim).is_some_and(|p| p.closing) {
            return Ok(());
        }
        let back = self.dim;
        self.drop_parked_level(dim, back)
    }

    /// Whether `dim`'s level is in use now: ticked, named by a viewed area, or holding a vehicle
    /// that waits for its rider (`close_idle_levels`). A level in use is not closed.
    fn level_in_use(&self, dim: Dim) -> bool {
        self.levels_to_tick().contains(&dim)
            || self.needs_view_pass(dim)
            || self.vehicle_waits_in(dim)
    }

    /// Take `dim` out of memory and put `back` in place: the tail of a close, once the level's
    /// save is queued or on disk. The mods hear its chunks unload, then the level.
    fn drop_parked_level(&mut self, dim: Dim, back: Dim) -> Result<()> {
        self.enter_level(dim)?;
        // A chunk that loaded while the level was parked hears its load here, before its unload.
        self.native_chunks_loaded();
        let chunks: Vec<_> = self.world.chunks.values().cloned().collect();
        self.native_chunks_unloaded(&chunks);
        // Its entities' attachments are saved with them; they leave memory too.
        for id in self.persist.uuids.keys() {
            self.entity_attachments.remove(id);
        }
        self.enter_level(back)?;
        self.levels.parked.remove(&dim);
        self.native_level_closed(dim);
        Ok(())
    }

    /// Save the parked level `dim` and drop it: the mods hear its chunks and the level unload. A
    /// level a mod opened at run time stays known, and opens again when entered, as any other.
    pub(super) fn close_parked_level(&mut self, dim: Dim) -> Result<()> {
        // A level whose close is waiting for its queued save is saved again here: whatever
        // reached it since that save (a mod's raid, a forced chunk) must not go with the level.
        // Entering it clears `closing`, so the close this finishes is the save made below.
        let back = self.dim;
        self.enter_level(dim)?;
        // Vanilla saves the level's chunks before their unload hooks.
        self.native_chunks_loaded();
        if let Err(why) = self.save_level(true) {
            // Not saved: the level stays parked, and the level in place is the one before.
            self.enter_level(back)?;
            return Err(why);
        }
        self.drop_parked_level(dim, back)
    }

    /// A server's world opened: level.dat is read and no chunk is loaded. The
    /// mods hear its saved mod list's differences (NeoForge's
    /// `ModMismatchEvent`), then the level it opens in.
    pub(super) fn native_world_opened(&mut self, dim: Dim) {
        self.native_level_opened(dim);
    }

    /// The mods hear `dim`'s level open (`LevelEvent.Load`), on a server.
    pub(super) fn native_level_opened(&mut self, dim: Dim) {
        if self.remote.is_some()
            || self.options.execution_role.is_client()
            || self.levels.heard_open.contains(&dim)
        {
            return;
        }
        let journal = self.resident_entities.table.borrow().producer();
        if dim == self.dim {
            self.bind_resident_owners(&journal, dim.key(), true);
        } else if let Some(parked) = self.levels.parked.get_mut(&dim) {
            parked.bind_resident_owners(&journal, dim.key(), true);
        }
        self.bind_dimension_attachments(dim);
        self.levels.heard_open.push(dim);
        self.native_level_load(dim.id());
        self.validate_mod_chunk_tickets(dim);
    }

    /// The mods hear `dim`'s level close (`LevelEvent.Unload`), once for
    /// each open they heard.
    pub(super) fn native_level_closed(&mut self, dim: Dim) {
        self.remove_level_fake_players(dim.id());
        if let Some(i) = self.levels.heard_open.iter().position(|d| *d == dim) {
            self.levels.heard_open.remove(i);
            self.native_level_unload(dim.id());
        }
        if dim == self.dim {
            self.world.water_tickets.clear();
        } else if let Some(world) = self.levels.parked_world_mut_of(dim.id()) {
            world.water_tickets.clear();
        }
        self.attachment_indexes.remove(&dim);
        self.passages.retire_level(dim.key());
    }

    /// Every level still open closes with the server, however it ends
    /// (`MinecraftServer.stopServer`'s `level.close()` loop), in vanilla's
    /// order of its levels: the Overworld, the Nether, the End.
    pub(super) fn native_levels_closing(&mut self) {
        for dim in Dim::all() {
            self.native_level_closed(dim);
        }
    }

    /// Save the levels mods asked for since the last tick, each as an autosave does but with its
    /// chunks on disk when it ends: `level_save` fires for each that was written, and one that
    /// could not be is logged. A level not open any more is opened to be saved, like a parked one.
    /// The level in place is put back.
    pub(super) fn run_requested_saves(&mut self) {
        if Self::admit_entity_save().is_err() {
            return;
        }
        let requested = std::mem::take(&mut self.levels.requested_saves);
        if requested.is_empty() || self.remote.is_some() {
            return;
        }
        let back = self.dim;
        for dim in requested {
            if let Err(e) = self.enter_level(dim).and_then(|()| self.save_level(true)) {
                log::error!("saving {} on a mod's request failed: {e}", dim.id());
            }
        }
        if let Err(e) = self.enter_level(back) {
            log::error!(
                "returning to {} after a requested save failed: {e}",
                back.id()
            );
        }
    }

    /// Save every parked level (`MinecraftServer.saveAllChunks`); the level
    /// in place is the caller's to save.
    pub(super) fn save_parked_levels(&mut self, wait: bool) -> Result<()> {
        if self.levels.parked.is_empty() {
            return Ok(());
        }
        let back = self.dim;
        let dims: Vec<Dim> = self.levels.parked.keys().copied().collect();
        for dim in dims {
            // A level whose close is queued is saved here like any other, and the close goes
            // with `enter_level`: this path cannot tell whether the level came back into use
            // (a ticket from another level's save hook, a viewed area), so the drop stays with
            // `close_idle_levels`, which checks. It re-saves and re-closes at its next pass.
            self.enter_level(dim)?;
            self.save_level(wait)?;
        }
        self.enter_level(back)
    }
}

/// The steps a slow tick noted as its slowest (`spans::note`), whose packets or which player's event.
fn slowest_steps(notes: &[(f32, String)]) -> String {
    let list: Vec<String> = notes
        .iter()
        .map(|(ms, what)| format!("{what} {ms:.1}"))
        .collect();
    list.join(", ")
}

impl Game<'_> {
    /// Rebind only an already-owning transfer, before installing it in this level.
    /// Memory still belongs to the parked source until the caller moves it.
    pub(super) fn rebind_transferred_entity(
        &mut self,
        id: u32,
        entity: &mut impl horizon_mod_runtime::attachments::HasAttachmentLifetime,
    ) {
        let slot = entity.attachment_lifetime();
        if !slot.is_bound() {
            return;
        }
        let Some(level) = self.level(self.dim.id()) else {
            return;
        };
        let Some(map) = self.entity_attachments.get_mut(&id) else {
            return;
        };
        slot.bind(
            map,
            self.attachment_indexes.entry(self.dim).or_default(),
            mod_api::AttachmentHolder::Entity(mod_api::EntityRef::new(level.epoch(), id)),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn levels_clear_empties_parked_and_heard_open() {
        let mut levels = Levels::default();
        levels.heard_open.push(Dim::Nether);
        assert!(!levels.heard_open.is_empty());
        levels.clear();
        assert!(levels.heard_open.is_empty());
        assert!(levels.parked.is_empty());
    }

    /// A game on the prepared resources in `HORIZON_TEST_RESOURCES`, with no
    /// world opened and commands allowed.
    fn prepared_game() -> Game<'static> {
        use crate::game::{Audio, Options, Progress, Settings, load_resources};
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        // The game keeps its resources for the whole test.
        let resources: &'static _ = Box::leak(Box::new(loaded.resources));
        let mut game = Game::new(
            resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        game.build_registries();
        game.set_allow_commands(true);
        game
    }

    /// `WorldBorderCommand.setSize` from `/worldborder add` with a time: the
    /// new change lasts the ticks still running plus the argument
    /// (`getLerpTime() + time`), where `/worldborder set` replaces it.
    #[test]
    #[ignore = "daily"]
    fn worldborder_add_extends_a_change_in_flight() {
        let mut game = prepared_game();
        assert!(game.run_command("worldborder set 100 1000"));
        assert_eq!(game.border.remaining_ticks(), 1000);
        // Part way through the change, where the size is between its ends.
        for _ in 0..100 {
            game.border.tick();
        }
        assert_eq!(game.border.remaining_ticks(), 900);
        let from = game.border.size();
        assert!(from < 59_999_968.0 && from > 100.0);
        // The new change starts from that size and lasts the ticks still
        // running plus the argument, not just the argument.
        assert!(game.run_command("worldborder add -50 5s"));
        assert_eq!(game.border.target_size(), from - 50.0);
        assert_eq!(game.border.remaining_ticks(), 1000);
        // `set` starts over from the size the border has now.
        assert!(game.run_command("worldborder set 200 40"));
        assert_eq!(game.border.target_size(), 200.0);
        assert_eq!(game.border.remaining_ticks(), 40);
    }

    /// `ServerLevel.getWorldBorder`: `/worldborder` in one level leaves the others' alone.
    #[test]
    #[ignore = "daily"]
    fn each_level_keeps_its_own_world_border() {
        let mut game = prepared_game();
        assert!(game.run_command("worldborder set 1000"));
        assert!(game.run_command("execute in minecraft:the_nether run worldborder set 100"));
        assert_eq!(game.dim, Dim::Overworld);
        assert_eq!(game.border.size(), 1000.0);
        let nether = game.levels.parked_border(Dim::Nether).unwrap();
        assert_eq!(nether.size(), 100.0);
    }

    /// `Game::find_portals` asks every tick whether a palette of the level's chunks may hold a
    /// portal (`PortalSections::any`). A level keeps its answers while another level ticks: its
    /// unchanged chunks' palettes are not read again, though the other level has chunks at the
    /// same x and z (a server ticks the Overworld and the Nether one after the other).
    #[test]
    #[ignore = "daily"]
    fn a_level_keeps_its_portal_palettes_while_another_ticks() {
        let mut game = prepared_game();
        let columns = |game: &mut Game| {
            for x in 0..4 {
                for z in 0..4 {
                    let chunk = horizon_client::chunk::Chunk::empty(x, z);
                    game.world.chunks.insert((x, z), Arc::new(chunk));
                }
            }
        };
        // Asked as `level_may_hold_portal` asks, counting the palette entries read.
        let reads = |game: &mut Game| {
            let mut reads = 0;
            game.portals.sections.any(&game.world.chunks, |_| {
                reads += 1;
                false
            });
            reads
        };
        columns(&mut game);
        assert!(reads(&mut game) > 0, "the first ask reads the palettes");
        game.enter_level(Dim::Nether).unwrap();
        columns(&mut game);
        reads(&mut game);
        game.enter_level(Dim::Overworld).unwrap();
        assert_eq!(
            reads(&mut game),
            0,
            "the Overworld's palettes were read again"
        );
    }

    /// An idle close takes two passes: the one that reaches the idle limit only queues the
    /// level's save and does not wait for the disk, and the level leaves memory once that save
    /// is on it.
    #[test]
    #[ignore = "daily"]
    fn an_idle_close_keeps_the_level_until_its_save_is_on_disk() {
        use horizon_client::{
            chunk::Chunk,
            save::{LevelShape, dimension_dir},
        };
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let home = std::path::PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap())
            .join("idle-level-close");
        // A new world each run: nothing is in its region files yet.
        let _ = std::fs::remove_dir_all(&home);
        let world = home.join("saves/idle-level-close");
        let mut resources = None;
        let mut game = open_game(&mut resources, &root, &home, &world);
        // A chunk of the Nether that only this level holds, and that nothing has written.
        game.enter_level(Dim::Nether).unwrap();
        // The tables a save writes the heightmaps and block ticks with: nothing has decoded a
        // chunk of this level yet, so the save would write the chunk without them and the read
        // below would not find the block it holds.
        let _ = game
            .save
            .tables
            .get_or_init(|| Arc::new(horizon_client::save::SaveTables::new(&game.pack.assets)));
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        let mut chunk = Chunk::with_layout(0, 0, game.world.shape.column());
        chunk.set(1, 70, 2, stone.0);
        game.world.chunks.insert((0, 0), Arc::new(chunk));
        game.world.modified.insert((0, 0));
        game.enter_level(Dim::Overworld).unwrap();
        // Nobody is in it, and its idle count is one pass short of the limit.
        game.levels.parked_mut(Dim::Nether).unwrap().idle = IDLE_TICKS - 1;
        game.close_idle_levels().unwrap();
        let parked = game
            .levels
            .parked
            .get(&Dim::Nether)
            .expect("the level left memory on the pass that reached its idle limit");
        assert!(parked.closing, "the level is not on its way out");
        assert!(
            parked.world.chunks.contains_key(&(0, 0)),
            "the level's chunk left memory before its save"
        );
        // With its save on disk, the next pass ends the close.
        game.levels
            .parked
            .get(&Dim::Nether)
            .unwrap()
            .save
            .flush()
            .unwrap();
        game.close_idle_levels().unwrap();
        assert!(
            !game.levels.parked.contains_key(&Dim::Nether),
            "the level whose save was on disk stayed in memory"
        );
        let save = SaveDir::new(dimension_dir(&world, Dim::Nether.id()), LevelShape::NETHER);
        let read_back = save
            .load(0, 0, game.pack.assets.registry, world_biomes!(game))
            .expect("the closed level's chunk is not in the region file");
        assert_eq!(
            read_back.block(1, 70, 2),
            stone,
            "the chunk on disk is not the one the level held"
        );
    }

    /// A level whose close is queued is saved and closed in two passes, and something can still
    /// reach it in between: a mod's raid, a forced chunk, another level's save hook. Such a
    /// write cancels the close, so the level is not dropped with the save of the first pass, and
    /// the write is saved when the level is saved again.
    #[test]
    #[ignore = "daily"]
    fn a_write_while_an_idle_close_waits_keeps_the_level_for_its_save() {
        use horizon_client::{
            chunk::Chunk,
            save::{LevelShape, dimension_dir},
        };
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let home = std::path::PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap())
            .join("idle-close-write");
        let _ = std::fs::remove_dir_all(&home);
        let world = home.join("saves/idle-close-write");
        let mut resources = None;
        let mut game = open_game(&mut resources, &root, &home, &world);
        game.enter_level(Dim::Nether).unwrap();
        let _ = game
            .save
            .tables
            .get_or_init(|| Arc::new(horizon_client::save::SaveTables::new(&game.pack.assets)));
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        let gold = game
            .pack
            .assets
            .registry
            .default_state("minecraft:gold_block")
            .unwrap();
        let mut chunk = Chunk::with_layout(0, 0, game.world.shape.column());
        chunk.set(1, 70, 2, stone.0);
        game.world.chunks.insert((0, 0), Arc::new(chunk));
        game.world.modified.insert((0, 0));
        game.enter_level(Dim::Overworld).unwrap();
        game.levels.parked_mut(Dim::Nether).unwrap().idle = IDLE_TICKS - 1;
        game.close_idle_levels().unwrap();
        assert!(
            game.levels.parked.get(&Dim::Nether).unwrap().closing,
            "the level is not on its way out"
        );
        // What a mod's handle does: a change through the level's world, without entering it.
        let changed = game
            .levels
            .parked_world_mut_of(Dim::Nether.id())
            .expect("the parked level");
        let layout = changed.shape.column();
        let mut chunk = Chunk::with_layout(0, 0, layout);
        chunk.set(1, 70, 2, gold.0);
        changed.chunks.insert((0, 0), Arc::new(chunk));
        changed.modified.insert((0, 0));
        assert!(
            !game.levels.parked.get(&Dim::Nether).unwrap().closing,
            "a write did not cancel the queued close"
        );
        // The pass that finds the first save done must not drop the level it was changed in.
        game.levels
            .parked
            .get(&Dim::Nether)
            .unwrap()
            .save
            .flush()
            .unwrap();
        game.close_idle_levels().unwrap();
        assert!(
            game.levels.parked.contains_key(&Dim::Nether),
            "the level was dropped with a write of its own unsaved"
        );
        // Saved again, and closed with it: the write is in the region file.
        game.close_parked_level(Dim::Nether).unwrap();
        let save = SaveDir::new(dimension_dir(&world, Dim::Nether.id()), LevelShape::NETHER);
        let read_back = save
            .load(0, 0, game.pack.assets.registry, world_biomes!(game))
            .expect("the closed level's chunk is not in the region file");
        assert_eq!(
            read_back.block(1, 70, 2),
            gold,
            "the write that cancelled the close was not saved"
        );
    }

    /// The same write can arrive inside the closing pass itself: the pass lists the levels to
    /// drop before it saves the others (`close_idle_levels`), and a save hook of one of those
    /// others can still write to a listed level through a parked accessor. The finish the pass
    /// then runs for the level must see the cancelled close and leave the level parked, and
    /// the write goes with the level's next save, not with the one already on disk.
    #[test]
    #[ignore = "daily"]
    fn a_write_after_a_close_is_listed_keeps_the_level() {
        use horizon_client::{
            chunk::Chunk,
            save::{LevelShape, dimension_dir},
        };
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let home = std::path::PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap())
            .join("idle-close-listed");
        let _ = std::fs::remove_dir_all(&home);
        let world = home.join("saves/idle-close-listed");
        let mut resources = None;
        let mut game = open_game(&mut resources, &root, &home, &world);
        game.enter_level(Dim::Nether).unwrap();
        let _ = game
            .save
            .tables
            .get_or_init(|| Arc::new(horizon_client::save::SaveTables::new(&game.pack.assets)));
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        let gold = game
            .pack
            .assets
            .registry
            .default_state("minecraft:gold_block")
            .unwrap();
        let mut chunk = Chunk::with_layout(0, 0, game.world.shape.column());
        chunk.set(1, 70, 2, stone.0);
        game.world.chunks.insert((0, 0), Arc::new(chunk));
        game.world.modified.insert((0, 0));
        game.enter_level(Dim::Overworld).unwrap();
        game.levels.parked_mut(Dim::Nether).unwrap().idle = IDLE_TICKS - 1;
        game.close_idle_levels().unwrap();
        assert!(
            game.levels.parked.get(&Dim::Nether).unwrap().closing,
            "the level is not on its way out"
        );
        game.levels
            .parked
            .get(&Dim::Nether)
            .unwrap()
            .save
            .flush()
            .unwrap();
        // The pass has the level in its drop list now, and another level's save hook writes to
        // it through its parked world before the pass reaches the drop.
        let changed = game
            .levels
            .parked_world_mut_of(Dim::Nether.id())
            .expect("the parked level");
        let layout = changed.shape.column();
        let mut chunk = Chunk::with_layout(0, 0, layout);
        chunk.set(1, 70, 2, gold.0);
        changed.chunks.insert((0, 0), Arc::new(chunk));
        changed.modified.insert((0, 0));
        assert!(
            !game.levels.parked.get(&Dim::Nether).unwrap().closing,
            "a write did not cancel the queued close"
        );
        // What the pass does with its listing, made by hand: the finish must not drop the
        // level whose close the write cancelled.
        game.finish_closing_parked_level(Dim::Nether).unwrap();
        assert!(
            game.levels.parked.contains_key(&Dim::Nether),
            "the level was dropped with a write of its own unsaved"
        );
        // Closed for good, the write is in the region file.
        game.close_parked_level(Dim::Nether).unwrap();
        let save = SaveDir::new(dimension_dir(&world, Dim::Nether.id()), LevelShape::NETHER);
        let read_back = save
            .load(0, 0, game.pack.assets.registry, world_biomes!(game))
            .expect("the closed level's chunk is not in the region file");
        assert_eq!(
            read_back.block(1, 70, 2),
            gold,
            "the write that cancelled the close was not saved"
        );
    }

    /// The same, for a ticket rather than a chunk: a level a ticket came to is in use, and the
    /// pass that would drop it after its queued save must leave it open.
    #[test]
    #[ignore = "daily"]
    fn a_ticket_while_an_idle_close_waits_keeps_the_level() {
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let home = std::path::PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap())
            .join("idle-close-ticket");
        let _ = std::fs::remove_dir_all(&home);
        let world = home.join("saves/idle-close-ticket");
        let mut resources = None;
        let mut game = open_game(&mut resources, &root, &home, &world);
        game.enter_level(Dim::Nether).unwrap();
        game.enter_level(Dim::Overworld).unwrap();
        game.levels.parked_mut(Dim::Nether).unwrap().idle = IDLE_TICKS - 1;
        game.close_idle_levels().unwrap();
        assert!(
            game.levels.parked.get(&Dim::Nether).unwrap().closing,
            "the level is not on its way out"
        );
        // A forced chunk, as another level's save hook leaves one (the ticket alone: the load is
        // left to the streaming that owns it).
        assert!(
            game.levels
                .parked_streaming_of_mut(Dim::Nether.id())
                .expect("the parked level's streaming")
                .set_forced(0, 0, true),
            "the level's chunk is not forced"
        );
        game.levels
            .parked
            .get(&Dim::Nether)
            .unwrap()
            .save
            .flush()
            .unwrap();
        game.close_idle_levels().unwrap();
        assert!(
            game.levels.parked.contains_key(&Dim::Nether),
            "the level holding a ticket was dropped"
        );
        assert!(
            !game.levels.parked.get(&Dim::Nether).unwrap().closing,
            "the close is still queued for a level in use"
        );
    }

    struct LogCapture;
    static LOGGED: std::sync::Mutex<Vec<String>> = std::sync::Mutex::new(Vec::new());

    impl log::Log for LogCapture {
        fn enabled(&self, _: &log::Metadata) -> bool {
            true
        }
        fn log(&self, record: &log::Record) {
            LOGGED.lock().unwrap().push(record.args().to_string());
        }
        fn flush(&self) {}
    }

    /// 300 slow ticks in ten seconds log no Horizon slow-tick line, and the ring holds the last 256.
    #[test]
    #[ignore = "daily"]
    fn slow_ticks_enter_the_ring_and_not_the_log() {
        use crate::game::{Audio, Options, Progress, Settings, load_resources};
        use horizon_client::server_perf::{ServerPerf, TickDetail, WorldCounts};
        log::set_logger(&LogCapture).expect("no other logger in this test run");
        log::set_max_level(log::LevelFilter::Info);
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.clone()),
            Vec::new(),
            None,
            &root,
            &settings,
            Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.clone();
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        options.spikes = Some(0.0);
        let mut game = Game::new(
            &loaded.resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.clone(),
            )),
            None,
        )
        .unwrap();
        game.build_registries();
        let mut perf = ServerPerf::new(0.0, 0.0);
        for i in 0..300 {
            let mut detail = None;
            let started = Instant::now();
            game.tick_server_spans(false, |timing| {
                detail = Some(TickDetail {
                    cpu_ms: timing.cpu_ms,
                    spans: timing.raw,
                });
            })
            .unwrap();
            let wall_ms = started.elapsed().as_secs_f32() * 1000.0;
            perf.tick(
                f64::from(i) / 30.0,
                wall_ms,
                detail,
                WorldCounts::default,
                Vec::new,
            );
        }
        let kept: Vec<u64> = perf.slow_records().map(|r| r.counter).collect();
        assert_eq!(kept, (45..=300).collect::<Vec<u64>>());
        let logged = LOGGED.lock().unwrap();
        assert!(
            logged.iter().all(|line| !line.contains("slow tick")),
            "{logged:?}"
        );
    }

    /// A game on a scratch world folder, the way `dedicated.rs` opens one
    /// (`load_world` with an absolute folder).
    fn open_game<'a>(
        resources: &'a mut Option<std::sync::Arc<crate::game::Resources>>,
        root: &std::path::Path,
        home: &std::path::Path,
        world: &std::path::Path,
    ) -> Game<'a> {
        use crate::game::{Audio, Options, Progress, Settings, load_resources};
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
            Vec::new(),
            None,
            root,
            &settings,
            Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        // The game keeps a reference to its resources for the whole test.
        let resources = resources.insert(loaded.resources);
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.to_path_buf();
        options.game_dir = Some(home.to_path_buf());
        // A fixed name: no generated horizon-player.txt in the working directory.
        options.player_name = Some("H260".into());
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.to_path_buf(),
            )),
            None,
        )
        .unwrap();
        game.build_registries();
        game.set_allow_commands(true);
        let folder = world.to_string_lossy().into_owned();
        game.load_world(
            crate::game::world_loading::WorldRequest::new(
                &folder,
                Some("h260"),
                Some(260),
                true,
                None,
                std::sync::Arc::new(crate::game::api_values::game_rule_registry()),
            )
            .unwrap(),
        )
        .unwrap();
        game
    }

    /// `RedstoneTorchBlock`'s `lit` at `p` in `world`.
    fn torch_lit(game: &Game<'_>, world: &World, p: [i32; 3]) -> bool {
        let behaviours = game.res.behaviours();
        behaviours
            .table
            .bool(world.block(p[0], p[1], p[2]), behaviours.props.lit)
    }

    /// `LevelChunkTicks.schedule` for the tick that turns the torch at `p` off,
    /// due at `trigger_tick` (`RedstoneTorchBlock`'s own is two ticks off, so
    /// the deadline is given before the torch is placed: a block keeps the
    /// first tick it holds).
    fn schedule_torch_tick(
        game: &mut Game<'_>,
        p: [i32; 3],
        trigger_tick: i64,
    ) -> horizon_core::ticks::ScheduleResult {
        use horizon_core::{position::BlockPos, ticks::TickPriority};
        let state = game
            .pack
            .assets
            .registry
            .default_state("minecraft:redstone_torch")
            .expect("a redstone torch");
        let block = game.res.behaviours().block(state);
        let now = game.world.game_time;
        game.block_ticks.schedule_block(
            now,
            BlockPos::new(p[0], p[1], p[2]),
            block,
            (trigger_tick - now) as i32,
            TickPriority::Normal,
        )
    }

    /// The game time the block tick at `p` in chunk `key` waits for.
    fn queued_deadline(game: &Game<'_>, key: (i32, i32), p: [i32; 3]) -> Option<i64> {
        let at = horizon_core::position::BlockPos::new(p[0], p[1], p[2]);
        game.block_ticks
            .blocks
            .chunk(horizon_core::position::ChunkPos::new(key.0, key.1))
            .and_then(|ticks| {
                ticks
                    .queued()
                    .iter()
                    .find(|t| t.pos == at)
                    .map(|t| t.trigger_tick)
            })
    }

    /// A chunk as the level's own folder holds it, read the way its workers read one (the tables
    /// that name `block_ticks` go in first, as `save::Persistent::load` does).
    fn load_saved_chunk(
        game: &Game<'_>,
        save: &horizon_client::save::SaveDir,
        key: (i32, i32),
    ) -> Option<horizon_client::chunk::Chunk> {
        let _ = save
            .tables
            .get_or_init(|| Arc::new(horizon_client::save::SaveTables::new(&game.pack.assets)));
        save.load(key.0, key.1, game.pack.assets.registry, world_biomes!(game))
    }

    /// The block ticks a saved chunk holds, with the delays it was written with.
    fn saved_block_ticks(
        game: &Game<'_>,
        save: &horizon_client::save::SaveDir,
        key: (i32, i32),
    ) -> Vec<(horizon_core::position::BlockPos, i32)> {
        load_saved_chunk(game, save, key)
            .and_then(|chunk| {
                chunk
                    .saved
                    .as_deref()
                    .map(|saved| saved.block_ticks.iter().map(|t| (t.pos, t.delay)).collect())
            })
            .unwrap_or_default()
    }

    /// A level closed for being idle (300 ticks of `close_idle_levels`) comes
    /// back with the scheduled block ticks its chunks were saved with, each at
    /// its own deadline: the tick already due when the level was saved runs on
    /// the first tick after the reopen, and the one still ahead runs at its own
    /// game time, not one shifted by the ticks the level spent closed.
    ///
    /// The chunk saves a tick as `triggerTick - gameTime`
    /// (`LevelChunk.getTicksForSerialization`) and `startTickingChunk` unpacks
    /// it at `gameTime + delay`, so the clock the level comes back at is what
    /// the deadlines are read against - every level shares the server's
    /// (`enter_level`).
    #[test]
    #[ignore = "daily"]
    fn a_closed_idle_levels_scheduled_ticks_come_back_at_their_deadlines() {
        use horizon_core::{position::ChunkPos, ticks::ScheduleResult};
        // Two standing torches on the power that turns them off: one tick is
        // already past when the level is saved, one is 400 ticks out.
        const DUE: [i32; 3] = [2, 64, 2];
        const AHEAD: [i32; 3] = [6, 64, 6];
        const TORCHES: [[i32; 3]; 2] = [DUE, AHEAD];
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let home =
            std::path::PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap()).join("h18238");
        let world = home.join("saves/h18238");
        // A new world each run: the last run's levels are long closed.
        let _ = std::fs::remove_dir_all(&home);
        let mut resources = None;
        let mut game = open_game(&mut resources, &root, &home, &world);
        game.enter_level(Dim::Nether).unwrap();
        // The tables a save writes `block_ticks` and the heightmaps with are the
        // streaming's: its chunk source (`save::Persistent::load`) installs them
        // on the first saved chunk it decodes, which the hand-made chunk below
        // stands in for.
        let _ = game
            .save
            .tables
            .get_or_init(|| Arc::new(horizon_client::save::SaveTables::new(&game.pack.assets)));
        // Its chunk (0, 0), made resident by hand as the streaming does, with
        // the tick container the scheduler needs before it takes a tick.
        let key = (0, 0);
        let mut chunk = horizon_client::chunk::Chunk::empty(key.0, key.1);
        chunk.lit = true;
        game.world.chunks.insert(key, Arc::new(chunk));
        let _ = game
            .block_ticks
            .add_empty_chunk(ChunkPos::new(key.0, key.1));
        for torch in TORCHES {
            // A redstone block under a standing torch is the signal that makes
            // its tick turn it off.
            assert!(game.run_command(&format!(
                "setblock {} {} {} minecraft:redstone_block",
                torch[0],
                torch[1] - 1,
                torch[2]
            )));
        }
        // The deadlines go in first: placing a lit torch on a signal schedules
        // the torch's own tick two ticks out, and a block keeps the first tick
        // it is given.
        let due = game.world.game_time + 40;
        let ahead = due + 360;
        assert_eq!(
            schedule_torch_tick(&mut game, DUE, due),
            ScheduleResult::Added
        );
        assert_eq!(
            schedule_torch_tick(&mut game, AHEAD, ahead),
            ScheduleResult::Added
        );
        for torch in TORCHES {
            assert!(game.run_command(&format!(
                "setblock {} {} {} minecraft:redstone_torch",
                torch[0], torch[1], torch[2]
            )));
            assert!(torch_lit(&game, &game.world, torch), "lit when placed");
        }
        assert_eq!(
            queued_deadline(&game, key, DUE),
            Some(due),
            "the torch kept the deadline it was given"
        );
        assert_eq!(queued_deadline(&game, key, AHEAD), Some(ahead));
        // The save writes a chunk it was told changed: these are unsaved.
        assert!(game.world.modified.contains(&key), "the chunk is unsaved");
        // Back in the Overworld: the Nether is parked with both ticks.
        let left = game.world.game_time;
        game.enter_level(Dim::Overworld).unwrap();
        // `close_idle_levels` closes a level left alone for 300 ticks
        // (`IDLE_TICKS`): the count is written out here, as the expectation it
        // is for the test.
        const LEFT_ALONE: i64 = 300;
        for tick in 0..LEFT_ALONE {
            game.tick_server(false).unwrap();
            if tick == LEFT_ALONE / 2 {
                let nether = game
                    .levels
                    .parked
                    .get(&Dim::Nether)
                    .expect("parked until it is idle");
                assert!(
                    torch_lit(&game, &nether.world, DUE),
                    "a parked level is not ticked: the due tick waits"
                );
            }
        }
        let closed_at = game.world.game_time;
        assert_eq!(closed_at, left + LEFT_ALONE, "a tick per idle tick");
        assert!(
            !game.levels.parked.contains_key(&Dim::Nether),
            "closed for being idle"
        );
        // What the close wrote, read from the level's own folder before the
        // level is opened again: the levels' deadlines are these.
        let nether_save = horizon_client::save::SaveDir::new(
            horizon_client::save::dimension_dir(&game.world_root, Dim::Nether.id()),
            game.level_shapes.get(Dim::Nether.id()),
        );
        let written = saved_block_ticks(&game, &nether_save, key);
        // The reopen: the level is built again from its save, and its chunk is
        // read back the way the level's workers read one.
        game.enter_level(Dim::Nether).unwrap();
        let reopened_at = game.world.game_time;
        let mut chunk = load_saved_chunk(&game, &game.save, key).expect("the closed level's chunk");
        let saved = saved_block_ticks(&game, &game.save, key);
        let delay_of = |p: [i32; 3]| {
            let at = horizon_core::position::BlockPos::new(p[0], p[1], p[2]);
            saved.iter().find(|(pos, _)| *pos == at).map(|(_, d)| *d)
        };
        assert!(
            delay_of(DUE).is_some() && delay_of(AHEAD).is_some(),
            "the closed level kept both ticks: written {written:?}, read back {saved:?}, \
             closed at {closed_at}, reopened at {reopened_at}"
        );
        // Resident again, with the neighbours a ticking chunk needs, and the
        // ticket that makes the level tick it.
        chunk.lit = true;
        for dx in -1..=1 {
            for dz in -1..=1 {
                if (dx, dz) != (0, 0) {
                    let at = (key.0 + dx, key.1 + dz);
                    game.world.chunks.insert(
                        at,
                        Arc::new(horizon_client::chunk::Chunk::empty(at.0, at.1)),
                    );
                }
            }
        }
        game.world.chunks.insert(key, Arc::new(chunk));
        assert!(
            game.streaming.set_forced(key.0, key.1, true),
            "the level's chunk loads again"
        );
        // The first tick back: the due tick's deadline is behind the clock the
        // level came back at, so it runs here and not on a later tick.
        game.tick_server(false).unwrap();
        assert!(
            !torch_lit(&game, &game.world, DUE),
            "the due tick ran on the first tick back: clock {}, its deadline {due}, closed at \
             {closed_at}, reopened at {reopened_at}, written {written:?}, read back {saved:?}",
            game.world.game_time
        );
        assert!(
            torch_lit(&game, &game.world, AHEAD),
            "the tick ahead waits for its own game time: clock {}, its deadline {ahead}, closed \
             at {closed_at}, reopened at {reopened_at}, written {written:?}, read back {saved:?}",
            game.world.game_time
        );
        // It runs at the game time it was due at, however many ticks the level
        // spent closed.
        while game.world.game_time < ahead {
            game.tick_server(false).unwrap();
            if game.world.game_time < ahead {
                assert!(
                    torch_lit(&game, &game.world, AHEAD),
                    "not before its deadline: {} < {ahead}, reopened at {reopened_at}, written \
                     {written:?}, read back {saved:?}",
                    game.world.game_time
                );
            }
        }
        assert_eq!(game.world.game_time, ahead, "the deadline's own game time");
        assert!(
            !torch_lit(&game, &game.world, AHEAD),
            "the tick ahead ran at its deadline: written {written:?}, read back {saved:?}"
        );
    }

    /// Expected values from the official 26.3 dedicated server restarted on the
    /// same world: each level's `world_border` SavedData is read when the level
    /// is made, so the Overworld's saved border is in place for the first
    /// command, and a first `/worldborder set` is not overwritten later.
    #[test]
    #[ignore = "daily"]
    fn a_reopened_world_has_each_levels_saved_border_from_the_start() {
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let home = std::path::PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap()).join("h260");
        let world = home.join("saves/h260");
        // A new world each run: a border saved by the last one would already be 50.
        let _ = std::fs::remove_dir_all(&home);
        {
            let mut resources = None;
            let mut game = open_game(&mut resources, &root, &home, &world);
            // A server ticks before it saves.
            game.tick_world(true).unwrap();
            assert!(game.run_command("worldborder set 50"));
            assert!(game.run_command("execute in minecraft:the_nether run worldborder set 100"));
            game.save_game().unwrap();
        }
        let mut resources = None;
        let mut game = open_game(&mut resources, &root, &home, &world);
        let mut failures = Vec::new();
        if game.border.size() != 50.0 {
            failures.push(format!("in place after the open: {}", game.border.size()));
        }
        assert!(game.run_command("worldborder set 70"));
        game.tick_world(true).unwrap();
        assert!(game.run_command("execute in minecraft:the_nether run worldborder get"));
        assert!(game.run_command("execute in minecraft:the_end run worldborder get"));
        assert_eq!(game.dim, Dim::Overworld);
        let sizes = (
            game.border.size(),
            game.levels.parked_border(Dim::Nether).map(|b| b.size()),
            game.levels.parked_border(Dim::End).map(|b| b.size()),
        );
        if sizes != (70.0, Some(100.0), Some(59_999_968.0)) {
            failures.push(format!("after the first set and a tick: {sizes:?}"));
        }
        assert!(failures.is_empty(), "reopened borders: {failures:#?}");
    }

    /// Added controls (H260, after the baseline): a new world, and a read and a
    /// set before the first tick that survive a tick, a save and another reopen.
    #[test]
    #[ignore = "daily"]
    fn reopened_borders_keep_through_a_tick_and_a_save_and_a_new_world_has_defaults() {
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let home = std::path::PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap())
            .join("h260-controls");
        // A new world each run, as the first open expects.
        let _ = std::fs::remove_dir_all(&home);
        let world = home.join("saves/h260");
        let sizes = |game: &mut Game<'_>| {
            assert!(game.run_command("execute in minecraft:overworld run worldborder get"));
            assert!(game.run_command("execute in minecraft:the_nether run worldborder get"));
            assert!(game.run_command("execute in minecraft:the_end run worldborder get"));
            let parked = |d| game.levels.parked_border(d).map(|b| b.size());
            let size = |d: Dim| {
                if game.dim == d {
                    Some(game.border.size())
                } else {
                    parked(d)
                }
            };
            (size(Dim::Overworld), size(Dim::Nether), size(Dim::End))
        };
        const DEFAULT: f64 = 59_999_968.0;
        let mut failures = Vec::new();
        {
            let mut resources = None;
            let mut game = open_game(&mut resources, &root, &home, &world);
            // A new world: every level at the default.
            if game.border.size() != DEFAULT {
                failures.push(format!("new world in place: {}", game.border.size()));
            }
            game.tick_world(true).unwrap();
            let got = sizes(&mut game);
            if got != (Some(DEFAULT), Some(DEFAULT), Some(DEFAULT)) {
                failures.push(format!("new world: {got:?}"));
            }
            assert!(game.run_command("worldborder set 50"));
            assert!(game.run_command("execute in minecraft:the_nether run worldborder set 100"));
            game.save_game().unwrap();
        }
        {
            let mut resources = None;
            let mut game = open_game(&mut resources, &root, &home, &world);
            // Before the first tick: the read, then the set.
            if game.border.size() != 50.0 {
                failures.push(format!("first read before a tick: {}", game.border.size()));
            }
            assert!(game.run_command("worldborder set 70"));
            if game.border.size() != 70.0 {
                failures.push(format!("first set before a tick: {}", game.border.size()));
            }
            game.tick_world(true).unwrap();
            let got = sizes(&mut game);
            if got != (Some(70.0), Some(100.0), Some(DEFAULT)) {
                failures.push(format!("after a tick: {got:?}"));
            }
            game.save_game().unwrap();
        }
        {
            let mut resources = None;
            let mut game = open_game(&mut resources, &root, &home, &world);
            let got = sizes(&mut game);
            if got != (Some(70.0), Some(100.0), Some(DEFAULT)) {
                failures.push(format!("after the save and another reopen: {got:?}"));
            }
        }
        assert!(failures.is_empty(), "reopened borders: {failures:#?}");
    }

    /// `ImposterProtoChunk(…, false)`: a new chunk's features read a full
    /// neighbour but their ordinary `setBlockState` writes do not change it,
    /// while `BulkSectionAccess` (ores) writes its sections. Expected values
    /// from the official 26.3 server on the constructed 1.20.4 world
    /// (`H077_OLD_WORLD`: full old chunks, their partial ring removed), where
    /// chunks 13..14 at z 0 generate next to the full chunk (12, -1).
    #[test]
    #[ignore = "needs HORIZON_TEST_RESOURCES, a scratch HORIZON_HOME and the constructed H077_OLD_WORLD"]
    fn features_write_a_full_neighbour_only_through_its_sections() {
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let home = std::path::PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap()).join("h078");
        let world = home.join("saves/h078");
        let source = std::path::PathBuf::from(std::env::var_os("H077_OLD_WORLD").unwrap());
        fn copy(from: &std::path::Path, to: &std::path::Path) {
            std::fs::create_dir_all(to).unwrap();
            for entry in std::fs::read_dir(from).unwrap().flatten() {
                let target = to.join(entry.file_name());
                if entry.file_type().unwrap().is_dir() {
                    copy(&entry.path(), &target);
                } else if entry.file_name() != "session.lock" {
                    std::fs::copy(entry.path(), target).unwrap();
                }
            }
        }
        copy(&source, &world);
        let mut resources = None;
        let mut game = open_game(&mut resources, &root, &home, &world);
        // The world's level.dat has cheats off: the open read it after open_game's setting.
        game.set_allow_commands(true);
        assert!(game.run_command("forceload add 208 0 239 15"));
        let mut loaded = false;
        // As the dedicated server's loop: a tick, then the chunk streaming.
        for _ in 0..2000 {
            game.tick_world(true).unwrap();
            game.update_world();
            std::thread::sleep(std::time::Duration::from_millis(10));
            if game.run_command_quietly("execute if loaded 215 64 8") {
                loaded = true;
                break;
            }
        }
        assert!(loaded, "the new chunk (13, 0) was not generated");
        game.save_game().unwrap();
        let mut failures = Vec::new();
        for (p, want) in [
            // A spruce tree and a mushroom of the new chunks: ordinary writes.
            ([207, 68, -10], "minecraft:air"),
            ([207, 69, -6], "minecraft:air"),
            ([202, 69, -5], "minecraft:air"),
            // A gold ore blob: section writes.
            ([207, -32, -12], "minecraft:deepslate_gold_ore"),
        ] {
            let got = game.block_name(p);
            if &*got != want {
                failures.push(format!("{p:?}: {got}, expected {want}"));
            }
        }
        assert!(
            failures.is_empty(),
            "the full chunk (12, -1): {failures:#?}"
        );
    }

    /// `BlendingData.pack` of an old chunk whose border columns a new chunk's
    /// generation calculated (`getOrUpdateBlendingData`), as the official 26.3
    /// server saves it: a constructed 1.20.4 world (full old chunks, their
    /// partial ring removed, `H077_OLD_WORLD`) where chunk (13, 0) is new.
    #[test]
    #[ignore = "needs HORIZON_TEST_RESOURCES, a scratch HORIZON_HOME and the constructed H077_OLD_WORLD"]
    fn an_old_chunk_saves_the_heights_its_new_neighbour_calculated() {
        use horizon_core::nbt::Tag;
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let home = std::path::PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap()).join("h077");
        let world = home.join("saves/h077");
        let source = std::path::PathBuf::from(std::env::var_os("H077_OLD_WORLD").unwrap());
        fn copy(from: &std::path::Path, to: &std::path::Path) {
            std::fs::create_dir_all(to).unwrap();
            for entry in std::fs::read_dir(from).unwrap().flatten() {
                let target = to.join(entry.file_name());
                if entry.file_type().unwrap().is_dir() {
                    copy(&entry.path(), &target);
                } else if entry.file_name() != "session.lock" {
                    std::fs::copy(entry.path(), target).unwrap();
                }
            }
        }
        copy(&source, &world);
        {
            let mut resources = None;
            let mut game = open_game(&mut resources, &root, &home, &world);
            // The world's level.dat has cheats off: the open read it after open_game's setting.
            game.set_allow_commands(true);
            assert!(game.run_command("forceload add 208 0 239 15"));
            let mut loaded = false;
            // As the dedicated server's loop: a tick, then the chunk streaming.
            for _ in 0..2000 {
                game.tick_world(true).unwrap();
                game.update_world();
                std::thread::sleep(std::time::Duration::from_millis(10));
                if game.run_command_quietly("execute if loaded 215 64 8") {
                    loaded = true;
                    break;
                }
            }
            assert!(loaded, "the new chunk (13, 0) was not generated");
            game.save_game().unwrap();
        }
        let save = horizon_client::save::SaveDir::new(
            horizon_client::save::dimension_dir(&world, "minecraft:overworld"),
            horizon_client::chunk::LevelShape::OVERWORLD,
        );
        let blending = |x, z| {
            save.raw_chunk(x, z)
                .and_then(|r| r.tag.get("blending_data").cloned())
        };
        let heights = |tag: &Tag| match tag.get("heights") {
            Some(Tag::List(list)) => Some(
                list.iter()
                    .map(|v| match v {
                        Tag::Float(bits) => *bits,
                        other => panic!("height {other:?}"),
                    })
                    .collect::<Vec<u32>>(),
            ),
            _ => None,
        };
        let sections = |tag: &Tag| {
            (
                tag.get("min_section").cloned(),
                tag.get("max_section").cloned(),
            )
        };
        let mut failures = Vec::new();
        let mut want = vec![f32::MAX.to_bits(); 12];
        want.extend([70.0f32, 70.0, 69.0, 68.0].map(f32::to_bits));
        match blending(12, 0) {
            Some(tag) => {
                if heights(&tag) != Some(want.clone()) {
                    failures.push(format!("(12, 0) heights {:?}", heights(&tag)));
                }
                if sections(&tag) != (Some(Tag::Int(-4)), Some(Tag::Int(20))) {
                    failures.push(format!("(12, 0) sections {:?}", sections(&tag)));
                }
            }
            None => failures.push("(12, 0) has no blending_data".into()),
        }
        // No calculation reached these: the compound stays, without heights.
        for (x, z) in [(11, 0), (0, 0)] {
            match blending(x, z) {
                Some(tag) => {
                    if heights(&tag).is_some()
                        || sections(&tag) != (Some(Tag::Int(-4)), Some(Tag::Int(20)))
                    {
                        failures.push(format!("({x}, {z}) {tag:?}"));
                    }
                }
                None => failures.push(format!("({x}, {z}) has no blending_data")),
            }
        }
        assert!(failures.is_empty(), "saved blending data: {failures:#?}");
    }

    /// Added control (H077, after the baseline): the exact cache state the save
    /// reads. After the generation of (13, 0), the Overworld folder's lookup
    /// holds the calculated data of the edge chunk (12, 0), with the official
    /// server's heights; an old neighbour without new sides holds no height,
    /// and a far chunk no entry.
    #[test]
    #[ignore = "needs HORIZON_TEST_RESOURCES, a scratch HORIZON_HOME and the constructed H077_OLD_WORLD"]
    fn the_overworld_cache_holds_the_edge_chunks_calculated_heights() {
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let home =
            std::path::PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap()).join("h077-cache");
        let world = home.join("saves/h077");
        let source = std::path::PathBuf::from(std::env::var_os("H077_OLD_WORLD").unwrap());
        fn copy(from: &std::path::Path, to: &std::path::Path) {
            std::fs::create_dir_all(to).unwrap();
            for entry in std::fs::read_dir(from).unwrap().flatten() {
                let target = to.join(entry.file_name());
                if entry.file_type().unwrap().is_dir() {
                    copy(&entry.path(), &target);
                } else if entry.file_name() != "session.lock" {
                    std::fs::copy(entry.path(), target).unwrap();
                }
            }
        }
        copy(&source, &world);
        let mut resources = None;
        let mut game = open_game(&mut resources, &root, &home, &world);
        game.set_allow_commands(true);
        assert!(game.run_command("forceload add 208 0 239 15"));
        let mut loaded = false;
        for _ in 0..2000 {
            game.tick_world(true).unwrap();
            game.update_world();
            std::thread::sleep(std::time::Duration::from_millis(10));
            if game.run_command_quietly("execute if loaded 215 64 8") {
                loaded = true;
                break;
            }
        }
        assert!(loaded, "the new chunk (13, 0) was not generated");
        assert_eq!(game.dim, Dim::Overworld);
        let mut want = vec![f32::MAX.to_bits(); 12];
        want.extend([70.0f32, 70.0, 69.0, 68.0].map(f32::to_bits));
        let edge = game
            .save
            .calculated_blending(12, 0)
            .expect("(12, 0) was calculated");
        let (min, max, heights) = edge.pack();
        assert_eq!((min, max), (-4, 20));
        assert_eq!(heights.map(|h| h.map(f32::to_bits).to_vec()), Some(want));
        // (11, 0) borders no new chunk: the generation's calculation adds no height
        // to it (it may have an entry), so it packs without heights, as vanilla's.
        let inner = game.save.calculated_blending(11, 0);
        assert!(
            inner.as_ref().is_none_or(|data| data.pack().2.is_none()),
            "{inner:?}"
        );
        // (0, 0) is far from the new chunks: nothing calculated it.
        assert!(game.save.calculated_blending(0, 0).is_none());
    }
}

#[cfg(test)]
mod request_save_tests {
    use super::{Dim, Levels};

    #[test]
    #[ignore = "daily"]
    fn requested_save_waits_without_consuming_the_queue_while_lifted() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.levels.request_save(Dim::Overworld);
            let pass = game.dropped.lift();
            game.run_requested_saves();
            assert_eq!(game.levels.requested_saves, [Dim::Overworld]);
            drop(pass);
            game.run_requested_saves();
            assert!(game.levels.requested_saves.is_empty());
        });
    }

    /// A level asked for twice is queued once, and clearing the levels drops the queue.
    #[test]
    fn a_level_asked_for_twice_is_saved_once() {
        let mut levels = Levels::default();
        levels.request_save(Dim::Nether);
        levels.request_save(Dim::Nether);
        levels.request_save(Dim::End);
        assert_eq!(levels.requested_saves, [Dim::Nether, Dim::End]);
        levels.clear();
        assert!(levels.requested_saves.is_empty());
    }
}
