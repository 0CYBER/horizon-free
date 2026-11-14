//! Shared table state, cross-kind registration and common test fixtures only.
use super::*;
use std::cell::Cell;
use std::sync::{PoisonError, RwLock, atomic::AtomicU64};
// no wire: process-local registration and dispatch storage contains trait objects and owning-mod identities.
/// A part, with the mod that attached it.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct Attached<P: ?Sized> {
    /// The mod's bundle id, retained for diagnostics after registration.
    pub(crate) mod_id: &'static str,
    /// The mod's place in dispatch order.
    pub(crate) mod_index: usize,
    /// Its place in its mod's registrations: a mod's parts on one thing keep this order, key or
    /// tag ( §3).
    pub(crate) order: usize,
    /// The part's kind, its Rust type's name: one part of each kind per thing.
    pub(crate) type_name: &'static str,
    pub(crate) part: Arc<P>,
}

impl<P: ?Sized> Attached<P> {
    pub(crate) fn with_part<Q: ?Sized>(&self, part: Arc<Q>) -> Attached<Q> {
        Attached {
            mod_id: self.mod_id,
            mod_index: self.mod_index,
            order: self.order,
            type_name: self.type_name,
            part,
        }
    }
    // hidden: the game's goal registration reads these (#1077)
    #[doc(hidden)]
    pub fn mod_id(&self) -> &'static str {
        self.mod_id
    }
    // hidden: the game's goal registration reads these (#1077)
    #[doc(hidden)]
    pub fn type_name(&self) -> &'static str {
        self.type_name
    }
    // hidden: the game's goal registration reads these (#1077)
    #[doc(hidden)]
    pub fn part(&self) -> &Arc<P> {
        &self.part
    }
}

impl<P: ?Sized> Clone for Attached<P> {
    fn clone(&self) -> Self {
        self.with_part(self.part.clone())
    }
}

// no wire: process-local registration and dispatch storage contains trait objects and owning-mod identities.
/// Every mod's parts, by the id of the thing they are attached to, in dispatch order.
#[derive(Clone, Default)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct Parts {
    pub(crate) key_mappings: Vec<(&'static str, crate::KeyRegistration)>,
    /// Settings by owner, in registration order (`Registrar::register_setting`).
    pub(crate) settings: Vec<(&'static str, crate::SettingSpec)>,
    /// Block parts, and block entity parts as the block parts they also are.
    pub(crate) blocks: HashMap<Box<str>, Vec<Attached<dyn BlockPart>>>,
    /// Block entity parts by block, for the hooks asked by block (`block_entity`, `screen`).
    pub(crate) block_entities: HashMap<Box<str>, Vec<Attached<dyn BlockEntityPart>>>,
    /// Block entity parts by block entity type, for the hooks asked by the entity
    /// (`block_entity_tick`, `menu_button`): its type id need not be its block's id.
    pub(crate) block_entity_kinds: HashMap<Box<str>, Vec<Attached<dyn BlockEntityPart>>>,
    pub(crate) items: HashMap<Box<str>, Vec<Attached<dyn ItemPart>>>,
    pub(crate) menus: HashMap<Box<str>, Vec<Attached<dyn MenuPart>>>,
    pub(crate) fluids: HashMap<Box<str>, Vec<Attached<dyn FluidPart>>>,
    /// Entity parts by entity type id (`minecraft:zombie`, `mymod:robot`), for the goal
    /// registration at every `Mob` construction and the scoped hooks the entity
    /// scope owns (`general_entity_tick`).
    pub(crate) entities: HashMap<Box<str>, Vec<Attached<dyn EntityPart>>>,
    /// Capability providers by capability name, then by block entity type, in registration
    /// order.
    pub(crate) capabilities: Capabilities<BlockProviderOf>,
    /// Block capability providers by capability name, then by block, in registration order:
    /// those asked of a block whatever block entity it has, or none.
    pub(crate) block_capabilities: Capabilities<BlockProviderOf>,
    /// Canonical block capability metadata, retained independently of provider availability.
    pub(crate) block_capability_metadata: HashMap<&'static str, (Arc<str>, Option<bool>)>,
    /// Built-in declaration order followed by the order mods first mention a capability.
    pub(crate) block_capability_order: Vec<&'static str>,
    /// Explicit domains and declarations of the game's default providers.
    pub(crate) default_block_containers: HashMap<Box<str>, Vec<(usize, crate::ModContainer)>>,
    pub(crate) default_block_fluids: HashMap<Box<str>, Vec<(usize, crate::ModFluidTanks)>>,
    pub(crate) default_block_energy: HashMap<Box<str>, Vec<(usize, crate::ModEnergyBuffer)>>,
    pub(crate) default_item_containers:
        HashMap<Box<str>, Vec<(usize, Arc<dyn crate::ItemContainerProvider>)>>,
    /// Item capability providers by capability name, then by item, in registration order.
    pub(crate) item_capabilities: Capabilities<ItemProviderOf>,
    /// Entity capability providers by capability name, then by entity type, in registration
    /// order.
    pub(crate) entity_capabilities: Capabilities<EntityProviderOf>,
    /// The mods' own item places (`ItemPlace::Mod`), by provider name.
    pub(crate) place_providers: HashMap<&'static str, PlaceProviderPart>,
    /// The tooltip appenders, in registration order.
    pub(crate) tooltip_appenders: Vec<TooltipAppenderPart>,
    /// The crash report lines, in registration order.
    pub(crate) crash_sections: Vec<CrashSectionPart>,
    /// Command argument types the mods registered, by namespaced id, in registration order.
    pub(crate) command_arguments: Vec<CommandArgumentInfo>,
    /// Separately declared mod recipe types.
    pub(crate) recipe_types: HashMap<Box<str>, usize>,
    /// Serializer IDs, their produced recipe types and retained owners.
    pub(crate) recipe_serializers:
        HashMap<Box<str>, (usize, String, Arc<dyn crate::CraftingRecipeHandler>)>,
    /// Entity selector tokens the mods registered, by name, in registration order.
    pub(crate) entity_selectors: Vec<EntitySelectorInfo>,
    /// Frozen raw payload declarations and their sole receiving owners.
    pub(crate) payloads: Option<Arc<crate::parts::payloads::Payloads>>,
    /// Only the parts that override each scoped hook, resolved at load or reload.
    pub(crate) hooks: Vec<Option<HookParts>>,
}

// no wire: process-local registration and dispatch storage contains trait objects and owning-mod identities.
#[derive(Clone, Default)]
/// The Parts attached to blocks for one hook, with which hooks have a subscriber.
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct HookParts {
    pub(crate) blocks: HashMap<Box<str>, Vec<Attached<dyn BlockPart>>>,
    pub(crate) block_keys: HashMap<crate::BlockKey, Vec<Attached<dyn BlockPart>>>,
    pub(crate) block_entities: HashMap<Box<str>, Vec<Attached<dyn BlockEntityPart>>>,
    pub(crate) block_entity_kinds: HashMap<Box<str>, Vec<Attached<dyn BlockEntityPart>>>,
    pub(crate) items: HashMap<Box<str>, Vec<Attached<dyn ItemPart>>>,
    pub(crate) menus: HashMap<Box<str>, Vec<Attached<dyn MenuPart>>>,
    pub(crate) fluids: HashMap<Box<str>, Vec<Attached<dyn FluidPart>>>,
    pub(crate) entities: HashMap<Box<str>, Vec<Attached<dyn EntityPart>>>,
}

/// Which things a capability's providers are registered by.
// hidden: a lookup's view of the registrar's table, not part of the mod surface
// exhaustive: the four kinds of thing a capability is asked of
#[doc(hidden)]
#[derive(Clone, Copy, Debug, horizon_mod_api_macros::Wire)]
pub enum ProvidedBy {
    BlockEntity,
    Block,
    Item,
    Entity,
}

/// The parts attached by tag, by tag name, in registration order.
#[derive(Clone, Default)]
pub(crate) struct TagParts {
    pub(crate) blocks: Vec<(Box<str>, Attached<dyn BlockPart>)>,
    pub(crate) items: Vec<(Box<str>, Attached<dyn ItemPart>)>,
}

#[path = "shared/registration_state.rs"]
mod registration_state;
pub(crate) use registration_state::Installed;

pub(crate) fn requested_keys() -> &'static horizon_mod_runtime::registry_keys::Requests {
    static EMPTY: LazyLock<horizon_mod_runtime::registry_keys::Requests> =
        LazyLock::new(Default::default);
    installed().map_or(&EMPTY, |installed| &installed.registry_keys)
}

pub(crate) static INSTALLED: OnceLock<Installed> = OnceLock::new();
#[cfg(any(test, feature = "testing"))]
thread_local! {
    /// Tests install their own parts per thread, as they do their mods (`TEST_MODS`).
    pub(crate) static TEST_PARTS: Cell<Option<&'static Installed>> = const { Cell::new(None) };
}
#[cfg(test)]
thread_local! {
    pub(crate) static TEST_PART_LOOKUPS: Cell<usize> = const { Cell::new(0) };
}
pub(crate) static NO_PARTS: LazyLock<Parts> = LazyLock::new(Parts::default);

/// The validity context travels with the owning game's lookup table.
pub(crate) struct GamePartsLookup {
    pub(super) parts: Option<Rc<Parts>>,
    pub(super) game_thread: bool,
    pre_world: bool,
}
thread_local! {
    /// The per-game table and the context that determines which fallback is valid.
    pub(crate) static GAME_PARTS: RefCell<GamePartsLookup> = const { RefCell::new(GamePartsLookup {
        parts: None, game_thread: false, pre_world: false,
    }) };
}

pub(crate) fn installed() -> Option<&'static Installed> {
    #[cfg(any(test, feature = "testing"))]
    if let Some(installed) = TEST_PARTS.get() {
        return Some(installed);
    }
    INSTALLED.get()
}

/// Whether any installed part subscribes to a scoped hook.
pub fn has_part_hook(hook: HookId) -> bool {
    installed().is_some_and(|installed| installed.hook_bits[hook as usize])
}

/// Preserve the no-mod fast path for level hooks.
pub fn has_scoped_hook(hook: HookId) -> bool {
    !crate::mods().is_empty() && has_part_hook(hook)
}

// no wire: process-local registration and dispatch storage contains trait objects and owning-mod identities.
/// The parts table a dispatch reads: the process-wide one, this game thread's, or the one the
/// server published for its world generation.
#[non_exhaustive]
pub enum PartsRef {
    /// The process-wide table.
    Shared(&'static Parts),
    /// This game thread's table.
    Game(Rc<Parts>),
    /// The table the server published for its world generation, as this thread holds it.
    Worldgen(Arc<Parts>),
}

impl std::ops::Deref for PartsRef {
    type Target = Parts;
    fn deref(&self) -> &Parts {
        match self {
            Self::Shared(parts) => parts,
            Self::Game(parts) => parts,
            Self::Worldgen(parts) => parts,
        }
    }
}

/// The Part table the server published for its world generation, with its generation: 0 before
/// a server published one.
static WORLDGEN_PARTS: RwLock<(u64, Option<Arc<Parts>>)> = RwLock::new((0, None));
/// The generation in `WORLDGEN_PARTS`, read without its lock.
static WORLDGEN_GENERATION: AtomicU64 = AtomicU64::new(0);

thread_local! {
    /// This thread's handle on the published table, with the generation it belongs to.
    static WORLDGEN_TABLE: RefCell<Option<(u64, Option<Arc<Parts>>)>> =
        const { RefCell::new(None) };
    /// Whether this thread runs a feature, whose lookups keep the table it started with.
    static IN_FEATURE: Cell<bool> = const { Cell::new(false) };
}

/// Publish the Part table this server resolved against its tags, at world open and at each
/// `/reload`, for the Part methods asked while a feature generates: the worldgen workers and the
/// server thread read it from their next feature on. Only a server publishes, so a client window's
/// table never reaches the workers.
pub fn publish_worldgen_parts(parts: Option<Parts>) {
    let mut published = WORLDGEN_PARTS
        .write()
        .unwrap_or_else(PoisonError::into_inner);
    let generation = published.0 + 1;
    *published = (generation, parts.map(Arc::new));
    WORLDGEN_GENERATION.store(generation, Ordering::Release);
}

/// The parts a worldgen Part method asks, on any thread: the table the server published, never
/// this thread's game table nor the process-wide key parts. It never fails: before a server
/// published, no part answers. One atomic load; the table's handle is fetched again only when the
/// server published since, and never inside a feature, which keeps the table it started with.
pub fn worldgen_parts() -> PartsRef {
    #[cfg(test)]
    TEST_PART_LOOKUPS.set(TEST_PART_LOOKUPS.get() + 1);
    WORLDGEN_TABLE.with_borrow_mut(|table| {
        if !IN_FEATURE.get() || table.is_none() {
            refresh_worldgen_table(table);
        }
        match table {
            Some((_, Some(parts))) => PartsRef::Worldgen(parts.clone()),
            _ => PartsRef::Shared(&NO_PARTS),
        }
    })
}

fn refresh_worldgen_table(table: &mut Option<(u64, Option<Arc<Parts>>)>) {
    let generation = WORLDGEN_GENERATION.load(Ordering::Acquire);
    if table.as_ref().is_some_and(|(seen, _)| *seen == generation) {
        return;
    }
    let published = WORLDGEN_PARTS
        .read()
        .unwrap_or_else(PoisonError::into_inner);
    *table = Some((published.0, published.1.clone()));
}

/// Run one feature: its worldgen lookups read the table published when it started, so a feature
/// that started before a `/reload` finishes with the tables it started with.
pub fn worldgen_feature<T>(feature: impl FnOnce() -> T) -> T {
    struct Leave(bool);
    impl Drop for Leave {
        fn drop(&mut self) {
            IN_FEATURE.set(self.0);
        }
    }
    let outer = IN_FEATURE.replace(true);
    let _leave = Leave(outer);
    if !outer {
        WORLDGEN_TABLE.with_borrow_mut(refresh_worldgen_table);
    }
    feature()
}

/// The mods whose parts on `state`'s block answer the worldgen hook `hook`, for a refused answer
/// to name.
pub fn worldgen_part_mods(hook: HookId, state: crate::BlockState) -> Vec<&'static str> {
    let parts = worldgen_parts();
    parts.for_hook(hook).map_or_else(Vec::new, |table| {
        table
            .block_value(state)
            .iter()
            .map(|part| part.mod_id)
            .collect()
    })
}

/// The parts hook `hook` asks: this game thread's resolved table, then the shared key parts
/// when there are no tag parts, then the off-thread path.
pub fn parts(hook: HookId, name: &'static str) -> PartsRef {
    try_parts(hook, name).unwrap_or_else(|error| panic!("{error}"))
}

// no wire: process-local registration diagnostic retains a static hook or mod identity.
/// The engine asked for tag Parts on a thread without that world's table.
#[derive(Debug)]
#[non_exhaustive]
pub struct MissingResolvedParts {
    /// The dispatch that required the table.
    pub hook: &'static str,
    /// The thread that attempted the dispatch.
    pub thread: String,
}

impl std::fmt::Display for MissingResolvedParts {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "hook {} requires resolved Parts on thread {}",
            self.hook, self.thread
        )
    }
}

pub(crate) fn try_parts(
    hook: HookId,
    name: &'static str,
) -> Result<PartsRef, MissingResolvedParts> {
    #[cfg(test)]
    TEST_PART_LOOKUPS.set(TEST_PART_LOOKUPS.get() + 1);
    let Some(installed) = installed() else {
        return Ok(PartsRef::Shared(&NO_PARTS));
    };
    let (game_parts, pre_world) =
        GAME_PARTS.with_borrow(|lookup| (lookup.parts.clone(), lookup.pre_world));
    if let Some(parts) = game_parts {
        return Ok(PartsRef::Game(parts));
    }
    if installed.tag_hook_bits[hook as usize] && !pre_world {
        return Err(MissingResolvedParts {
            hook: name,
            thread: std::thread::current()
                .name()
                .unwrap_or("unnamed")
                .to_owned(),
        });
    }
    Ok(PartsRef::Shared(&installed.keys))
}

/// Enter a worldless game context, discarding the previous world's table.
pub fn clear_game_parts() {
    let previous = GAME_PARTS.with_borrow_mut(|lookup| {
        lookup.pre_world = true;
        lookup.parts.take()
    });
    drop(previous);
}

/// Mark this thread as running a `Game`, whose dispatches read its own resolved parts.
pub fn mark_game_thread() {
    GAME_PARTS.with_borrow_mut(|lookup| lookup.game_thread = true);
    clear_game_parts();
}

// no wire: registration diagnostic contains ItemKeyError with static owning-mod identity.
/// Refusals at the Parts load boundary.
#[derive(Debug)]
#[non_exhaustive]
pub enum ResolvePartsError {
    /// An item key names no loaded item.
    ItemKeys(Vec<ItemKeyError>),
    /// Block or item Part resolution refusals, including unavailable block metadata.
    Parts(Vec<String>),
}

// no wire: borrowed parser or registration callbacks execute only in their owning process.
/// The ids a game knows, which a part's key must name.
// exhaustive: the game builds it with a literal; mods only read it
pub struct KnownIds<'a> {
    /// Whether a block id is known.
    pub block: &'a dyn Fn(&str) -> bool,
    /// Whether an item id is known.
    pub item: &'a dyn Fn(&str) -> bool,
}

/// Resolve this thread's game's parts against its tags (`block_tags`, `item_tags`: flattened,
/// tag name without `#` to members). Each run starts again from the key parts, so `/reload` drops
/// what left a tag. A key naming a block or item the game doesn't know is an error, naming the
/// mod; a missing tag or a part a tag gives twice is a warning, and the earlier mod's part stays.
pub fn prepare_parts(
    block_tags: &HashMap<String, HashSet<String>>,
    item_tags: &HashMap<String, HashSet<String>>,
    known: KnownIds<'_>,
) -> Result<Option<Parts>, ResolvePartsError> {
    let Some(installed) = installed() else {
        return Ok(None);
    };
    if let Err(errors) = installed.registry_keys.items.resolve(
        |id| (known.item)(id).then_some(()),
        |mod_id, id| ItemKeyError {
            mod_id,
            id: id.to_owned(),
        },
    ) {
        return Err(ResolvePartsError::ItemKeys(errors));
    }
    let mut unknown: Vec<String> = unknown_keys(&installed.keys.blocks, known.block, "block")
        .chain(unknown_keys(&installed.keys.items, known.item, "item"))
        .collect();
    if !unknown.is_empty() {
        unknown.sort();
        return Err(ResolvePartsError::Parts(unknown));
    }
    let mut parts = installed.keys.clone();
    add_tagged(
        &mut parts.blocks,
        &installed.tags.blocks,
        block_tags,
        "block",
    );
    add_tagged(&mut parts.items, &installed.tags.items, item_tags, "item");
    parts.index_hooks();
    for table in parts.hooks.iter_mut().flatten() {
        if table.blocks.is_empty() {
            continue;
        }
        table.block_keys = table
            .blocks
            .iter()
            .map(|(name, attached)| {
                crate::content::registry_keys::block_key(name)
                    .map(|key| (key, attached.clone()))
                    .ok_or_else(|| {
                        ResolvePartsError::Parts(vec![format!(
                            "block metadata unavailable for {name}"
                        )])
                    })
            })
            .collect::<Result<_, _>>()?;
    }
    Ok(Some(parts))
}

/// Publish Parts prepared for this owning game thread.
pub fn install_resolved_parts(parts: Option<Parts>) {
    if let Some(parts) = parts {
        let previous = GAME_PARTS.with_borrow_mut(|lookup| {
            lookup.pre_world = false;
            lookup.parts.replace(Rc::new(parts))
        });
        drop(previous);
    }
}

/// Resolve and install Parts at a load boundary.
pub fn resolve_parts(
    block_tags: &HashMap<String, HashSet<String>>,
    item_tags: &HashMap<String, HashSet<String>>,
    known: KnownIds<'_>,
) -> Result<(), ResolvePartsError> {
    install_resolved_parts(prepare_parts(block_tags, item_tags, known)?);
    Ok(())
}

/// A line for each part attached to an id `known` refuses.
pub(crate) fn unknown_keys<'a, P: ?Sized>(
    table: &'a HashMap<Box<str>, Vec<Attached<P>>>,
    known: &'a dyn Fn(&str) -> bool,
    scope: &'a str,
) -> impl Iterator<Item = String> + 'a {
    table
        .iter()
        .filter(|(id, _)| !known(id))
        .flat_map(move |(id, list)| {
            list.iter().map(move |part| {
                format!(
                    "{} attached {} to {scope} {id}, which this game does not have",
                    part.mod_id, part.type_name
                )
            })
        })
}

/// Attach each tag part to its tag's members, in dispatch order: by mod, then by registration
/// within a mod.
pub(crate) fn add_tagged<P: ?Sized>(
    table: &mut HashMap<Box<str>, Vec<Attached<P>>>,
    tagged: &[(Box<str>, Attached<P>)],
    tags: &HashMap<String, HashSet<String>>,
    scope: &str,
) {
    for (tag, part) in tagged {
        let Some(members) = tags.get(&**tag) else {
            log::warn!(
                "{} attached {} to {scope} tag #{tag}, which does not exist",
                part.mod_id,
                part.type_name
            );
            continue;
        };
        for member in members {
            let list = table.entry(member.as_str().into()).or_default();
            match list.iter_mut().find(|a| a.type_name == part.type_name) {
                Some(other) => {
                    log::warn!(
                        "{} is attached to {member} twice, by {} and {} (#{tag}); {}'s is kept",
                        part.type_name,
                        other.mod_id,
                        part.mod_id,
                        if other.mod_index < part.mod_index {
                            other.mod_id
                        } else {
                            part.mod_id
                        },
                    );
                    if part.mod_index < other.mod_index {
                        *other = part.clone();
                    }
                }
                None => list.push(part.clone()),
            }
            list.sort_by_key(|a| (a.mod_index, a.order));
        }
    }
}

impl Parts {
    pub(crate) fn for_hook(&self, hook: HookId) -> Option<&HookParts> {
        self.hooks.get(hook as usize).and_then(Option::as_ref)
    }
    pub(super) fn index_hooks(&mut self) {
        self.hooks = vec![None; HOOK_COUNT];
        macro_rules! index {
            ($field:ident, $method:ident) => {
                for (key, parts) in &self.$field {
                    for part in parts {
                        for hook in part.part.$method().iter().flatten() {
                            let table =
                                self.hooks[*hook as usize].get_or_insert_with(HookParts::default);
                            table
                                .$field
                                .entry(key.clone())
                                .or_default()
                                .push(part.clone());
                        }
                    }
                }
            };
        }
        index!(blocks, block_subscribed);
        index!(block_entities, block_entity_subscribed);
        index!(block_entity_kinds, block_entity_subscribed);
        index!(items, item_subscribed);
        index!(menus, menu_subscribed);
        index!(fluids, fluid_subscribed);
        index!(entities, entity_subscribed);
    }
}

/// Walk only this capability and type's registrations, once each even if a provider switches
/// a mod off during the lookup. The table and the game's direct lookups share this path.
pub(crate) fn next_provider<P: Clone>(
    parts: &[CapabilityPart<P>],
    index: &mut u32,
) -> Option<(P, &'static str, bool)> {
    while let Some(part) = parts.get(*index as usize) {
        *index += 1;
        if crate::mod_index_on(part.mod_index) {
            return Some((part.provider.clone(), part.mod_id, part.before_game));
        }
    }
    None
}

pub(crate) fn attached<'a, 'k, P: ?Sized>(
    table: &'a HashMap<Box<str>, Vec<Attached<P>>>,
    id: impl FnOnce() -> Option<&'k str>,
) -> &'a [Attached<P>] {
    if table.is_empty() {
        return &[];
    }
    id().and_then(|id| table.get(id)).map_or(&[], Vec::as_slice)
}

#[cfg(test)]
mod lookup_context_tests {
    use super::*;

    struct DropObserver(Arc<AtomicBool>);
    #[mod_api::horizon_part]
    impl BlockPart for DropObserver {}
    impl Drop for DropObserver {
        fn drop(&mut self) {
            self.0.store(
                GAME_PARTS.with(|lookup| lookup.try_borrow().is_ok()),
                Ordering::Relaxed,
            );
        }
    }
    fn table(observed: Arc<AtomicBool>) -> Parts {
        let mut parts = Parts::default();
        parts.blocks.insert(
            "test:drop".into(),
            vec![Attached {
                mod_id: "drop",
                mod_index: 0,
                order: 0,
                type_name: "DropObserver",
                part: Arc::new(DropObserver(observed)),
            }],
        );
        parts
    }
    #[test]
    fn replacing_and_clearing_lookup_tables_release_the_borrow_before_dropping_parts() {
        let first = Arc::new(AtomicBool::new(false));
        let second = Arc::new(AtomicBool::new(false));
        install_resolved_parts(Some(table(first.clone())));
        install_resolved_parts(Some(table(second.clone())));
        assert!(first.load(Ordering::Relaxed));
        clear_game_parts();
        assert!(second.load(Ordering::Relaxed));
    }
    #[test]
    fn unsubscribed_scoped_dispatch_never_looks_up_parts() {
        struct Level;
        impl crate::LevelHost for Level {}
        struct Hook;
        #[mod_api::__subscriptions]
        impl crate::Hooks for Hook {}
        crate::testing::with_dispatch(
            vec![crate::ModEntry::new("probe", Box::new(Hook))].into(),
            || {
                super::super::install_input(super::super::installation::InstalledInput::default());
                TEST_PART_LOOKUPS.set(0);
                let state =
                    crate::BlockState::from(crate::BlockStateRecord::new(crate::BlockKey(0), 0));
                let mut answer = false;
                crate::dispatch_should_check_weak_power(
                    &Level,
                    crate::LevelDimension::Overworld,
                    crate::BlockPos::new(0, 64, 0),
                    state,
                    crate::Direction::East,
                    &mut answer,
                );
                assert_eq!(TEST_PART_LOOKUPS.get(), 0);
                assert!(!answer);
            },
        );
    }

    #[test]
    fn with_no_parts_the_block_entity_is_never_named() {
        let parts = HookParts::default();
        let named = std::cell::Cell::new(false);
        let id = || {
            named.set(true);
            Some("test:press")
        };
        assert!(parts.block_entity(id).is_empty());
        assert!(!named.get(), "an empty table resolved the block's name");
    }
}

#[cfg(test)]
#[path = "shared/worldgen_tests.rs"]
mod worldgen_tests;
