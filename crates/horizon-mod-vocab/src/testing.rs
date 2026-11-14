//! Test support, enabled by the `testing` feature: scoped trait-mod installation
//! ([`with_dispatch`]), and [`FakeHost`], a host built from small fakes so that a mod's tests run
//! without starting the game.
//!
//! ```
//! use mod_api::testing::FakeHost;
//! use mod_api::prelude::ItemsHost;
//! use mod_api::{HostTransaction, ITEMS, ItemResource, ItemStack, ServerHost, move_resources};
//!
//! let mut host = FakeHost::new();
//! let level = host.level.level();
//! host.level.block_entity([0, 64, 0]).with_items(1);
//! host.level.block_entity([1, 64, 0]).with_items(1);
//! let stone_id = mod_api::Identifier::parse("minecraft:stone").unwrap();
//! host.registries.resolve_items([stone_id.clone()]).unwrap();
//! let stone = ItemStack::new(host.item_key(&stone_id).unwrap(), 5).unwrap();
//! host.level.block_entity([0, 64, 0]).items.as_mut().unwrap().slots[0] = Some(stone);
//!
//! let server: &mut dyn ServerHost = &mut host;
//! let from = level.capability(server, &ITEMS, [0, 64, 0], None).unwrap();
//! let into = level.capability(server, &ITEMS, [1, 64, 0], None).unwrap();
//! let moved = server.transaction(|host, tx| {
//!     move_resources(host, &from, &into, |_: &ItemResource| true, 3, tx)
//! });
//! assert_eq!(moved.ok(), Some(3));
//! let into = host.level.block_entity_at([1, 64, 0]).unwrap().items.as_ref().unwrap();
//! assert_eq!(into.count("minecraft:stone"), 3);
//! ```
#[path = "testing/journal.rs"]
mod journal;
#[path = "testing/level.rs"]
mod level;
#[path = "testing/players.rs"]
mod players;
#[path = "testing/registries.rs"]
mod registries;
#[path = "testing/services.rs"]
mod services;

/// An item forward reference missing from a fake's load-owned catalog.
pub use crate::parts::ItemKeyError;
pub use journal::FakeJournal;
pub use level::{
    FakeBattery, FakeBlockEntity, FakeContainer, FakeEntity, FakeGameEvent, FakeLevel, FakeSound,
    FakeTank,
};
pub use players::{FakePlayer, FakePlayers};
pub use registries::FakeRegistries;
pub use services::{FakeCommands, FakeMessages};

use crate::item::transaction::{Transaction, TransactionSeal};
use crate::runtime::Installed;
use crate::world::ticket_controller::{ControllerInfo, TEST_CONTROLLERS};
use crate::{
    Backend, BackendMut, BackendMutRef, BackendRef, BlockEntityRef, BlockEntityTypeKey, BlockSide,
    ClientHost, Commands, EntityRef, FluidStack, Host, ItemPlace, ItemStack, LevelRef, Messages,
    Mods, PlayerRef, Players, ServerHost, TEST_MODS, Tag, Text, TicketController, WorldEpoch,
};

/// The world epoch of every fake's handles.
const FAKE_EPOCH: u64 = 1;

/// Run a closure with these owned mods on this thread, restoring the previous list on exit.
///
/// The test allocation is leaked because dispatch exposes a static slice. Nested scopes and
/// panic unwinding restore the preceding list; other threads are unaffected.
// hidden: API-owned test setup uses this vocabulary fixture, not mod authoring code.
#[doc(hidden)]
pub fn with_dispatch<R>(mods: Mods, f: impl FnOnce() -> R) -> R {
    struct Restore(Option<&'static Installed>);
    impl Drop for Restore {
        fn drop(&mut self) {
            TEST_MODS.set(self.0);
        }
    }
    let installed = Box::leak(Box::new(Installed::new(mods)));
    let _restore = Restore(TEST_MODS.replace(Some(installed)));
    f()
}

/// Rebuild a fixture's Parts snapshot from a fresh registration epoch on this thread.
///
/// On success the replacement stays installed. An error or unwinding restores the previous snapshot.
/// Production registries remain frozen; only game fixtures that need a complete key table
/// after their initial synthetic registration should use this.
// hidden: only the shared game fixtures in horizon-mod-api rebuild their Parts epoch; not API.
#[doc(hidden)]
pub fn rebuild_parts<R, E>(f: impl FnOnce() -> Result<R, E>) -> Result<R, E> {
    struct Restore {
        previous: Option<&'static crate::parts::Installed>,
        committed: bool,
    }
    impl Drop for Restore {
        fn drop(&mut self) {
            if !self.committed {
                crate::parts::TEST_PARTS.set(self.previous);
            }
        }
    }
    let mut restore = Restore {
        previous: crate::parts::TEST_PARTS.replace(None),
        committed: false,
    };
    let result = f();
    restore.committed = result.is_ok();
    result
}

#[cfg(test)]
#[test]
fn rebuilding_fixture_parts_commits_success_and_restores_on_error_or_unwind() {
    use crate::parts::{custom_stat_declarations, install_input, installation::InstalledInput};
    let install = |id: &str| {
        install_input(InstalledInput {
            custom_stats: vec![id.into()],
            ..Default::default()
        });
    };
    install("fixture:old");
    let result: Result<(), ()> = rebuild_parts(|| {
        assert!(custom_stat_declarations().is_empty());
        install("fixture:new");
        Ok(())
    });
    assert_eq!(result, Ok(()));
    assert_eq!(custom_stat_declarations(), ["fixture:new"]);
    let result: Result<(), &str> = rebuild_parts(|| {
        install("fixture:failed");
        Err("fixture refused")
    });
    assert_eq!(result, Err("fixture refused"));
    assert_eq!(custom_stat_declarations(), ["fixture:new"]);
    let panic = std::panic::catch_unwind(|| {
        let _: Result<(), ()> = rebuild_parts(|| {
            install("fixture:panicked");
            panic!("fixture refused");
        });
    });
    assert!(panic.is_err());
    assert_eq!(custom_stat_declarations(), ["fixture:new"]);
}

/// Capture this thread's test installation for work sent to another thread.
///
/// Production installations are process-wide. Tests isolate them per thread; a worker
/// using an owning thread's immutable resource graph must see that same installation.
pub fn inherit_mods<A, R, F: FnOnce(&A) -> R + Send>(f: F) -> impl FnOnce(&A) -> R + Send {
    let installed = crate::runtime::installed();
    let parts = crate::parts::installed();
    move |argument| {
        struct Restore(
            Option<&'static Installed>,
            Option<&'static crate::parts::Installed>,
        );
        impl Drop for Restore {
            fn drop(&mut self) {
                TEST_MODS.set(self.0);
                crate::parts::TEST_PARTS.set(self.1);
            }
        }
        let _restore = Restore(
            TEST_MODS.replace(installed),
            crate::parts::TEST_PARTS.replace(parts),
        );
        f(argument)
    }
}

/// The ticket controller with this id that the mods of the enclosing `with_mods` registered.
pub fn ticket_controller(id: &str) -> Option<TicketController> {
    TicketController::from_id(id)
}

struct RestoreControllers(Option<&'static [ControllerInfo]>);
impl Drop for RestoreControllers {
    fn drop(&mut self) {
        TEST_CONTROLLERS.set(self.0);
    }
}

// no wire: in-process testing harness state, not a mod boundary payload.
/// A server and client host for tests, composed of one fake per service: the registries, the
/// players, one level, the commands and messages services, and the transaction journal, which
/// keeps the players and the level on each transaction and puts them back when it aborts.
///
/// Set a test's world up through the fakes' fields, run the code under test with
/// `&mut host` as its `&mut dyn ServerHost`, then read the fakes back. A host method no fake
/// answers panics, naming the method, as a test double's would. The host has no client
/// backend, so client handle writes refuse.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct FakeHost {
    /// The registries.
    pub registries: FakeRegistries,
    /// The players online.
    pub players: FakePlayers,
    /// The one level.
    pub level: FakeLevel,
    /// The command service.
    pub commands: FakeCommands,
    /// The message service.
    pub messages: FakeMessages,
    /// The transaction journal over the players, the level and the holders' attachments.
    pub journal: FakeJournal<(
        FakePlayers,
        FakeLevel,
        horizon_mod_runtime::attachments::Attachments,
    )>,
    /// The sounds played in the level, in order. Like the game's, they are sent as they are
    /// played: an aborted transaction does not take them back.
    pub sounds: Vec<FakeSound>,
    /// The game events sent in the level, in order, kept as the sounds are.
    pub game_events: Vec<FakeGameEvent>,
    /// The holders' attachments: a lone map answers for any holder.
    pub attachments: horizon_mod_runtime::attachments::Attachments,
}

impl FakeHost {
    /// A host with an empty overworld, no players and no mods loaded.
    pub fn new() -> Self {
        Self::default()
    }
    /// Freeze the current load's entity-type table after declaring the fake world's entities.
    /// Includes requested mod keys, the player type and every kind in this host's level.
    /// Repeated setup may use already resolved kinds, but cannot extend the frozen table.
    pub fn resolve_entity_types(&mut self) -> Result<(), Vec<String>> {
        if crate::parts::installed().is_none() {
            crate::parts::install_input(
                crate::__private::parts_installation::InstalledInput::default(),
            );
        }
        let mut ids: Vec<_> = crate::parts::requested_keys()
            .entity_types
            .entries
            .iter()
            .map(|entry| entry.id.as_str())
            .collect();
        ids.push("minecraft:player");
        ids.extend(
            self.level
                .entities
                .values()
                .map(|entity| entity.kind.as_str()),
        );
        if crate::parts::installed()
            .is_some_and(|installed| installed.resolved_entity_types.get().is_some())
        {
            let errors: Vec<_> = ids
                .into_iter()
                .filter(|id| crate::content::registry_keys::resolved_entity_type_key(id).is_none())
                .map(|id| {
                    format!("fake entity kind {id} was not declared before entity-type resolution")
                })
                .collect();
            return if errors.is_empty() {
                Ok(())
            } else {
                Err(errors)
            };
        }
        crate::content::registry_keys::resolve_entity_types(ids)
    }
    /// Player `uuid` online as entity `entity` in the level, joined if missing.
    pub fn join(&mut self, uuid: u128, entity: u32) -> &mut FakePlayer {
        self.players.join(uuid, entity, self.level.level())
    }

    /// A queued bucket sound and game event, as the game plays them at the outermost commit.
    fn play_fluid_effects(&mut self, queued: level::QueuedFluidEffects) {
        if let Some(sound) = self.registries.fluid_sound(queued.fluid, queued.pickup) {
            self.sounds.push(FakeSound {
                sound: sound.to_owned(),
                at: queued.at,
                source: crate::SoundSource::Blocks,
                volume: 1.0,
                pitch: 1.0,
            });
        }
        self.game_events.push(FakeGameEvent {
            event: match queued.pickup {
                true => "minecraft:fluid_pickup",
                false => "minecraft:fluid_place",
            }
            .to_owned(),
            at: queued.at,
            context: crate::GameEventContext {
                source_entity: queued.player.map(PlayerRef::entity),
                affected_state: None,
            },
        });
    }
    fn max_stack(&self, resource: &crate::ItemResource) -> u32 {
        match resource.value_id(self, "minecraft:max_stack_size") {
            Some(Tag::Int(size)) if (1..=99).contains(&size) => size as u32,
            None => 1,
            _ => 0,
        }
    }
}
impl Host for FakeHost {
    fn backend(&self) -> Option<BackendRef<'_>> {
        Some(BackendRef::of(self))
    }
}
impl crate::prelude::BlockEntitiesHost for FakeHost {
    fn block_entity(&self, level: LevelRef, pos: [i32; 3]) -> Option<BlockEntityRef> {
        self.level.standing(level, pos)?;
        Some(BlockEntityRef {
            level,
            position: pos,
            generation: 0,
        })
    }
}
impl crate::prelude::EntitiesHost for FakeHost {
    fn level(&self, dimension: &str) -> Option<LevelRef> {
        let level = self.level.level();
        (crate::content::registry_keys::dimension_id(level.dimension()) == Some(dimension))
            .then_some(level)
    }
    fn entity(&self, id: u32) -> Option<EntityRef> {
        let known = self.players.by_entity(id).is_some() || self.level.entity_at(id).is_some();
        known.then(|| EntityRef::new(WorldEpoch::new(FAKE_EPOCH), id))
    }
}
impl crate::prelude::PlayersHost for FakeHost {
    fn player(&self, uuid: u128) -> Option<PlayerRef> {
        self.players.handle(uuid)
    }
}
impl crate::prelude::ItemsHost for FakeHost {
    fn item_key(&self, id: &crate::Identifier) -> Option<crate::ItemKey> {
        crate::content::registry_keys::item_key_by_identifier(id)
    }
    fn item_default_component(&self, item: crate::ItemKey, component: &str) -> Option<Tag> {
        self.registries
            .default_component(FakeRegistries::resolved_item_id(item)?, component)
    }
    fn max_stack_size(&self, item: crate::ItemKey) -> Option<u32> {
        Some(
            self.registries
                .max_stack_size(FakeRegistries::resolved_item_id(item)?),
        )
    }
    fn max_damage(&self, item: crate::ItemKey) -> Option<u32> {
        let id = FakeRegistries::resolved_item_id(item)?;
        match self
            .registries
            .default_component(id, "minecraft:max_damage")
        {
            Some(Tag::Int(damage)) => u32::try_from(damage).ok(),
            None => Some(0),
            _ => None,
        }
    }
}
impl crate::prelude::TagsHost for FakeHost {
    fn tags(&self) -> &dyn crate::Tags {
        &self.registries
    }
    fn fluid_pickup_sound(&self, fluid: &str) -> Option<&'static str> {
        self.registries.fluid_sound(fluid, true)
    }
    fn fluid_empty_sound(&self, fluid: &str) -> Option<&'static str> {
        self.registries.fluid_sound(fluid, false)
    }
}
impl crate::prelude::BlockStatesHost for FakeHost {
    fn fluid_key(&self, id: &str) -> Option<crate::FluidKey> {
        crate::content::registry_keys::fluid_key(id)
    }
}
impl crate::prelude::LevelsHost for FakeHost {
    fn is_loaded(&self, id: &str) -> bool {
        self.registries.is_loaded(id)
    }
}
crate::impl_host_areas!(
    FakeHost,
    except(
        block_entities,
        block_states,
        entities,
        items,
        levels,
        mod_state,
        players,
        tags
    )
);

impl ServerHost for FakeHost {
    fn backend_mut(&mut self) -> Option<BackendMutRef<'_>> {
        Some(BackendMutRef::of(self))
    }
}
impl crate::host::mod_state::ModStateHost for FakeHost {
    fn attachments(&mut self) -> &mut dyn mod_api::AttachmentStore {
        &mut self.attachments
    }
}
impl crate::prelude::ModStateServerHost for FakeHost {
    fn transaction_open(&mut self) -> TransactionSeal {
        self.journal.open((
            self.players.clone(),
            self.level.clone(),
            self.attachments.clone(),
        ))
    }
    fn transaction_close(&mut self, seal: &TransactionSeal, commit: bool) {
        if let Some((players, level, attachments)) = self.journal.close(seal, commit) {
            (self.players, self.level, self.attachments) = (players, level, attachments);
        }
        // The outermost commit plays what waited for it.
        if self.journal.depth() == 0 {
            for queued in std::mem::take(&mut self.level.fluid_effects) {
                self.play_fluid_effects(queued);
            }
        }
    }
    fn transaction_lifecycle(&self) -> crate::TransactionLifecycle {
        // The fake tells the world of nothing at a close, so no close is ever seen running.
        if self.journal.depth() == 0 {
            crate::TransactionLifecycle::None
        } else {
            crate::TransactionLifecycle::Open
        }
    }
    fn transaction_current(&self) -> Option<crate::Transaction> {
        self.journal.current()
    }
    fn transaction_depth(&self, tx: &crate::Transaction) -> Option<u32> {
        self.journal.depth_of(tx)
    }
}
impl crate::prelude::ServicesServerHost for FakeHost {
    fn commands(&mut self) -> &mut dyn Commands {
        &mut self.commands
    }
    fn messages(&self) -> &dyn Messages {
        &self.messages
    }
    fn players(&self) -> &dyn Players {
        &self.players
    }
}
crate::impl_server_host_areas!(FakeHost, except(mod_state, services));
impl ClientHost for FakeHost {}
crate::impl_client_host_areas!(FakeHost);

impl Backend for FakeHost {}
impl crate::prelude::LevelsBackend for FakeHost {
    fn level_in_place(&self, level: LevelRef) -> bool {
        level == self.level.level()
    }
    fn level_block(&self, level: LevelRef, pos: [i32; 3]) -> Option<crate::BlockKey> {
        self.level
            .block_at(level, pos)
            .and_then(crate::__private::block_key)
    }
    /// The named block's first state: the fake keeps no properties.
    fn level_block_state(&self, level: LevelRef, pos: [i32; 3]) -> Option<crate::BlockState> {
        let block = crate::prelude::LevelsBackend::level_block(self, level, pos)?;
        Some(crate::BlockStateRecord::new(block, 0).into())
    }
}
impl crate::prelude::BlockEntitiesBackend for FakeHost {
    fn block_entity_kind(&self, entity: BlockEntityRef) -> Option<BlockEntityTypeKey> {
        let standing = self.level.standing(entity.level, entity.position)?;
        standing.kind().cloned()
    }
}
impl crate::prelude::ItemsBackend for FakeHost {
    fn container_kind_amount(&self, at: BlockSide, resource: &crate::ItemResource) -> Option<u64> {
        let Some(items) = self
            .level
            .standing(at.level, at.pos)
            .and_then(|s| s.items.as_ref())
        else {
            return Some(0);
        };
        items
            .slots
            .iter()
            .flatten()
            .filter(|stack| resource.matches(stack))
            .try_fold(0u64, |total, stack| total.checked_add(stack.count() as u64))
    }
    fn container_kind_space(&self, at: BlockSide, resource: &crate::ItemResource) -> Option<u64> {
        let max_stack = self.max_stack(resource);
        let Some(items) = self
            .level
            .standing(at.level, at.pos)
            .and_then(|s| s.items.as_ref())
        else {
            return Some(0);
        };
        (0..items.slots.len()).try_fold(0u64, |total, slot| {
            let room = match &items.slots[slot] {
                Some(held) if resource.matches(held) => {
                    items.capacity(slot, max_stack).saturating_sub(held.count())
                }
                Some(_) => 0,
                None => items.capacity(slot, max_stack),
            };
            total.checked_add(room as u64)
        })
    }
    fn container_listing(
        &self,
        at: BlockSide,
        from: usize,
        limit: usize,
    ) -> Option<Vec<Option<ItemStack>>> {
        let items = self.level.standing(at.level, at.pos)?.items.as_ref()?;
        let limit = limit.min(crate::MAX_STORAGE_PAGE);
        Some(items.slots.iter().skip(from).take(limit).cloned().collect())
    }
    fn container_size(&self, at: BlockSide) -> Option<usize> {
        let items = self.level.standing(at.level, at.pos)?.items.as_ref()?;
        Some(items.slots.len())
    }
    fn container_stack(&self, at: BlockSide, slot: usize) -> Option<ItemStack> {
        let items = self.level.standing(at.level, at.pos)?.items.as_ref()?;
        items.slots.get(slot)?.clone()
    }
    fn container_capacity(&self, at: BlockSide, slot: usize, stack: &ItemStack) -> u32 {
        let items = self
            .level
            .standing(at.level, at.pos)
            .and_then(|standing| standing.items.as_ref());
        items.map_or(0, |items| {
            items.capacity(slot, self.max_stack(&crate::ItemResource::of(stack)))
        })
    }
    fn entity_container_size(&self, _: u32) -> Option<usize> {
        None
    }
    fn entity_automation_size(&self, _: u32) -> Option<usize> {
        None
    }
    fn entity_equipment_size(&self, _: u32) -> Option<usize> {
        None
    }
    fn item_bundle_slots(&self, _: &ItemStack) -> Option<Vec<Option<ItemStack>>> {
        // The fake registry models declared containers, not game bundle components.
        None
    }
    fn item_container_slots(&self, stack: &ItemStack) -> Option<Vec<Option<ItemStack>>> {
        self.registries.container_slots(stack)
    }
}
impl crate::prelude::FluidsEnergyBackend for FakeHost {
    fn fluid_tank_count(&self, at: BlockSide) -> Option<usize> {
        Some(self.level.standing(at.level, at.pos)?.tanks.len())
    }
    fn fluid_tank(&self, at: BlockSide, tank: usize) -> Option<FluidStack> {
        let standing = self.level.standing(at.level, at.pos)?;
        standing.tanks.get(tank)?.held.clone()
    }
    fn fluid_tank_capacity(&self, at: BlockSide, tank: usize, _: &FluidStack) -> u32 {
        let standing = self.level.standing(at.level, at.pos);
        standing
            .and_then(|standing| standing.tanks.get(tank))
            .map_or(0, |tank| tank.capacity)
    }
    fn energy_amount(&self, at: BlockSide) -> Option<u32> {
        let energy = self.level.standing(at.level, at.pos)?.energy.as_ref()?;
        Some(energy.amount)
    }
    fn energy_capacity(&self, at: BlockSide) -> u32 {
        let standing = self.level.standing(at.level, at.pos);
        standing
            .and_then(|standing| standing.energy.as_ref())
            .map_or(0, |energy| energy.capacity)
    }
}
impl crate::prelude::EntitiesBackend for FakeHost {
    fn entity_kind(&self, entity: EntityRef) -> Option<crate::EntityTypeKey> {
        let kind = if self.players.by_entity(entity.id()).is_some() {
            "minecraft:player"
        } else {
            &self.level.entity_at(entity.id())?.kind
        };
        crate::content::registry_keys::resolved_entity_type_key(kind)
    }
    fn entity_level(&self, entity: EntityRef) -> Option<LevelRef> {
        if let Some((_, player)) = self.players.by_entity(entity.id()) {
            return Some(player.level);
        }
        Some(self.level.entity_at(entity.id())?.level)
    }
}
impl crate::prelude::KeysBackend for FakeHost {
    fn entity_type_key_id(&self, key: crate::EntityTypeKey) -> String {
        crate::content::registry_keys::entity_type_key_id(key)
            .expect("fake entity type key belongs to its load")
    }

    fn item_key_id(&self, key: crate::ItemKey) -> String {
        crate::content::registry_keys::item_id(key)
            .expect("fake item key belongs to its load")
            .to_owned()
    }
}
impl crate::prelude::PlayersBackend for FakeHost {
    fn player_selected_slot(&self, player: PlayerRef) -> Option<usize> {
        Some(self.players.get(player.uuid())?.selected_slot)
    }
    fn player_name(&self, player: PlayerRef) -> Option<String> {
        Some(self.players.get(player.uuid())?.name.clone())
    }
    fn player_level(&self, player: PlayerRef) -> Option<LevelRef> {
        (self.players.handle(player.uuid())? == player).then_some(())?;
        Some(self.players.get(player.uuid())?.level)
    }
    fn player_dimension(&self, player: PlayerRef) -> Option<crate::Identifier> {
        let level = self.player_level(player)?;
        crate::Identifier::parse(level.dimension_id()).ok()
    }
    fn player_has_infinite_materials(&self, player: PlayerRef) -> Option<bool> {
        Some(self.players.get(player.uuid())?.infinite_materials)
    }
    fn player_inventory(&self, player: PlayerRef) -> Option<Vec<ItemStack>> {
        let slots = &self.players.get(player.uuid())?.slots;
        Some(slots.range(0..36).map(|(_, stack)| stack.clone()).collect())
    }
}
crate::impl_backend_areas!(
    FakeHost,
    except(
        block_entities,
        entities,
        fluids_energy,
        items,
        keys,
        levels,
        players
    )
);

impl BackendMut for FakeHost {}
impl crate::prelude::ItemsBackendMut for FakeHost {
    fn container_insert(
        &mut self,
        at: BlockSide,
        slot: usize,
        resource: &crate::ItemResource,
        amount: u32,
        _: &Transaction,
    ) -> Result<Option<ItemStack>, crate::StackWriteError> {
        let max_stack = self.max_stack(resource);
        if max_stack == 0 {
            return Err(crate::StackWriteError::InvalidMaximum);
        }
        let standing = self.level.standing_mut(at.level, at.pos);
        let put = match standing.and_then(|standing| standing.items.as_mut()) {
            Some(items) => items.insert(slot, resource, amount, max_stack),
            None => 0,
        };
        Ok(resource.to_stack(amount - put))
    }
    fn container_extract(
        &mut self,
        at: BlockSide,
        slot: usize,
        max: u32,
        _: &Transaction,
    ) -> Option<ItemStack> {
        let standing = self.level.standing_mut(at.level, at.pos)?;
        standing.items.as_mut()?.extract(slot, max)
    }
    fn container_insert_kind(
        &mut self,
        at: BlockSide,
        resource: &crate::ItemResource,
        amount: u64,
        tx: &Transaction,
    ) -> u64 {
        use crate::prelude::ItemsBackend;
        let size = self.container_size(at).unwrap_or(0);
        let mut left = amount;
        for holding in [true, false] {
            for slot in 0..size {
                if left == 0 {
                    break;
                }
                let held = self.container_stack(at, slot);
                let pick = match &held {
                    Some(held) => holding && resource.matches(held),
                    None => !holding,
                };
                if !pick {
                    continue;
                }
                let ask = left.min(u32::MAX as u64) as u32;
                let rest = self
                    .container_insert(at, slot, resource, ask, tx)
                    .ok()
                    .flatten()
                    .map_or(0, |rest| rest.count());
                left -= (ask - rest) as u64;
            }
        }
        amount - left
    }
    fn container_extract_kind(
        &mut self,
        at: BlockSide,
        resource: &crate::ItemResource,
        amount: u64,
        tx: &Transaction,
    ) -> u64 {
        use crate::prelude::ItemsBackend;
        let mut left = amount;
        for slot in 0..self.container_size(at).unwrap_or(0) {
            if left == 0 {
                break;
            }
            if self
                .container_stack(at, slot)
                .is_some_and(|held| resource.matches(&held))
            {
                let ask = left.min(u32::MAX as u64) as u32;
                left -= self
                    .container_extract(at, slot, ask, tx)
                    .map_or(0, |out| out.count()) as u64;
            }
        }
        amount - left
    }
    fn game_stack(&mut self, place: &ItemPlace) -> Option<ItemStack> {
        let (player, slot) = self.players.place(place)?;
        player.slots.get(&slot).cloned()
    }
    fn game_insert(
        &mut self,
        place: &ItemPlace,
        resource: &crate::ItemResource,
        amount: u32,
        _: &Transaction,
    ) -> Result<u32, crate::StackWriteError> {
        let max_stack = self.max_stack(resource);
        if max_stack == 0 {
            return Err(crate::StackWriteError::InvalidMaximum);
        }
        Ok(match self.players.place(place) {
            Some((player, slot)) => player.insert(slot, resource, amount, max_stack),
            _ => 0,
        })
    }
    fn game_extract(
        &mut self,
        place: &ItemPlace,
        resource: &crate::ItemResource,
        amount: u32,
        _: &Transaction,
    ) -> Result<u32, crate::StackWriteError> {
        Ok(self
            .players
            .place(place)
            .map_or(0, |(player, slot)| player.extract(slot, resource, amount)))
    }
}
impl crate::prelude::FluidsEnergyBackendMut for FakeHost {
    fn fluid_tank_insert(
        &mut self,
        at: BlockSide,
        tank: usize,
        fluid: FluidStack,
        _: &Transaction,
    ) -> Option<FluidStack> {
        let standing = self.level.standing_mut(at.level, at.pos);
        match standing.and_then(|standing| standing.tanks.get_mut(tank)) {
            Some(tank) => tank.insert(fluid),
            None => Some(fluid),
        }
    }
    fn fluid_tank_extract(
        &mut self,
        at: BlockSide,
        tank: usize,
        max: u32,
        _: &Transaction,
    ) -> Option<FluidStack> {
        let standing = self.level.standing_mut(at.level, at.pos)?;
        standing.tanks.get_mut(tank)?.extract(max)
    }
    fn energy_insert(&mut self, at: BlockSide, amount: u32, _: &Transaction) -> u32 {
        let standing = self.level.standing_mut(at.level, at.pos);
        standing
            .and_then(|standing| standing.energy.as_mut())
            .map_or(0, |energy| energy.insert(amount))
    }
    fn energy_extract(&mut self, at: BlockSide, amount: u32, _: &Transaction) -> u32 {
        let standing = self.level.standing_mut(at.level, at.pos);
        standing
            .and_then(|standing| standing.energy.as_mut())
            .map_or(0, |energy| energy.extract(amount))
    }
    /// The source block at `pos` ([`FakeLevel::set_block`]), a bucket of it; the fake plays no
    /// sound or game event for it.
    fn level_pickup_fluid(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        _: Option<PlayerRef>,
        tx: &Transaction,
    ) -> Option<FluidStack> {
        if !self.journal.holds(tx) {
            return None;
        }
        let fluid = self.level.take_source(level, pos)?;
        Some(FluidStack::new(fluid, 1000))
    }
    /// A source of `fluid` where nothing is; the fake plays no sound or game event for it.
    fn level_place_fluid(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        fluid: crate::FluidKey,
        _: Option<PlayerRef>,
        _: bool,
        tx: &Transaction,
    ) -> bool {
        self.journal.holds(tx) && self.level.put_source(level, pos, fluid)
    }
    /// Queued with the level for the outermost commit, which plays it as the game does.
    fn level_play_fluid_effects(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        fluid: crate::FluidKey,
        pickup: bool,
        player: Option<PlayerRef>,
        tx: &Transaction,
    ) -> bool {
        let Some(id) = crate::content::registry_keys::fluid_id(fluid) else {
            return false;
        };
        if !self.journal.holds(tx) || level != self.level.level() {
            return false;
        }
        self.level.fluid_effects.push(level::QueuedFluidEffects {
            at: pos.map(|c| f64::from(c) + 0.5),
            fluid: id,
            pickup,
            player,
        });
        true
    }
}
impl crate::prelude::EntitiesBackendMut for FakeHost {
    fn entity_uuid(&mut self, entity: EntityRef) -> Option<u128> {
        if let Some((uuid, _)) = self.players.by_entity(entity.id()) {
            return Some(uuid);
        }
        Some(self.level.entity_at(entity.id())?.uuid)
    }
}
impl crate::prelude::PlayersBackendMut for FakeHost {
    fn player_action_bar(
        &mut self,
        player: PlayerRef,
        line: Text,
    ) -> Result<bool, crate::text::TextFormError> {
        line.validate_depth()?;
        let Some(player) = self.players.get_mut(player.uuid()) else {
            return Ok(false);
        };
        player.action_bar.push(line);
        Ok(true)
    }
}
impl crate::prelude::LevelsBackendMut for FakeHost {
    fn level_play_sound(
        &mut self,
        level: LevelRef,
        sound: &str,
        at: [f64; 3],
        source: crate::SoundSource,
        volume: f32,
        pitch: f32,
    ) -> bool {
        if level != self.level.level() {
            return false;
        }
        self.sounds.push(FakeSound {
            sound: sound.to_owned(),
            at,
            source,
            volume,
            pitch,
        });
        true
    }
    fn level_game_event(
        &mut self,
        level: LevelRef,
        event: &str,
        at: [f64; 3],
        context: crate::GameEventContext,
    ) -> bool {
        if level != self.level.level() {
            return false;
        }
        self.game_events.push(FakeGameEvent {
            event: event.to_owned(),
            at,
            context,
        });
        true
    }
}
crate::impl_backend_mut_areas!(
    FakeHost,
    except(entities, fluids_energy, items, levels, players)
);

impl crate::prelude::PlayersClientBackend for FakeHost {}

/// Scope the controller lookup rows prepared by the API-owned registration fixture.
// hidden: API-owned test setup uses this vocabulary fixture, not mod authoring code.
#[doc(hidden)]
pub fn with_controller_tables<R>(
    controllers: Vec<crate::__private::parts_installation::ControllerInfoInput>,
    poses: Vec<crate::__private::parts_installation::PoseInfoInput>,
    f: impl FnOnce() -> R,
) -> R {
    let controllers: Vec<ControllerInfo> = controllers.into_iter().map(Into::into).collect();
    let controllers: &'static [_] = Box::leak(controllers.into_boxed_slice());
    let _poses = crate::client::arm_pose::test_table(poses.into_iter().map(Into::into).collect());
    let _controllers = RestoreControllers(TEST_CONTROLLERS.replace(Some(controllers)));
    f()
}

// no wire: read-only in-process phase fixture metadata.
#[derive(Default)]
// hidden: API-owned test setup uses this vocabulary fixture, not mod authoring code.
#[doc(hidden)]
// exhaustive: the platform or fixture constructs this process-local record as a complete unit.
pub struct TestReader {
    pub registries: std::collections::BTreeMap<String, std::sync::Arc<[String]>>,
    pub tags: std::sync::Arc<[(String, std::sync::Arc<[String]>)]>,
}
impl crate::LoadReader for TestReader {
    fn registry(&self, name: &str) -> Option<std::sync::Arc<[String]>> {
        self.registries.get(name).cloned()
    }
    fn fluid_tags(&self) -> std::sync::Arc<[(String, std::sync::Arc<[String]>)]> {
        self.tags.clone()
    }
    fn setting(&self, _: &str, _: &str) -> Option<String> {
        None
    }
}
impl crate::LoopReader for TestReader {
    fn registry(&self, name: &str) -> Option<std::sync::Arc<[String]>> {
        self.registries.get(name).cloned()
    }
    fn setting(&self, _: &str, _: &str) -> Option<String> {
        None
    }
}

#[cfg(test)]
#[path = "testing/handle_level_tests.rs"]
mod handle_level_tests;
