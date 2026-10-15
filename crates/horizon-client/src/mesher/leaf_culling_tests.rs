use super::*;
use horizon_assets::model::Direction as ModelDirection;

#[test]
fn leaf_culling_removes_only_internal_leaf_and_root_faces() {
    let root =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../.local/horizon-client/26.3");
    if !root.join("horizon-resources.json").exists() {
        return;
    }
    let assets = Assets::load(&root).unwrap();
    let facts = BlockFacts::load(&root, assets.registry).unwrap();
    let visibility = crate::models::load_visibility(&root, assets.registry).unwrap();
    let tables = MeshTables::new(&assets, &facts, visibility.clone()).unwrap();
    let biomes = Biomes::load(&root).unwrap();
    let state = |name| assets.registry.default_state(name).unwrap().0 as u16;
    let oak = state("minecraft:oak_leaves");
    let birch = state("minecraft:birch_leaves");
    let roots = state("minecraft:mangrove_roots");
    let stone = state("minecraft:stone");
    let mesh = |blocks: &[([i32; 3], u16)], cull_leaves, cutout_leaves| {
        let mut center = Chunk::empty(0, 0);
        let mut east = Chunk::empty(1, 0);
        for &([x, y, z], state) in blocks {
            let chunk = if x < 16 { &mut center } else { &mut east };
            chunk.set(x, y, z, state);
        }
        let mut region = [[None; 3]; 3];
        region[1][1] = Some(&center);
        region[1][2] = Some(&east);
        Mesher {
            nether_shading: false,
            assets: &assets,
            tables: &tables,
            biomes: &biomes,
            visibility: visibility.clone(),
            zoom: BiomeZoom::new(0),
            blend_radius: 0,
            smooth_lighting: false,
            cutout_leaves,
            cull_leaves,
        }
        .mesh(&region, (0 - MIN_Y) / 16)
        .solid
    };

    // Native leaves can contain unculled interior geometry as well as their
    // outer faces. Select the touching cull group from the actual baked model,
    // preserving every other quad and its full vertex attributes.
    let without_face = |at: [i32; 3], id, direction| {
        let vertices = mesh(&[(at, id)], false, true);
        let models = assets
            .model_catalog
            .collect(BlockStateId(id.into()), BlockPos::new(at[0], at[1], at[2]))
            .unwrap();
        let faces: Vec<_> = models.iter().flat_map(|(_, model)| &model.faces).collect();
        assert_eq!(vertices.len(), faces.len() * 4);
        let retained: Vec<_> = vertices
            .as_chunks::<4>()
            .0
            .iter()
            .zip(faces)
            .filter(|(_, face)| face.quad.cull != Some(direction))
            .flat_map(|(quad, _)| quad.iter().copied())
            .collect();
        assert!(retained.len() < vertices.len(), "fixture has a cull group");
        retained
    };

    // Same-species leaves, mixed leaves and roots hide only their touching
    // model groups. Disabling the option restores the isolated models.
    for (a, b) in [(oak, oak), (oak, birch), (roots, roots)] {
        let blocks = [([7, 7, 7], a), ([8, 7, 7], b)];
        let off = mesh(&blocks, false, true);
        let on = mesh(&blocks, true, true);
        assert_eq!(
            off.len(),
            mesh(&[blocks[0]], false, true).len() + mesh(&[blocks[1]], false, true).len()
        );
        let mut retained = without_face(blocks[0].0, a, ModelDirection::East);
        retained.extend(without_face(blocks[1].0, b, ModelDirection::West));
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&on),
            bytemuck::cast_slice::<_, u8>(&retained)
        );
    }

    // Roots do not join leaves into a single culling group. Ordinary opaque
    // neighbors also keep their existing occlusion behavior.
    for (a, b) in [(oak, roots), (roots, oak), (oak, stone)] {
        let blocks = [([7, 7, 7], a), ([8, 7, 7], b)];
        let off = mesh(&blocks, false, true);
        let on = mesh(&blocks, true, true);
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&on),
            bytemuck::cast_slice::<_, u8>(&off)
        );
    }

    // Opaque leaves already hide internal leaf faces independently of this
    // option, and a leaf beside air retains its complete baked model.
    let pair = [([7, 7, 7], oak), ([8, 7, 7], birch)];
    assert_eq!(
        mesh(&pair, false, false).len(),
        mesh(&pair, true, false).len()
    );
    let isolated = [([7, 7, 7], oak)];
    assert_eq!(
        bytemuck::cast_slice::<_, u8>(&mesh(&isolated, false, true)),
        bytemuck::cast_slice::<_, u8>(&mesh(&isolated, true, true))
    );

    // The padding read must also remove the center chunk's internal face
    // against a different leaf species in the neighboring chunk.
    let boundary = [([15, 7, 7], oak), ([16, 7, 7], birch)];
    assert_eq!(
        mesh(&boundary, false, true).len(),
        mesh(&[boundary[0]], false, true).len()
    );
    assert_eq!(
        bytemuck::cast_slice::<_, u8>(&mesh(&boundary, true, true)),
        bytemuck::cast_slice::<_, u8>(&without_face(boundary[0].0, oak, ModelDirection::East))
    );
}
