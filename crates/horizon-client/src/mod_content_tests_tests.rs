use super::*;

/// Mods merge into a tag in load order, as data packs do: each adds its
/// values, and one that replaces drops what came before it, with a warning
/// naming what it dropped.
#[test]
fn tags_merge_in_load_order_and_replace_warns() {
    let v = |ids: &[&str]| -> Vec<J> { ids.iter().map(|i| J::Str((*i).into())).collect() };
    let file = "data/minecraft/tags/worldgen/biome/has_structure/village_plains.json";
    let mut merge = TagMerge::default();
    assert_eq!(merge.add("a", file, false, true, &v(&["a:x"])), None);
    let warning = merge.add("b", file, true, true, &v(&["minecraft:plains"]));
    assert_eq!(
        warning.as_deref(),
        Some(
            "mod b replaces data/minecraft/tags/worldgen/biome/has_structure/village_plains.json: \
             vanilla's entries and those of a are left out, as a data pack's replace does; \
             the mods after it in load order still add to it"
        )
    );
    assert_eq!(merge.add("c", file, false, true, &v(&["c:y"])), None);
    assert!(merge.replaced);
    let members = |merge: &TagMerge| {
        let text = merge_tag(
            None,
            &merge.layers(),
            file,
            &horizon_world::conditions::LoadConditions::vanilla(),
        )
        .unwrap();
        let doc = serde_json::from_str(&text).unwrap();
        horizon_world::tag_layers::TagLayers::from_file(&doc).build(|_| Vec::new())
    };
    assert_eq!(members(&merge), ["minecraft:plains", "c:y"]);
    // A second replace drops only the mods' entries.
    let warning = merge.add("d", file, true, true, &v(&["d:z"])).unwrap();
    assert!(
        warning.contains(": those of b, c are left out"),
        "{warning}"
    );
    assert_eq!(members(&merge), ["d:z"]);
    // A mod replacing a tag of its own that nobody added to drops nothing.
    let own = "data/a/tags/item/gems.json";
    assert_eq!(
        TagMerge::default().add("a", own, true, false, &v(&["a:x"])),
        None
    );
}
