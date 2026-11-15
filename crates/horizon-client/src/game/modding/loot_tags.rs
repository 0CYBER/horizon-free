//! Adapt the current loot evaluator's tag reads to the existing API resource.
use mod_api::{BlockKey, FluidKey, Identifier, ItemKey, TagKey, Tags};

pub(crate) struct LootTags<'a>(pub &'a dyn horizon_world::loot::LootTags);

impl Tags for LootTags<'_> {
    fn key(&self, id: &Identifier) -> Option<TagKey> {
        mod_api::__private::tag_key(id)
    }
    fn is_in_tag(&self, item: ItemKey, tag: TagKey) -> Option<bool> {
        let item = mod_api::__private::item_key_id(item)?;
        mod_api::__private::with_tag_name(tag, |tag| Some(self.0.contains("item", tag?, item)))
    }
    fn items(&self, tag: TagKey) -> Option<Vec<ItemKey>> {
        mod_api::__private::with_tag_name(tag, |tag| {
            self.0
                .members("item", tag?)
                .iter()
                .map(|id| mod_api::__private::item_key(id))
                .collect()
        })
    }
    fn blocks(&self, tag: TagKey) -> Option<Vec<BlockKey>> {
        mod_api::__private::with_tag_name(tag, |tag| {
            self.0
                .members("block", tag?)
                .iter()
                .map(|id| mod_api::__private::block_key(id))
                .collect()
        })
    }
    fn fluids(&self, tag: TagKey) -> Option<Vec<FluidKey>> {
        mod_api::__private::with_tag_name(tag, |tag| {
            self.0
                .members("fluid", tag?)
                .iter()
                .map(|id| mod_api::__private::fluid_key(id))
                .collect()
        })
    }
}
