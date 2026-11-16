//! Effective configured-carver registries for biome modifiers.
use super::{J, Result, SavedModifiers, biomes::Registry};
use horizon_world::worldgen::overrides::WorldgenOverrides;

pub(super) fn expand(doc: &mut J, registry: &Registry<'_>) -> Result<()> {
    if let Some(J::Str(tag)) = doc.get("carvers")
        && tag.starts_with('#')
    {
        let selector = serde_json::Value::String(tag.clone());
        let ids = registry.resolve(&selector)?;
        doc.set("carvers", J::Arr(ids.into_iter().map(J::Str).collect()));
    }
    Ok(())
}

pub(super) fn refresh(saved: &mut SavedModifiers, packs: &WorldgenOverrides) -> Result<()> {
    saved
        .carver_ids
        .extend(packs.entries("carvers").map(|(id, _)| id.to_owned()));
    for (id, _, text) in packs.tags("carver_tags") {
        let tag: J = serde_json::from_str(text)?;
        let (values, remove) = (
            super::tag_list(&tag, "values"),
            super::tag_list(&tag, "remove"),
        );
        let old = if matches!(tag.get("replace"), Some(J::Bool(true))) {
            None
        } else {
            saved.carver_tags.get(id).map(String::as_str)
        };
        let layers = [super::TagLayers { values, remove }];
        let merged = super::merge_tag(
            old,
            &layers,
            id,
            &horizon_world::conditions::LoadConditions::new(
                saved.enabled_mods.iter().map(String::as_str),
            ),
        )?;
        saved.carver_tags.insert(id.to_owned(), merged);
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn existing_tag_expands_to_effective_carvers() {
        let ids = ["test:cave".to_owned()].into_iter().collect();
        let tags = [(
            "test:all".to_owned(),
            r#"{"values":["test:cave"]}"#.to_owned(),
        )]
        .into_iter()
        .collect();
        let registry = Registry {
            what: "carver",
            ids: &ids,
            tags: &tags,
        };
        let mut doc: J = serde_json::from_str(r##"{"carvers":"#test:all"}"##).unwrap();
        expand(&mut doc, &registry).unwrap();
        assert_eq!(
            serde_json::to_string(doc.get("carvers").unwrap()).unwrap(),
            r#"["test:cave"]"#
        );
    }
    #[test]
    fn pack_carvers_and_replacement_tags_refresh() {
        let mut saved: SavedModifiers = serde_json::from_str(r#"{"enabled_mods":[],"modifiers":[],"unmodified":{},"feature_tags":{},"entity_categories":{},"entity_tags":{},"carver_tags":{"test:all":"{\"values\":[\"test:old\"]}"}}"#).unwrap();
        let mut packs = WorldgenOverrides::default();
        packs.set("carvers", "test:new", "{}".into());
        packs.add_tag(
            "carver_tags",
            "test:all",
            "test",
            r#"{"replace":true,"values":["test:new"]}"#.into(),
        );
        refresh(&mut saved, &packs).unwrap();
        let registry = Registry {
            what: "carver",
            ids: &saved.carver_ids,
            tags: &saved.carver_tags,
        };
        assert_eq!(
            registry.resolve(&serde_json::json!("#test:all")).unwrap(),
            ["test:new"]
        );
    }
    #[test]
    fn a_pack_tags_remove_list_takes_carvers_out() {
        let mut saved: SavedModifiers = serde_json::from_str(r##"{"enabled_mods":[],"modifiers":[],"unmodified":{},"feature_tags":{},"entity_categories":{},"entity_tags":{},"carver_tags":{"test:all":"{\"values\":[\"test:cave\",\"test:canyon\",\"#test:wet\"]}","test:wet":"{\"values\":[\"test:sea\"]}"}}"##).unwrap();
        saved
            .carver_ids
            .extend(["test:cave", "test:canyon", "test:sea"].map(String::from));
        let mut packs = WorldgenOverrides::default();
        packs.add_tag(
            "carver_tags",
            "test:all",
            "test",
            r##"{"values":[],"remove":["test:canyon","#test:wet","#test:absent"]}"##.into(),
        );
        refresh(&mut saved, &packs).unwrap();
        let registry = Registry {
            what: "carver",
            ids: &saved.carver_ids,
            tags: &saved.carver_tags,
        };
        assert_eq!(
            registry.resolve(&serde_json::json!("#test:all")).unwrap(),
            ["test:cave"]
        );
    }
    #[test]
    fn pack_carvers_without_modifiers_leave_world_data_unchanged() {
        let root = std::env::temp_dir().join(format!("carver-invariance-{}", std::process::id()));
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(root.join("biome-modifiers.json"), r#"{"enabled_mods":[],"modifiers":[],"unmodified":{},"feature_tags":{},"entity_categories":{},"entity_tags":{}}"#).unwrap();
        let mut packs = WorldgenOverrides::default();
        packs.set("carvers", "test:new", "{}".into());
        packs.set("biomes", "test:plains", r#"{"carvers":"test:new"}"#.into());
        let result = super::super::biome_modifiers_over_packs(&root, &packs, &[])
            .unwrap()
            .unwrap();
        assert_eq!(result, packs);
        std::fs::remove_dir_all(root).unwrap();
    }
}
