//! Block-model tints without a level, including blank and invalid tint indices.
use horizon_client::{
    assets::Assets, blocks::BlockFacts, item_render::ItemRenderer, mesher::MeshTables, models,
};
use horizon_world::registry::BlockStateId;
use serde_json::Value;
use std::{
    fs,
    io::{BufWriter, Write},
    path::Path,
};

pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = crate::args().collect();
    let input: Value = serde_json::from_slice(&fs::read(&args[1])?)?;
    let root = Path::new(input["resources"].as_str().unwrap());
    let assets = Assets::load(root)?;
    let render = assets.reload(&horizon_assets::resource_source::ResourceRoot::directory(
        root.to_path_buf(),
    ))?;
    let facts = BlockFacts::load(root, assets.registry)?;
    let tables = MeshTables::new(
        &assets,
        &render,
        &facts,
        models::load_visibility(root, assets.registry)?,
        None,
    )?;
    let item_data = crate::test_resources::item_data(
        root,
        horizon_world::native_load_context::DataLoadOrigin::ClientPreload,
    )?;
    let grass = ItemRenderer::load(
        &horizon_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
        &item_data,
    )?
    .grass_color(0.5, 1.0) as u32;
    let mut output = BufWriter::new(fs::File::create(&args[2])?);
    for row in input["cases"].as_array().unwrap() {
        let state = row[0].as_u64().unwrap() as u32;
        let index = row[1].as_i64().unwrap() as i32;
        let color = tables
            .default_tint(BlockStateId(state), index, grass)
            .map_or(-1, |rgb| (0xFF00_0000 | rgb) as i32);
        writeln!(output, "{state} {index} {color}")?;
    }
    Ok(())
}
