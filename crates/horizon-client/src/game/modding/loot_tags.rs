//! Adapt the current loot evaluator's tag reads to the existing API resource.
use mod_api::{BlockKey, FluidKey, Identifier, ItemKey, TagKey, Tags};

pub(crate) struct LootTags<'a>(pub &'a dyn horizon_world::loot::LootTags);

impl LootTags<'_> {
    /// `Holder.tags()` over the loot evaluator's tags: every tag of `registry` that holds
    /// `member`, sorted by name, as the game's own reverse reads. None when the evaluator can't
    /// list its tags or a name can't get a key: never a partial list.
    fn tags_holding(&self, registry: &str, member: &str) -> Option<Vec<TagKey>> {
        let mut names = self.0.tag_names(registry)?;
        names.retain(|name| self.0.contains(registry, name, member));
        names.sort_unstable();
        names
            .iter()
            .map(|name| mod_api::__private::tag_key(&Identifier::parse(name).ok()?))
            .collect()
    }
}

impl Tags for LootTags<'_> {
    fn key(&self, id: &Identifier) -> Option<TagKey> {
        mod_api::__private::tag_key(id)
    }
    fn name(&self, tag: TagKey) -> Option<Identifier> {
        crate::game::native_tags::name(tag)
    }
    fn is_in_tag(&self, item: ItemKey, tag: TagKey) -> Option<bool> {
        let item = mod_api::__private::item_key_id(item)?;
        mod_api::__private::with_tag_name(tag, |tag| Some(self.0.contains("item", tag?, item)))
    }
    fn item_tags(&self, item: ItemKey) -> Option<Vec<TagKey>> {
        self.tags_holding("item", mod_api::__private::item_key_id(item)?)
    }
    fn block_tags(&self, block: BlockKey) -> Option<Vec<TagKey>> {
        self.tags_holding("block", mod_api::__private::block_key_id(block)?)
    }
    fn fluid_tags(&self, fluid: FluidKey) -> Option<Vec<TagKey>> {
        self.tags_holding("fluid", mod_api::__private::fluid_id(fluid)?)
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
