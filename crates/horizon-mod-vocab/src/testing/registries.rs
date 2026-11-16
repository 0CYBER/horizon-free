//! The registries fake: item reports, items with slots of their own, and the loaded mods.
use crate::{ItemStack, Tag};
use std::collections::{HashMap, HashSet};

// no wire: in-process testing harness state, not a mod boundary payload.
/// The registries a test reads: each item's default components (its item report), the items
/// that hold slots of their own, and the mods loaded beside the one under test.
#[derive(Clone, Debug, Default)]
#[non_exhaustive]
pub struct FakeRegistries {
    components: HashMap<(String, String), Tag>,
    containers: HashMap<String, usize>,
    loaded: HashSet<String>,
    fluid_sounds: HashMap<String, (&'static str, &'static str)>,
}

impl FakeRegistries {
    /// Complete the current load's item table before creating stacks. Call once after part
    /// registration, with every fake item id. Without registered parts, starts an empty load.
    pub fn resolve_items(
        &mut self,
        ids: impl IntoIterator<Item = crate::Identifier>,
    ) -> Result<(), Vec<super::ItemKeyError>> {
        if crate::parts::installed().is_none() {
            crate::parts::install_input(
                crate::__private::parts_installation::InstalledInput::default(),
            );
        }
        let ids: Vec<_> = ids.into_iter().map(|id| id.to_string()).collect();
        crate::content::registry_keys::resolve_items(ids.iter().map(String::as_str))
    }
    /// Complete the current load's fluid table with `ids`, as the game does once the fluid
    /// registry loads, so `host.fluid_key` and stacks of them work.
    pub fn resolve_fluids<'a>(
        &mut self,
        ids: impl IntoIterator<Item = &'a str>,
    ) -> Result<(), Vec<crate::UnknownFluid>> {
        if crate::parts::installed().is_none() {
            crate::parts::install_input(
                crate::__private::parts_installation::InstalledInput::default(),
            );
        }
        let ids: Vec<_> = ids.into_iter().collect();
        crate::content::registry_keys::resolve_fluids(&ids, &[])
    }
    pub(super) fn resolved_item_id(key: crate::ItemKey) -> Option<&'static str> {
        crate::parts::installed()?.resolved_items.get()?.id(key.0)
    }

    /// Gives item `item` the default `component` `value`, as its item report would.
    pub fn item_component(&mut self, item: &str, component: &str, value: Tag) -> &mut Self {
        self.components
            .insert((item.to_owned(), component.to_owned()), value);
        self
    }
    /// Gives item `item` `slots` slots of the game's own, as `item_container` answers for a
    /// mod's backpack; they read empty.
    pub fn item_slots(&mut self, item: &str, slots: usize) -> &mut Self {
        self.containers.insert(item.to_owned(), slots);
        self
    }
    /// Gives fluid `fluid` the sounds a bucket plays as it fills with it (`pickup`) and empties
    /// it (`empty`), as `fluid_pickup_sound` and `fluid_empty_sound` answer; a fluid not given
    /// any has none, as a tank-only fluid has none in the game.
    pub fn fluid_sounds(
        &mut self,
        fluid: &str,
        pickup: &'static str,
        empty: &'static str,
    ) -> &mut Self {
        self.fluid_sounds.insert(fluid.to_owned(), (pickup, empty));
        self
    }
    /// Marks mod `id` as loaded.
    pub fn loaded(&mut self, id: &str) -> &mut Self {
        self.loaded.insert(id.to_owned());
        self
    }

    /// Item `item`'s default `component`. Every item's report has `minecraft:max_stack_size`,
    /// 64 unless given.
    pub(super) fn default_component(&self, item: &str, component: &str) -> Option<Tag> {
        let given = self
            .components
            .get(&(item.to_owned(), component.to_owned()))
            .cloned();
        given.or_else(|| (component == "minecraft:max_stack_size").then_some(Tag::Int(64)))
    }
    /// How many of item `item` one stack holds.
    pub(super) fn max_stack_size(&self, item: &str) -> u32 {
        match self.default_component(item, "minecraft:max_stack_size") {
            Some(Tag::Int(size)) => u32::try_from(size).unwrap_or(0),
            _ => 64,
        }
    }
    /// The slots `stack` holds, when its item has any.
    pub(super) fn container_slots(&self, stack: &ItemStack) -> Option<Vec<Option<ItemStack>>> {
        self.containers
            .get(Self::resolved_item_id(stack.item())?)
            .map(|&slots| vec![None; slots])
    }
    /// The sound a bucket plays filling with `fluid` (`pickup`) or emptying it, if given.
    pub(super) fn fluid_sound(&self, fluid: &str, pickup: bool) -> Option<&'static str> {
        let (fill, empty) = self.fluid_sounds.get(fluid)?;
        Some(if pickup { fill } else { empty })
    }
    pub(super) fn is_loaded(&self, id: &str) -> bool {
        self.loaded.contains(id)
    }
}

// A fake has an authoritative empty tag registry; it never invents members.
impl crate::Tags for FakeRegistries {
    fn key(&self, id: &crate::Identifier) -> Option<crate::TagKey> {
        crate::world::tags::tag_key(id)
    }
    fn is_in_tag(&self, item: crate::ItemKey, _: crate::TagKey) -> Option<bool> {
        Self::resolved_item_id(item)?;
        Some(false)
    }
    fn items(&self, _: crate::TagKey) -> Option<Vec<crate::ItemKey>> {
        crate::parts::installed()?.resolved_items.get()?;
        Some(Vec::new())
    }
    fn fluids(&self, _: crate::TagKey) -> Option<Vec<crate::FluidKey>> {
        crate::parts::resolved_fluids().get()?;
        Some(Vec::new())
    }
    fn blocks(&self, _: crate::TagKey) -> Option<Vec<crate::BlockKey>> {
        crate::parts::installed()?.resolved_blocks.get()?;
        Some(Vec::new())
    }
}
