use super::*;
/// Whether this hook has a prepared subscriber for an API block key.
/// The caller owns validation of the routing key.
pub fn block_key_listens(hook: HookId, key: crate::BlockKey) -> bool {
    parts(hook, "block routing")
        .for_hook(hook)
        .is_some_and(|table| !table.block_key(Some(key)).is_empty())
}

/// Whether every part on this API block key that answers `explosion_resistance` declares the
/// answer pure (`BlockPart::explosion_resistance_is_pure`), so a batched read may ask once a state.
pub fn explosion_resistance_key_pure(key: crate::BlockKey) -> bool {
    parts(HookId::ExplosionResistance, "block routing")
        .for_hook(HookId::ExplosionResistance)
        .is_none_or(|table| {
            table
                .block_key(Some(key))
                .iter()
                .all(|attached| attached.part.explosion_resistance_is_pure())
        })
}

/// Whether the feature-pinned published table subscribes to this worldgen block key.
pub fn worldgen_block_key_listens(hook: HookId, key: crate::BlockKey) -> bool {
    worldgen_parts()
        .for_hook(hook)
        .is_some_and(|table| !table.block_key(Some(key)).is_empty())
}

impl HookParts {
    /// Prepared subscribers for an API block key; missing keys have no subscribers.
    pub fn block_key(&self, key: Option<crate::BlockKey>) -> &[Attached<dyn BlockPart>] {
        key.and_then(|key| self.block_keys.get(&key))
            .map_or(&[], |parts| parts.as_slice())
    }
    /// Parts for a checked API state. Key lookup does not depend on world state numbers.
    pub fn block_value(&self, state: crate::BlockState) -> &[Attached<dyn BlockPart>] {
        let Some(blocks) = crate::__private::block_definitions() else {
            return &[];
        };
        let Some(definition) = blocks.definitions.get(state.block().0 as usize) else {
            return &[];
        };
        if definition.property(state.ordinal(), "").is_err() {
            return &[];
        }
        self.block_key(Some(state.block()))
    }
}

impl dyn Registrar + '_ {
    /// Attach `part` to block `block` (`mymod:shaft`, `minecraft:oak_log`).
    pub fn block_part<P: BlockPart>(&mut self, block: crate::BlockKey, part: P) {
        self.attach_block_part(block, type_name::<P>(), Arc::new(part));
    }
    /// Attach `part` to every block in tag `tag` (`minecraft:logs`, without `#`), as each game's
    /// tags have it when its world opens and after `/reload`.
    pub fn block_part_tag<P: BlockPart>(&mut self, tag: &str, part: P) {
        self.attach_block_part_tag(tag, type_name::<P>(), Arc::new(part));
    }
    /// Answer capability `key` for block `block` with `provider`, whether or not a block entity
    /// stands there (NeoForge's `RegisterCapabilitiesEvent.registerBlock`): a trash can that voids
    /// what it is given, a machine's port block passing a pipe on to its controller. `provider`
    /// is given the position and side asked about, the block state there and the block entity,
    /// if any: `r.block_capability(&ITEMS, &bin, |_, _, _, _| Some(Box::new(Voids)))`. Asked
    /// in one list with the providers of the block entity type there, as
    /// [`LevelRef::capability`](crate::LevelRef::capability) orders them.
    pub fn block_capability<T: ?Sized + 'static>(
        &mut self,
        key: &CapabilityKey<T>,
        block: &crate::BlockKey,
        provider: impl Fn(
            &mut dyn ServerHost,
            BlockSide,
            crate::BlockState,
            Option<crate::BlockEntityRef>,
        ) -> Option<Box<T>>
        + Send
        + Sync
        + 'static,
    ) {
        self.provide_block_capability(key.name, block, false, block_provider(*block, provider));
    }
    /// `block_capability`, asked ahead of the game's own provider.
    pub fn block_capability_before_game<T: ?Sized + 'static>(
        &mut self,
        key: &CapabilityKey<T>,
        block: &crate::BlockKey,
        provider: impl Fn(
            &mut dyn ServerHost,
            BlockSide,
            crate::BlockState,
            Option<crate::BlockEntityRef>,
        ) -> Option<Box<T>>
        + Send
        + Sync
        + 'static,
    ) {
        self.provide_block_capability(key.name, block, true, block_provider(*block, provider));
    }
}

/// A block provider as the table keeps it: one asked by position and side, which reads the block
/// state and block entity there for `provider`. A position whose block can't be read (a stale
/// level, an unloaded chunk), or is no longer `block`, has no answer.
fn block_provider<T: ?Sized + 'static>(
    block: crate::BlockKey,
    provider: impl Fn(
        &mut dyn ServerHost,
        BlockSide,
        crate::BlockState,
        Option<crate::BlockEntityRef>,
    ) -> Option<Box<T>>
    + Send
    + Sync
    + 'static,
) -> BlockProviderOf {
    BlockProviderOf::of(Arc::new(move |host: &mut dyn ServerHost, at: BlockSide| {
        // The block may have changed since the lookup chose this provider: an earlier provider
        // replaced it, say. Then it is another block's position, and this one has no answer.
        if at.level.block(host, at.pos)? != block {
            return None;
        }
        let state = at.level.block_state(host, at.pos)?;
        let entity = host.block_entity(at.level, at.pos);
        provider(host, at, state, entity)
    }) as Arc<dyn BlockCapabilityProvider<T>>)
}

/// Registrar operations for blocks.
pub trait BlocksRegistrar {
    /// Request a block by canonical registry id. All mods' declarations are checked at load.
    fn block_key(&mut self, id: &str) -> crate::BlockKey;

    /// Attach `part`, of kind `kind` (its type's name), to block `block`.
    fn attach_block_part(
        &mut self,
        block: crate::BlockKey,
        kind: &'static str,
        part: Arc<dyn BlockPart>,
    );

    /// Attach `part`, of kind `kind`, to every block in tag `tag`.
    fn attach_block_part_tag(&mut self, tag: &str, kind: &'static str, part: Arc<dyn BlockPart>);

    /// Answer block capability `name` for block `block` with `provider`, asked ahead of the
    /// game's own when `before_game`; mods reach it through `block_capability`.
    fn provide_block_capability(
        &mut self,
        name: &'static str,
        block: &crate::BlockKey,
        before_game: bool,
        provider: BlockProviderOf,
    );
}

/// CapabilityProviders operations for blocks.
// hidden: per-kind lookup implementation, exposed for generated provider bindings
#[doc(hidden)]
pub trait BlocksCapabilityProviders {
    /// The next enabled provider of block capability `name` at a position: those registered for
    /// block entity type `kind` standing there and for block `block`, as one list in
    /// [`LevelRef::capability`](crate::LevelRef::capability)'s order, with its mod and whether it
    /// is asked ahead of the game. `index` starts at zero and advances past every visited
    /// registration, including disabled ones.
    fn block_provider(
        &self,
        name: &str,
        kind: Option<crate::BlockEntityTypeKey>,
        block: Option<crate::BlockKey>,
        index: &mut u32,
    ) -> Option<(BlockProviderOf, &'static str, bool)>;
}

impl BlocksCapabilityProviders for InstalledProviders {
    fn block_provider(
        &self,
        name: &str,
        kind: Option<crate::BlockEntityTypeKey>,
        block: Option<crate::BlockKey>,
        index: &mut u32,
    ) -> Option<(BlockProviderOf, &'static str, bool)> {
        use crate::content::registry_keys::{block_entity_type_name, block_id};
        let keys = &installed()?.keys;
        let kind = kind.and_then(block_entity_type_name);
        let block = block.and_then(block_id);
        next_merged(
            providers_at(&keys.capabilities, name, kind),
            providers_at(&keys.block_capabilities, name, block),
            index,
        )
    }
}

/// `table`'s providers of capability `name` for `id`: none for no id.
pub(crate) fn providers_at<'a, P>(
    table: &'a Capabilities<P>,
    name: &str,
    id: Option<&str>,
) -> &'a [CapabilityPart<P>] {
    id.and_then(|id| table.get(name)?.get(id))
        .map_or(&[], Vec::as_slice)
}
