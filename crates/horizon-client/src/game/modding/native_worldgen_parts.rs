// game-module-visibility: pub(crate)
//! What a server publishes for its world generation at each Parts install (world open and
//! `/reload`): its resolved Part table, which the worldgen Part methods read on the workers and on
//! the server thread alike, the block-state index their states convert through, with the
//! registry that index was built from, and its merged block tags.
use super::api_values::block_state::Index;
use horizon_world::registry::BlockRegistry;
use mod_api::BlockKey;
use std::collections::{HashMap, HashSet};
use std::sync::{Arc, PoisonError, RwLock};

/// The server's merged block tags, by tag id, as the live game's `block_in_tag` reads them.
pub(in crate::game) type BlockTags = HashMap<String, HashSet<BlockKey>>;

/// The published index, the registry it converts against and the block tags.
type Published = (Arc<Index>, &'static BlockRegistry, Arc<BlockTags>);

static INDEX: RwLock<Option<Published>> = RwLock::new(None);

/// Held by each test that publishes: the published tables are process-wide.
#[cfg(test)]
pub(in crate::game) static PUBLISHING: std::sync::Mutex<()> = std::sync::Mutex::new(());

/// Publish this server's Part table, resolved against its tags, its block registry's index (the
/// worker resource's own) and its merged block tags for the features generated from now on.
pub(in crate::game) fn publish(
    parts: Option<&mod_api::Parts>,
    index: &Arc<Index>,
    registry: &'static BlockRegistry,
    tags: Arc<BlockTags>,
) {
    *INDEX.write().unwrap_or_else(PoisonError::into_inner) =
        Some((index.clone(), registry, tags));
    mod_api::__private::publish_worldgen_parts(parts.cloned());
}

/// The index the server published last; None before a server published one.
pub(in crate::game) fn index() -> Option<Arc<Index>> {
    published().map(|(index, _, _)| index)
}

fn published() -> Option<Published> {
    INDEX.read().unwrap_or_else(PoisonError::into_inner).clone()
}

/// Runs `f` with the server's worldgen context as a mod reads it: its registries, settings,
/// `state_with_property` and `block_in_tag`, which answer None before the server published its
/// index and tags. None before a server published its context.
pub(in crate::game) fn with_host<R>(
    f: impl FnOnce(&mod_api::OwningWorldgen<'_>) -> R,
) -> Option<R> {
    let context = horizon_world::native_worldgen_hooks::context()?;
    let published = published();
    let registries = |name: &str| context.registry(name);
    let setting = |mod_id: &str, key: &str| context.setting(mod_id, key);
    let state_with_property = |state: mod_api::BlockState, key: &str, value: &str| {
        let (index, registry, _) = published.as_ref()?;
        let engine = index.to_engine(state).ok()?;
        let set = super::native_hooks::with_property(registry, engine, key, value)?;
        index.to_api(set).ok()
    };
    let block_in_tag = |block: BlockKey, tag: &str| block_in_tag(block, tag);
    Some(f(&mod_api::OwningWorldgen::new(
        &registries,
        &setting,
        &state_with_property,
    )
    .with_block_in_tag(&block_in_tag)))
}

/// `BlockState.is(TagKey)` over the merged block tags the server published last: whether
/// `block` is in `tag` (a tag without a namespace is vanilla's). An unknown tag is false; None
/// before a server published its tags.
pub(in crate::game) fn block_in_tag(block: BlockKey, tag: &str) -> Option<bool> {
    let (_, _, tags) = published()?;
    let tag = horizon_client::command::args::normalized(tag);
    Some(tags.get(tag.as_ref()).is_some_and(|set| set.contains(&block)))
}

/// The engine's feature callback: one feature's Part lookups read the tables published when it
/// started.
pub(crate) fn feature(place: &mut dyn FnMut()) {
    mod_api::__private::worldgen_feature(place);
}
