/// Saved preset/editor stems fill only levels that mods and enabled packs did not define.
pub fn merge_saved_world_gen_dimensions(
    world: &Path,
    overrides: &mut horizon_world::worldgen::overrides::WorldgenOverrides,
) {
    use horizon_world::worldgen::overrides::DIMENSIONS;
    let Some(Tag::Compound(dimensions)) =
        world_gen_settings(world).and_then(|settings| settings.get("dimensions").cloned())
    else {
        return;
    };
    for (id, stem) in dimensions {
        let id = id.to_string_lossy();
        if valid_dimension_key(&id) && !overrides.entries(DIMENSIONS).any(|(level, _)| level == id)
        {
            overrides.set(
                DIMENSIONS,
                &id,
                crate::scoreboard::tag_to_json(&stem).to_string(),
            );
        }
    }
}
