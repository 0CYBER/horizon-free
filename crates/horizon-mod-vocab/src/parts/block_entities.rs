use super::*;
/// What answers a capability for a block entity of one type: given the block entity and the
/// side asked about, its handler, or none to leave it to the next provider. A closure
/// `|host, at| Some(Box::new(Tank(at)))` is one.
pub trait BlockCapabilityProvider<T: ?Sized>: Send + Sync {
    /// The handler for the block entity at `at`, or none.
    fn provide(&self, host: &mut dyn ServerHost, at: BlockSide) -> Option<Box<T>>;
}
impl<T: ?Sized, F> BlockCapabilityProvider<T> for F
where
    F: Fn(&mut dyn ServerHost, BlockSide) -> Option<Box<T>> + Send + Sync,
{
    fn provide(&self, host: &mut dyn ServerHost, at: BlockSide) -> Option<Box<T>> {
        self(host, at)
    }
}
// no wire: trait-object alias inherits its provider trait policy; callbacks are not serialized.
/// A block capability provider, as the registrar keeps it.
pub type BlockProvider<T> = dyn BlockCapabilityProvider<T>;

pub use super::capabilities::providers_of::{BlockProviderOf, EntityProviderOf, ItemProviderOf};

/// Snapshot the callbacks once at registry preparation for worldgen workers.
pub fn block_entity_transforms()
-> std::collections::HashMap<String, Arc<dyn crate::BlockEntityTransform>> {
    installed().map_or_else(Default::default, |table| {
        table.block_entity_transforms.clone()
    })
}

/// The registered saved-data transform, looked up only during structure placement.
pub fn block_entity_transform(id: &str) -> Option<&'static dyn crate::BlockEntityTransform> {
    installed()?
        .block_entity_transforms
        .get(id)
        .map(AsRef::as_ref)
}

/// Owned declarations of the block entity types mods registered.
/// The game converts these snapshots to its content model when registration freezes.
pub fn block_entity_types() -> &'static [BlockEntityTypeDecl] {
    installed().map_or(&[], |installed| &installed.block_entity_types)
}

pub(crate) fn resolved_block_entity_types()
-> &'static OnceLock<horizon_mod_runtime::registry_keys::Names> {
    static EMPTY: OnceLock<horizon_mod_runtime::registry_keys::Names> = OnceLock::new();
    installed().map_or(&EMPTY, |installed| &installed.resolved_block_entity_types)
}

impl HookParts {
    pub(crate) fn block_entity<'k>(
        &self,
        id: impl FnOnce() -> Option<&'k str>,
    ) -> &[Attached<dyn BlockEntityPart>] {
        attached(&self.block_entities, id)
    }
    pub(crate) fn block_entity_kind<'k>(
        &self,
        id: impl FnOnce() -> Option<&'k str>,
    ) -> &[Attached<dyn BlockEntityPart>] {
        attached(&self.block_entity_kinds, id)
    }
}

/// The providers mods registered for capability `key` at `at`, of the block there and of the
/// type of the block entity standing there, for the game's own lookups (a hopper's): none where
/// no mod registered one, so a vanilla container's lookup costs one table check for each kind and
/// allocates nothing, and the game needn't work out its own answer. [`BlockProviders::ask`] then
/// asks them.
pub fn block_providers<T: ?Sized + 'static>(
    key: &CapabilityKey<T>,
    host: &dyn Host,
    at: BlockSide,
) -> Option<BlockProviders<T>> {
    let keys = &installed()?.keys;
    // Each table checked for the capability first, so a lookup no mod answers reads nothing at
    // the position.
    let entity = match keys.capabilities.contains_key(key.name) {
        true => providers_at(
            &keys.capabilities,
            key.name,
            at.kind(host)
                .and_then(crate::content::registry_keys::block_entity_type_name),
        ),
        false => &[],
    };
    let block = match keys.block_capabilities.contains_key(key.name) {
        true => providers_at(
            &keys.block_capabilities,
            key.name,
            at.level
                .block(host, at.pos)
                .and_then(crate::content::registry_keys::block_id),
        ),
        false => &[],
    };
    if entity.is_empty() && block.is_empty() {
        return None;
    }
    Some(BlockProviders {
        entity,
        block,
        at,
        handler: PhantomData,
    })
}

// no wire: provider lookup borrows static capability dispatch tables and process-local type identity.
/// A position's providers of one capability ([`block_providers`]).
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct BlockProviders<T: ?Sized + 'static> {
    pub(crate) entity: &'static [CapabilityPart<BlockProviderOf>],
    pub(crate) block: &'static [CapabilityPart<BlockProviderOf>],
    pub(crate) at: BlockSide,
    pub(crate) handler: PhantomData<fn() -> Box<T>>,
}

impl<T: ?Sized + 'static> BlockProviders<T> {
    /// A mod's handler where the game would answer with its own unless a mod comes first: one
    /// registered `before_game`, else, where the game has no answer (`game`), the first mod's,
    /// in [`LevelRef::capability`]'s order.
    pub fn ask(self, host: &mut dyn ServerHost, game: bool) -> Option<Capability<T>> {
        let at = self.at;
        let answer = ask_providers(
            host,
            |index| next_merged(self.entity, self.block, index),
            BlockProviderOf::get::<T>,
            |provider, host| provider.provide(host, at),
            |_| game,
        );
        match answer {
            Answer::Mod(found) => Some(found),
            Answer::Game | Answer::Nobody => None,
        }
    }
}

impl dyn Registrar + '_ {
    /// Declare a block entity type in a namespace owned by this mod's manifest.
    pub fn block_entity_type(&mut self, id: &str) -> crate::BlockEntityTypeBuilder<'_> {
        crate::BlockEntityTypeBuilder {
            id: id.to_owned(),
            registrar: self,
            transform: None,
            blocks: Vec::new(),
        }
    }
    /// Attach a Part to a native type, including one another mod declares later.
    /// Vanilla keys are usable for capabilities, but not for native payload Parts.
    pub fn block_entity_part<P: BlockEntityPart>(
        &mut self,
        kind: &crate::BlockEntityTypeKey,
        part: P,
    ) -> Result<(), crate::BlockEntityPartError> {
        self.attach_block_entity_part(kind, type_name::<P>(), Arc::new(part))
    }
    /// [`Self::block_entity_part`] for a Part with typed state (`BlockEntityPartState`):
    /// each block entity of type `kind` keeps one `P::Data`, under `P::DATA`. Refused, and
    /// reported as the mod's registration refusal, when `P::DATA` syncs, copies on death,
    /// lacks the codec a wasm mod needs, is outside the mod's namespaces, or names a key
    /// this mod already declared with another type.
    pub fn block_entity_part_with_state<P: crate::BlockEntityPartState>(
        &mut self,
        kind: &crate::BlockEntityTypeKey,
        part: P,
    ) -> Result<(), crate::BlockEntityStateError> {
        self.declare_block_entity_part_state(
            type_name::<P>(),
            crate::PartStateDeclaration::of(P::DATA),
        )?;
        self.block_entity_part(kind, part)
            .map_err(crate::BlockEntityStateError::Part)
    }
    /// Answer capability `key` for block entity type `kind` with `provider`, which gives
    /// a handler for one block entity and side, or none:
    /// `r.block_entity_capability(&ITEMS, &press, |_, _| Some(Box::new(PressItems)))`.
    /// Like NeoForge's `RegisterCapabilitiesEvent`, providers add up: in the fork's chosen order
    /// (NeoForge's javadoc states none), the game's own is asked first, then the mods' in
    /// registration order, and the first to answer wins.
    pub fn block_entity_capability<T: ?Sized + 'static>(
        &mut self,
        key: &CapabilityKey<T>,
        kind: &crate::BlockEntityTypeKey,
        provider: impl Fn(&mut dyn ServerHost, BlockSide) -> Option<Box<T>> + Send + Sync + 'static,
    ) {
        self.provide_block_entity_capability(
            key.name,
            kind,
            false,
            BlockProviderOf::of(Arc::new(provider)),
        );
    }
    /// `block_entity_capability`, asked ahead of the game's own
    /// provider (NeoForge's priority), as a mod that replaces how a block entity's slots are
    /// reached.
    pub fn block_entity_capability_before_game<T: ?Sized + 'static>(
        &mut self,
        key: &CapabilityKey<T>,
        kind: &crate::BlockEntityTypeKey,
        provider: impl Fn(&mut dyn ServerHost, BlockSide) -> Option<Box<T>> + Send + Sync + 'static,
    ) {
        self.provide_block_entity_capability(
            key.name,
            kind,
            true,
            BlockProviderOf::of(Arc::new(provider)),
        );
    }
}

/// Registrar operations for block entities.
pub trait BlockEntitiesRegistrar {
    /// Set the transform of a type declared by this mod.
    fn set_block_entity_transform(
        &mut self,
        key: crate::BlockEntityTypeKey,
        step: Arc<dyn crate::BlockEntityTransform>,
    );
    /// Look up a vanilla or another mod's type. The final registry validates it at load.
    fn block_entity_type_key(&mut self, id: &str) -> crate::BlockEntityTypeKey;

    /// Declare block entity type `path` in a namespace owned by this mod's manifest, held by `blocks`
    /// (a bare name is this mod's); mods usually reach it through `block_entity_type`.
    fn declare_block_entity_type(
        &mut self,
        path: &str,
        blocks: Vec<String>,
    ) -> crate::BlockEntityTypeKey;

    /// Attach `part`, of kind `kind`, to the mod block entity type `key`.
    fn attach_block_entity_part(
        &mut self,
        key: &crate::BlockEntityTypeKey,
        kind: &'static str,
        part: Arc<dyn BlockEntityPart>,
    ) -> Result<(), crate::BlockEntityPartError>;

    /// Record the state declaration of Part `part`, or refuse it (as the mod's registration
    /// refusal too): see `PartStateDeclaration::check`, and a key outside this mod's
    /// namespaces or one it declared with another type. Mods reach it through
    /// `block_entity_part_with_state`.
    fn declare_block_entity_part_state(
        &mut self,
        part: &'static str,
        declaration: crate::PartStateDeclaration,
    ) -> Result<(), crate::BlockEntityStateError>;

    /// Answer block entity capability `name` for type `kind` with `provider`, asked ahead of the
    /// game's own when `before_game`; mods reach it through `block_entity_capability`.
    fn provide_block_entity_capability(
        &mut self,
        name: &'static str,
        kind: &crate::BlockEntityTypeKey,
        before_game: bool,
        provider: BlockProviderOf,
    );
}
