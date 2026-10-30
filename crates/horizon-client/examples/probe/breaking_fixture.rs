//! Headless transport for real model-shaped breaking overlays.
use horizon_client::{
    assets::Assets,
    blocks::{BlockFacts, RenderShape},
    breaking,
    mesher::MeshTables,
    models,
};
use horizon_core::position::BlockPos;
use horizon_world::registry::BlockStateId;
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufRead, BufReader, BufWriter, Write},
    path::Path,
};
/// The stage sprite origin the cracks carry (the oracle compares the geometry and UVs only).
const STAGE: [f32; 2] = [0.25, 0.5];
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = crate::args().collect();
    let root = Path::new(&args[1]);
    let assets = Assets::load(root)?;
    let render = assets.reload(root)?;
    let facts = BlockFacts::load(root, assets.registry)?;
    let tables = MeshTables::new(
        &assets,
        &render,
        &facts,
        models::load_visibility(root, assets.registry)?,
    )?;
    let mut output = BufWriter::new(fs::File::create(&args[3])?);
    for line in BufReader::new(fs::File::open(&args[2])?).lines() {
        let row: Value = serde_json::from_str(&line?)?;
        let state = BlockStateId(row["state"].as_u64().unwrap() as u32);
        let p: [i32; 3] = serde_json::from_value(row["pos"].clone())?;
        let camera: [f64; 3] = serde_json::from_value(row["camera"].clone())?;
        let mut vertices = Vec::new();
        let mut translucent = false;
        if facts.get(state).render_shape == RenderShape::Model {
            let models = render
                .model_catalog
                .collect(state, BlockPos::new(p[0], p[1], p[2]))?;
            let offset = tables.offset(state, p[0], p[2]).unwrap_or([0.0; 3]);
            let origin = (glam::DVec3::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2]))
                - glam::DVec3::from(camera))
            .as_vec3()
                + glam::DVec3::from(offset).as_vec3();
            translucent = models
                .iter()
                .any(|(_, m)| m.faces.iter().any(|f| f.translucent));
            for (_, m) in models {
                breaking::geometry(m, origin, translucent, STAGE, &mut vertices);
            }
        }
        let quads: Vec<_> = vertices
            .as_chunks::<4>()
            .0
            .iter()
            .map(|q| {
                q.iter()
                    .map(|v| {
                        assert_eq!(v.color, 0xffff_ffff);
                        assert_eq!(v.light, breaking::stage_light(STAGE));
                        assert_eq!(v.layer, if translucent { 12 } else { 11 });
                        [v.pos[0], v.pos[1], v.pos[2], v.uv[0], v.uv[1]]
                    })
                    .collect::<Vec<_>>()
            })
            .collect();
        serde_json::to_writer(
            &mut output,
            &json!({"case":row["case"],"translucent":translucent,"quads":quads}),
        )?;
        output.write_all(&[10])?;
    }
    output.flush()?;
    Ok(())
}
