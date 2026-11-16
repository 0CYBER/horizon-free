//! `BannerBlockEntity.loadAdditional`: the `CustomName` is a component and a
//! layer's `pattern` is a `Holder<BannerPattern>`, which may be a direct holder
//! (`{asset_id, translation_key}`) instead of a registry reference.
use super::super::spill::Pending;
use super::map_banner;
use horizon_client::{banner, containers::BlockEntity, items::ItemStack, save};
use horizon_core::nbt::Tag;

fn banner_tag(extra: Vec<(&str, Tag)>) -> Tag {
    let mut entries = vec![
        ("id", Tag::String("minecraft:banner".into())),
        ("x", Tag::Int(1)),
        ("y", Tag::Int(2)),
        ("z", Tag::Int(3)),
    ];
    entries.extend(extra);
    Tag::compound(entries)
}

/// `ComponentSerialization.CODEC` reads a component object as well as a string,
/// so a banner's `CustomName` keeps its style through the block entity, the
/// `data get block` tag, the map marker's plain text and the dropped item.
#[test]
fn a_banner_keeps_a_component_name() {
    let name = Tag::compound([
        ("text", Tag::String("Flag".into())),
        ("color", Tag::String("red".into())),
    ]);
    let tag = banner_tag(vec![("CustomName", name.clone())]);
    let (pos, entity) = save::decode_block_entity(&tag).expect("banner");
    // `BannerBlockEntity.saveAdditional` -> `BlockEntity.saveWithFullMetadata`.
    let saved = save::block_entity_data(pos, &entity, "minecraft:white_banner", None);
    assert_eq!(
        saved.get("CustomName"),
        Some(&name),
        "the name keeps its style"
    );
    // `MapBanner.fromWorld` -> `MapBanner.getName`: the marker's plain text.
    let marker = map_banner([1, 2, 3], Some(&entity), "minecraft:white_banner").expect("marker");
    assert_eq!(marker.name.as_deref(), Some("Flag"));
    // The banner loot table's `copy_components`: `minecraft:custom_name`.
    let mut pending = Pending::default();
    pending.record([1, 2, 3], &entity, "minecraft:white_banner", None, None);
    let mut dropped = ItemStack::new("minecraft:white_banner", 1);
    pending.attach([1, 2, 3], &mut dropped);
    assert_eq!(
        dropped.data().and_then(|d| d.custom_name.clone()),
        Some(name),
        "the dropped banner keeps the styled name"
    );
}

/// `BannerPattern.CODEC` is `RegistryCodecs.holder(..., DIRECT_CODEC)` with
/// inline definitions allowed, so `{asset_id, translation_key}` is a direct
/// holder: a layer of its own, with its own texture and tooltip key.
#[test]
fn a_banner_keeps_an_inline_pattern() {
    let pattern = Tag::compound([
        ("asset_id", Tag::String("minecraft:brick".into())),
        (
            "translation_key",
            Tag::String("block.minecraft.banner.brick".into()),
        ),
    ]);
    let patterns = Tag::List(vec![Tag::compound([
        ("pattern", pattern.clone()),
        ("color", Tag::String("red".into())),
    ])]);
    let tag = banner_tag(vec![("patterns", patterns.clone())]);
    let (pos, entity) = save::decode_block_entity(&tag).expect("banner");
    let BlockEntity::Banner(banner) = &entity else {
        panic!("not a banner");
    };
    assert_eq!(banner.layers.len(), 1, "the inline pattern is a layer");
    // `Holder`'s encode: a direct holder writes its definition back.
    let saved = save::block_entity_data(pos, &entity, "minecraft:white_banner", None);
    assert_eq!(saved.get("patterns"), Some(&patterns));
    // `Sheets.getBannerSprite` / `Layer.description` read the definition.
    let banners = banner::Banners::default();
    assert_eq!(
        banners.layer_texture(&banner.layers[0]),
        "entity/banner/brick"
    );
    assert_eq!(
        banners.layer_key(&banner.layers[0]),
        "block.minecraft.banner.brick.red"
    );
    // A direct holder needs both fields; a bad layer leaves the valid one.
    let broken = Tag::List(vec![
        Tag::compound([("pattern", pattern), ("color", Tag::String("red".into()))]),
        Tag::compound([
            (
                "pattern",
                Tag::compound([("asset_id", Tag::String("minecraft:brick".into()))]),
            ),
            ("color", Tag::String("blue".into())),
        ]),
    ]);
    let tag = banner_tag(vec![("patterns", broken)]);
    let (_, entity) = save::decode_block_entity(&tag).expect("banner");
    assert!(
        matches!(&entity, BlockEntity::Banner(b) if b.layers == banner.layers),
        "a layer without a translation key is dropped"
    );
}
