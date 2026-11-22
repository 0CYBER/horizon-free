//! Resolved tag names and the read-only merged-tag service.
use crate::{BlockKey, FluidKey, Identifier, ItemKey};

/// A process-lifetime tag name resolved once through [`Tags::key`].
/// Membership belongs to the current authoritative data reload, not to the key.
// no wire: decoding validates the u32 slot with TagKey::from_slot on the host; wasm carries the host-issued opaque token without consulting guest tables.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: the runtime alone creates name slots
pub struct TagKey(pub(crate) u32);

impl TagKey {
    #[cfg(any(not(target_arch = "wasm32"), test))]
    pub(crate) fn from_slot(slot: u32) -> Result<Self, &'static str> {
        horizon_mod_runtime::registry_keys::with_tag_name(slot, |id| {
            id.map(|_| Self(slot)).ok_or("unknown TagKey slot")
        })
    }
}

/// Read merged block, item and fluid tags without borrowing the game's registry structs.
/// Unknown tags answer false or an empty member list. A client reads the tags its server
/// sent. Membership reads answer `None` before authoritative tags are loaded, on a client
/// before its server's are applied, or while their key table is unavailable.
pub trait Tags {
    /// Resolve a validated name once. The name need not currently have members;
    /// retain its key across reloads. None if the runtime cannot allocate a name slot.
    fn key(&self, _id: &Identifier) -> Option<TagKey> {
        crate::world::phase::unprovided(
            "Tags::key",
            "tags_key",
            crate::world::phase::LoadAccess::Metadata,
        )
    }
    /// The name `tag` was resolved from (`TagKey.location()`), as `key` normalised it
    /// (`c:ingots/iron`, `minecraft:logs`). None for a key no name gave. Names outlive
    /// reloads, so this never depends on loaded tags.
    fn name(&self, _tag: TagKey) -> Option<Identifier> {
        crate::world::phase::unprovided(
            "Tags::name",
            "tags_name",
            crate::world::phase::LoadAccess::Metadata,
        )
    }
    /// Whether `item` is in the current item tag. An unknown tag is false;
    /// unavailable authoritative tags or item mappings answer None.
    fn is_in_tag(&self, _item: ItemKey, _tag: TagKey) -> Option<bool> {
        crate::world::phase::unprovided(
            "Tags::is_in_tag",
            "tags_is_in_tag",
            crate::world::phase::LoadAccess::Metadata,
        )
    }
    /// Item members in vanilla's order, including unrequested vanilla items.
    /// Unknown tags are empty; unavailable authoritative tags or item mappings answer None.
    fn items(&self, _tag: TagKey) -> Option<Vec<ItemKey>> {
        crate::world::phase::unprovided(
            "Tags::items",
            "tags_items",
            crate::world::phase::LoadAccess::Metadata,
        )
    }
    /// Fluid members in vanilla's order. Unknown tags are empty;
    /// unavailable authoritative tags or fluid mappings answer None.
    fn fluids(&self, _tag: TagKey) -> Option<Vec<FluidKey>> {
        crate::world::phase::unprovided(
            "Tags::fluids",
            "tags_fluids",
            crate::world::phase::LoadAccess::Metadata,
        )
    }
    /// Block members in vanilla's order, including unrequested vanilla blocks.
    /// Unknown tags are empty; unavailable authoritative tags or block mappings answer None.
    fn blocks(&self, _tag: TagKey) -> Option<Vec<BlockKey>> {
        crate::world::phase::unprovided(
            "Tags::blocks",
            "tags_blocks",
            crate::world::phase::LoadAccess::Metadata,
        )
    }
    /// The item tags `item` is in (vanilla's `Holder.tags()`, `ItemStack.getTags()`), by tag
    /// name, over the same merged tags `is_in_tag` reads: `is_in_tag(item, tag)` is true for
    /// exactly the tags listed. Empty for an item in no tag. None as for the member reads, and
    /// never a partial list. Each call walks every item tag, so a mod that asks often (a filter
    /// matching `c:ingots/*`) caches the answer until the next `tags_updated`.
    fn item_tags(&self, _item: ItemKey) -> Option<Vec<TagKey>> {
        crate::world::phase::unprovided(
            "Tags::item_tags",
            "tags_item_tags",
            crate::world::phase::LoadAccess::Metadata,
        )
    }
    /// The block tags `block` is in (`BlockState.getTags()`), by tag name, as `item_tags`.
    fn block_tags(&self, _block: BlockKey) -> Option<Vec<TagKey>> {
        crate::world::phase::unprovided(
            "Tags::block_tags",
            "tags_block_tags",
            crate::world::phase::LoadAccess::Metadata,
        )
    }
    /// The fluid tags `fluid` is in (`FluidState.getTags()`), by tag name, as `item_tags`.
    fn fluid_tags(&self, _fluid: FluidKey) -> Option<Vec<TagKey>> {
        crate::world::phase::unprovided(
            "Tags::fluid_tags",
            "tags_fluid_tags",
            crate::world::phase::LoadAccess::Metadata,
        )
    }
}

/// Resolve a tag name at the game boundary.
pub fn tag_key(id: &Identifier) -> Option<TagKey> {
    horizon_mod_runtime::registry_keys::tag_slot(id).map(TagKey)
}

/// Borrow a tag's canonical name within the runtime's read lock, without formatting or cloning.
pub fn with_tag_name<R>(key: TagKey, read: impl FnOnce(Option<&str>) -> R) -> R {
    horizon_mod_runtime::registry_keys::with_tag_name(key.0, read)
}
