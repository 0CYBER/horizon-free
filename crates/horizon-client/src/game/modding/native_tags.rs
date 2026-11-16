//! One membership implementation for full-game and in-write tag services.
use horizon_items::items::Items;
use mod_api::{BlockKey, FluidKey, ItemKey, TagKey};

pub(super) fn is_in_tag(items: Option<&Items>, item: ItemKey, tag: TagKey) -> Option<bool> {
    let items = items?;
    let item = mod_api::__private::item_key_id(item)?;
    mod_api::__private::with_tag_name(tag, |name| {
        Some(
            items
                .item_tags
                .get(name?)
                .is_some_and(|members| members.contains(item)),
        )
    })
}
pub(super) fn items(items: Option<&Items>, tag: TagKey) -> Option<Vec<ItemKey>> {
    let items = items?;
    mod_api::__private::with_tag_name(tag, |name| {
        items.item_tag_order.get(name?).map_or_else(
            || Some(Vec::new()),
            |members| {
                members
                    .iter()
                    .map(|id| mod_api::__private::item_key(id))
                    .collect()
            },
        )
    })
}
pub(super) fn fluids(items: Option<&Items>, tag: TagKey) -> Option<Vec<FluidKey>> {
    let items = items?;
    mod_api::__private::with_tag_name(tag, |name| {
        items.fluid_tag_order.get(name?).map_or_else(
            || Some(Vec::new()),
            |members| {
                members
                    .iter()
                    .map(|id| mod_api::__private::fluid_key(id))
                    .collect()
            },
        )
    })
}

pub(super) fn blocks(items: Option<&Items>, tag: TagKey) -> Option<Vec<BlockKey>> {
    let items = items?;
    mod_api::__private::with_tag_name(tag, |name| {
        items.block_tag_order.get(name?).map_or_else(
            || Some(Vec::new()),
            |members| {
                members
                    .iter()
                    .map(|id| mod_api::__private::block_key(id))
                    .collect()
            },
        )
    })
}

// A restricted host supplies the same authoritative item tables but cannot borrow Game.
// Only that choice differs; key resolution, ordered reads and membership stay above.
macro_rules! impl_tags {
    ($ty:ty, $this:ident, $items:expr) => {
        impl mod_api::Tags for $ty {
            fn key(&self, id: &mod_api::Identifier) -> Option<mod_api::TagKey> {
                mod_api::__private::tag_key(id)
            }
            fn is_in_tag(&self, item: mod_api::ItemKey, tag: mod_api::TagKey) -> Option<bool> {
                let $this = self;
                $crate::game::native_tags::is_in_tag($items, item, tag)
            }
            fn items(&self, tag: mod_api::TagKey) -> Option<Vec<mod_api::ItemKey>> {
                let $this = self;
                $crate::game::native_tags::items($items, tag)
            }
            fn fluids(&self, tag: mod_api::TagKey) -> Option<Vec<mod_api::FluidKey>> {
                let $this = self;
                $crate::game::native_tags::fluids($items, tag)
            }
            fn blocks(&self, tag: mod_api::TagKey) -> Option<Vec<mod_api::BlockKey>> {
                let $this = self;
                $crate::game::native_tags::blocks($items, tag)
            }
        }
    };
}
pub(super) use impl_tags;
impl_tags!(super::Game<'_>, game, game.native_tag_items());

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn block_members_preserve_authoritative_order_reload_and_atomic_refusal() {
        crate::game::api_values::install_parts(&[]).unwrap();
        mod_api::__private::resolve_block_keys(
            ["minecraft:air", "minecraft:stone", "minecraft:birch_log"],
            |_| {
                let properties = std::collections::BTreeMap::new();
                horizon_mod_runtime::block_definitions::BlockDefinition::new(&[&properties], 0)
                    .map_err(|error| format!("{error:?}"))
            },
        )
        .unwrap();
        let stone = mod_api::__private::block_key("minecraft:stone").unwrap();
        let birch = mod_api::__private::block_key("minecraft:birch_log").unwrap();
        let tag = mod_api::__private::tag_key(&mod_api::Identifier::parse("test:blocks").unwrap())
            .unwrap();
        let missing =
            mod_api::__private::tag_key(&mod_api::Identifier::parse("test:empty").unwrap())
                .unwrap();
        let mut table = Items::for_tests(Default::default());
        table.block_tag_order.insert(
            "test:blocks".into(),
            vec!["minecraft:stone".into(), "minecraft:birch_log".into()],
        );
        struct TagView<'a>(Option<&'a Items>);
        impl_tags!(TagView<'_>, view, view.0);
        assert_eq!(
            mod_api::Tags::blocks(&TagView(Some(&table)), tag),
            Some(vec![stone, birch])
        );
        assert_eq!(
            mod_api::Tags::blocks(&TagView(Some(&table)), missing),
            Some(vec![])
        );
        assert_eq!(mod_api::Tags::blocks(&TagView(None), tag), None);
        table
            .block_tag_order
            .insert("test:blocks".into(), vec!["minecraft:birch_log".into()]);
        assert_eq!(blocks(Some(&table), tag), Some(vec![birch]));
        table.block_tag_order.insert(
            "test:blocks".into(),
            vec!["minecraft:stone".into(), "test:unloaded".into()],
        );
        assert_eq!(
            blocks(Some(&table), tag),
            None,
            "no partial member list on unresolved block mapping"
        );
    }

    #[test]
    fn empty_stacks_refuse_before_membership_and_air_is_an_explicit_key() {
        use crate::game::api_values::{StackReadError, TryIntoApi};
        crate::game::api_values::install_parts(&[]).unwrap();
        mod_api::__private::resolve_items(["minecraft:air", "minecraft:stone"]).unwrap();
        let air = mod_api::__private::item_key("minecraft:air").unwrap();
        let stone = mod_api::__private::item_key("minecraft:stone").unwrap();
        let tag = mod_api::__private::tag_key(&mod_api::Identifier::parse("x").unwrap()).unwrap();
        let missing =
            mod_api::__private::tag_key(&mod_api::Identifier::parse("minecraft:y").unwrap())
                .unwrap();
        let mut table = Items::for_tests(
            [(
                "minecraft:x".to_owned(),
                ["minecraft:air".to_owned()].into(),
            )]
            .into(),
        );
        let empty = horizon_client::items::ItemStack::new("minecraft:stone", 0);
        assert_eq!(empty.try_into_api(), Err(StackReadError::Empty));
        assert_eq!(is_in_tag(Some(&table), air, tag), Some(true));
        assert_eq!(is_in_tag(Some(&table), stone, tag), Some(false));
        assert_eq!(is_in_tag(Some(&table), air, missing), Some(false));
        table.item_tag_order.insert(
            "minecraft:x".into(),
            vec!["minecraft:air".into(), "missing:unloaded".into()],
        );
        assert_eq!(
            items(Some(&table), tag),
            None,
            "unresolved members must not produce a partial list"
        );
    }
    #[test]
    fn keys_survive_membership_reload_and_reads_distinguish_unavailable_from_unknown() {
        crate::game::api_values::install_parts(&[]).unwrap();
        mod_api::__private::resolve_items(["minecraft:stick", "minecraft:stone"]).unwrap();
        let stick = mod_api::__private::item_key("minecraft:stick").unwrap();
        let stone = mod_api::__private::item_key("minecraft:stone").unwrap();
        let tag = mod_api::__private::tag_key(&mod_api::Identifier::parse("test:reload").unwrap())
            .unwrap();
        let missing =
            mod_api::__private::tag_key(&mod_api::Identifier::parse("test:missing").unwrap())
                .unwrap();
        let mut table = Items::for_tests(Default::default());
        table
            .item_tags
            .insert("test:reload".into(), ["minecraft:stick".into()].into());
        table
            .item_tag_order
            .insert("test:reload".into(), vec!["minecraft:stick".into()]);
        assert_eq!(is_in_tag(Some(&table), stick, tag), Some(true));
        assert_eq!(items(Some(&table), tag), Some(vec![stick]));
        table
            .item_tags
            .insert("test:reload".into(), ["minecraft:stone".into()].into());
        table
            .item_tag_order
            .insert("test:reload".into(), vec!["minecraft:stone".into()]);
        assert_eq!(is_in_tag(Some(&table), stick, tag), Some(false));
        assert_eq!(items(Some(&table), tag), Some(vec![stone]));
        assert_eq!(is_in_tag(Some(&table), stone, missing), Some(false));
        assert_eq!(items(Some(&table), missing), Some(vec![]));
        assert_eq!(is_in_tag(None, stone, tag), None);
        assert_eq!(items(None, tag), None);
    }
}
