//! The quads' looks (`QuadLook`, a shader pack's `tint`, `shade`, `ao` and `face`) through the
//! real mesher: each quad's look gives back its vertex colour, they stay with their quads
//! through the solid sort and a mesh without them is the same; and `emission`, which the vertex
//! holds, is the block's light level. Needs prepared resources, like the other mesher geometry
//! tests.
use super::opaque_ice_tests::{Fixture, fixture};
use super::*;
use crate::chunk::Chunk;

fn mesh(f: &Fixture, chunk: &Chunk, looks: bool) -> SectionMesh {
    mesh_in(f, chunk, looks, false)
}

fn mesh_in(f: &Fixture, chunk: &Chunk, looks: bool, nether: bool) -> SectionMesh {
    mesh_for(f, chunk, (looks, nether, false))
}

/// `mesh_in`, and whether a `shaders/` pack draws the terrain (`Mesher::shader_pack`).
fn mesh_for(
    f: &Fixture,
    chunk: &Chunk,
    (looks, nether, shader_pack): (bool, bool, bool),
) -> SectionMesh {
    let region = [[Some(chunk); 3]; 3];
    let mut mesher = Mesher {
        nether_shading: nether,
        shader_pack_lighting: false,
        separate_ao: false,
        shader_pack,
        looks,
        assets: &f.assets,
        render: &f.render,
        tables: &f.tables,
        biomes: &f.biomes,
        visibility: f.visibility.clone(),
        zoom: BiomeZoom::new(0),
        blend_radius: 0,
        smooth_lighting: true,
        cutout_leaves: true,
        opaque_ice: false,
        translucent_layers: &[],
        leaf_culling: LeafCullingMode::Off,
        hidden_states: &[],
        part_tables: Default::default(),
    };
    let mut mesh = mesher.mesh(&region, 0);
    mesh.sort_solid();
    mesh
}

fn state(f: &Fixture, name: &str, properties: &[(&str, &str)]) -> u32 {
    let registry = f.assets.registry;
    let mut state = registry
        .state(registry.default_state(name).unwrap())
        .unwrap()
        .clone();
    for (key, value) in properties {
        state.properties.insert((*key).into(), (*value).into());
    }
    registry.id(&state).unwrap().0
}

const BASE: i32 = MIN_Y + 4;

/// The blocks of the emission table, each with vanilla's light level (`None`: drawn as a block
/// entity, so no terrain quad), one every other block along x and z. The water of a
/// waterlogged one gives off what water does, nothing. A lit candle gives 3 a candle.
const BLOCKS: [(&str, &[(&str, &str)], Option<u8>); 24] = [
    ("minecraft:torch", &[], Some(14)),
    ("minecraft:lantern", &[], Some(15)),
    ("minecraft:lantern", &[("waterlogged", "true")], Some(15)),
    ("minecraft:lantern", &[("hanging", "true")], Some(15)),
    ("minecraft:candle", &[("lit", "true")], Some(3)),
    (
        "minecraft:candle",
        &[("lit", "true"), ("candles", "4")],
        Some(12),
    ),
    ("minecraft:candle", &[("candles", "4")], Some(0)),
    ("minecraft:candle_cake", &[("lit", "true")], Some(3)),
    ("minecraft:fire", &[], Some(15)),
    ("minecraft:sea_lantern", &[], Some(15)),
    ("minecraft:glowstone", &[], Some(15)),
    ("minecraft:ochre_froglight", &[], Some(15)),
    ("minecraft:soul_torch", &[], Some(10)),
    ("minecraft:soul_fire", &[], Some(10)),
    ("minecraft:lava", &[], Some(15)),
    ("minecraft:lava", &[("level", "4")], Some(15)),
    ("minecraft:glow_lichen", &[("down", "true")], Some(7)),
    ("minecraft:sea_pickle", &[("waterlogged", "false")], Some(0)),
    ("minecraft:sea_pickle", &[("pickles", "4")], Some(15)),
    ("minecraft:end_portal_frame", &[], Some(1)),
    ("minecraft:ender_chest", &[], None),
    ("minecraft:stone", &[], Some(0)),
    ("minecraft:water", &[], Some(0)),
    // Its petals' faces have their own `light_emission` (`cross_emissive`).
    ("minecraft:open_eyeblossom", &[], Some(0)),
];

fn place(i: usize) -> (i32, i32) {
    (1 + 2 * (i as i32 % 7), 1 + 2 * (i as i32 / 7))
}

fn all_quads(mesh: &SectionMesh) -> impl Iterator<Item = &[TerrainVertex; 4]> {
    let solid = mesh.solid.as_chunks::<4>().0.iter();
    solid.chain(mesh.translucent.as_chunks::<4>().0.iter())
}

fn quads(mesh: &SectionMesh) -> impl Iterator<Item = (&[TerrainVertex; 4], QuadLook)> {
    let solid = mesh.solid.as_chunks::<4>().0.iter().zip(&mesh.solid_looks);
    let translucent = mesh.translucent.as_chunks::<4>().0.iter();
    solid
        .chain(translucent.zip(&mesh.translucent_looks))
        .map(|(q, &l)| (q, l))
}

#[test]
#[ignore = "daily"]
fn each_quads_look_gives_back_its_vertex_colour() {
    let f = fixture();
    let mut chunk = Chunk::empty(0, 0);
    for (i, (name, properties, _)) in BLOCKS.iter().enumerate() {
        let (x, z) = place(i);
        chunk.set(x, BASE, z, state(&f, name, properties));
    }
    // Tinted overlays and cut-outs, water above and below, and an inside corner's shade.
    for (x, name) in [
        (2, "grass_block"),
        (4, "oak_leaves"),
        (6, "water"),
        (8, "stone"),
    ] {
        chunk.set(x, BASE + 4, 4, state(&f, &format!("minecraft:{name}"), &[]));
        chunk.set(x, BASE + 5, 4, state(&f, &format!("minecraft:{name}"), &[]));
        chunk.set(
            x + 1,
            BASE + 4,
            4,
            state(&f, &format!("minecraft:{name}"), &[]),
        );
    }
    // The Nether's shades too, with its fluid sides' (`SHADES` 6 and 7).
    for nether in [false, true] {
        let with = mesh_in(&f, &chunk, true, nether);
        let without = mesh_in(&f, &chunk, false, nether);
        assert!(without.solid_looks.is_empty() && without.translucent_looks.is_empty());
        let bytes = |v: &[TerrainVertex]| bytemuck::cast_slice::<_, u8>(v).to_vec();
        assert_eq!(bytes(&with.solid), bytes(&without.solid));
        assert_eq!(bytes(&with.translucent), bytes(&without.translucent));
        assert_eq!(with.packed, without.packed);
        assert_eq!(with.solid_looks.len() * 4, with.solid.len());
        assert_eq!(with.translucent_looks.len() * 4, with.translucent.len());
        let mut checked = 0;
        for (quad, look) in quads(&with) {
            let [a, ao] = look.0;
            let code = (a >> 24 & 7) as usize;
            assert!(code > 0, "a quad without its look");
            let shade = SHADES[code - 1];
            for (i, v) in quad.iter().enumerate() {
                let gray = lighting::scale((ao >> (8 * i) & 255) as i32, shade) as u32;
                for (channel, shift) in [(0, 0), (8, 8), (16, 16)] {
                    let want = gray * (a >> channel & 255) / 255;
                    let got = v.color >> shift & 255;
                    assert!(
                        want.abs_diff(got) <= 1,
                        "corner {i}: look {:08x} {ao:08x} gives {want}, the vertex has {got}",
                        a
                    );
                }
            }
            checked += 1;
        }
        assert!(checked > 60, "{checked} quads");
    }
}

#[test]
#[ignore = "daily"]
fn emission_is_the_blocks_light_level() {
    let f = fixture();
    let mut chunk = Chunk::empty(0, 0);
    for (i, (name, properties, _)) in BLOCKS.iter().enumerate() {
        let (x, z) = place(i);
        chunk.set(x, BASE, z, state(&f, name, properties));
    }
    // Without the looks: the vertex holds it, as `gfx/native_material.rs` reads it back.
    let mesh = mesh(&f, &chunk, false);
    let level_of = |q: &[TerrainVertex; 4]| {
        let levels = q.map(|v| 15 - (v.color >> 28));
        assert!(levels.iter().all(|&l| l == levels[0]), "{levels:?}");
        levels[0]
    };
    let y = (BASE - MIN_Y) as f32;
    for (i, (name, properties, level)) in BLOCKS.iter().enumerate() {
        let (x, z) = place(i);
        let (x, z) = (x as f32, z as f32);
        // Translucent quads here are water's.
        let levels: Vec<(bool, u32)> = all_quads(&mesh)
            .enumerate()
            .map(|(i, q)| (i >= mesh.solid.len() / 4, q, level_of(q)))
            .filter(|(_, q, _)| {
                let c = q.iter().fold([0.0; 3], |c, v| {
                    [
                        c[0] + v.pos[0] / 4.0,
                        c[1] + v.pos[1] / 4.0,
                        c[2] + v.pos[2] / 4.0,
                    ]
                });
                (x - 0.01..x + 1.01).contains(&c[0])
                    && (y - 0.01..y + 1.01).contains(&c[1])
                    && (z - 0.01..z + 1.01).contains(&c[2])
            })
            .map(|(translucent, _, l)| (translucent, l))
            .collect();
        let waterlogged = properties.contains(&("waterlogged", "true"))
            || name.ends_with("sea_pickle") && properties.iter().all(|p| p.0 != "waterlogged");
        if waterlogged || *name == "minecraft:water" {
            assert!(
                levels.iter().any(|q| q.0),
                "{name} {properties:?} has no water"
            );
        }
        match level {
            Some(level) => {
                let block = |water: bool| !water || *name == "minecraft:water";
                assert!(
                    levels.iter().any(|q| block(q.0)),
                    "{name} {properties:?} has no quads"
                );
                let want = |water: bool| if block(water) { u32::from(*level) } else { 0 };
                assert!(
                    levels.iter().all(|q| q.1 == want(q.0)),
                    "{name} {properties:?}: {levels:?}, not {level} (its water 0)"
                );
            }
            None => assert!(levels.is_empty(), "{name} has terrain quads"),
        }
    }
    // A face's own `light_emission` reaches its vertices' block light instead (`light`): the
    // eyeblossom's petals have 15, its other faces the light around them.
    let (x, z) = place(BLOCKS.len() - 1);
    let middle =
        |q: &[TerrainVertex; 4], axis: usize| q.iter().map(|v| v.pos[axis]).sum::<f32>() / 4.0;
    let blocks: Vec<u16> = (mesh.solid.as_chunks::<4>().0.iter())
        .filter(|q| middle(q, 0).floor() == x as f32 && middle(q, 2).floor() == z as f32)
        .map(|q| q[0].light >> 8)
        .collect();
    assert!(
        blocks.contains(&240) && blocks.iter().any(|&b| b < 240),
        "the eyeblossom's quads' block light: {blocks:?}"
    );
}

/// With a `shaders/` pack the vertex alpha is main's byte from before the light level and the fluid
/// flag, as that path reads it: the HDR tier's `15 - code` in the low four bits, and in the high
/// four 0xF, or on a fluid 8 plus the quad's face (`fluid_face`); without one, 6c's byte.
#[test]
#[ignore = "daily"]
fn a_shaders_format_pack_gets_the_alpha_from_before_the_light_level() {
    let f = fixture();
    // (block, its HDR tier, its light level, a fluid)
    let blocks = [
        ("glowstone", hdr_tier("glowstone", 15), 15, false),
        ("lava", hdr_tier("lava", 15), 15, true),
        ("water", 0, 0, true),
        ("stone", 0, 0, false),
    ];
    let mut chunk = Chunk::empty(0, 0);
    for (i, (name, ..)) in blocks.iter().enumerate() {
        let (x, z) = place(i);
        chunk.set(x, BASE, z, state(&f, &format!("minecraft:{name}"), &[]));
    }
    let y = (BASE - MIN_Y) as f32;
    for shader_pack in [false, true] {
        // With looks, for each quad's face; the vertices are the same without.
        let mesh = mesh_for(&f, &chunk, (true, false, shader_pack));
        for (i, &(name, tier, level, fluid)) in blocks.iter().enumerate() {
            let (x, z) = place(i);
            let (x, z) = (x as f32, z as f32);
            let alphas: Vec<(u32, u32)> = quads(&mesh)
                .filter(|(q, _)| {
                    let c = q.iter().fold([0.0; 3], |c, v| {
                        [
                            c[0] + v.pos[0] / 4.0,
                            c[1] + v.pos[1] / 4.0,
                            c[2] + v.pos[2] / 4.0,
                        ]
                    });
                    (x - 0.01..x + 1.01).contains(&c[0])
                        && (y - 0.01..y + 1.01).contains(&c[1])
                        && (z - 0.01..z + 1.01).contains(&c[2])
                })
                .flat_map(|(q, l)| q.map(|v| (l.0[0] >> 27 & 7, v.color >> 24)))
                .collect();
            let old = emitting(0, tier) >> 24 | 0xF0;
            let want = |face: u32| match (shader_pack, fluid) {
                (true, false) => old,
                // Main's `fluid_face`.
                (true, true) => 0x80 | face << 4 | old & 0xF,
                (false, _) => (15 - level) << 4 | u32::from(!fluid) << 3 | old & 7,
            };
            assert!(!alphas.is_empty(), "{name} has no quads");
            assert!(
                alphas.iter().all(|&(face, a)| a == want(face)),
                "{name}, shader pack {shader_pack}: {alphas:x?}"
            );
        }
    }
}

/// `shade` is the shade the mesher gave the quad, not one of its normal: a torch's element
/// takes the top's shade on every face (`shade_direction_override`), a lantern's body is
/// shaded by face and its handle as the top, and a comparator holds both kinds and a torch
/// shaded by face. In the Nether the top's shade is 0.9.
#[test]
#[ignore = "daily"]
fn shade_is_the_shade_the_quad_was_given() {
    let f = fixture();
    let blocks = [
        ("minecraft:torch", &[][..]),
        ("minecraft:lantern", &[]),
        ("minecraft:comparator", &[("powered", "true")]),
    ];
    let mut chunk = Chunk::empty(0, 0);
    for (i, (name, properties)) in blocks.iter().enumerate() {
        chunk.set(1 + 4 * i as i32, BASE, 1, state(&f, name, properties));
    }
    for nether in [false, true] {
        let mesh = mesh_in(&f, &chunk, true, nether);
        let top = if nether { 0.9 } else { 1.0 };
        // The shades of each block's quads by the face they look toward: up, the sides.
        let shades = |i: usize| -> Vec<(u32, f32)> {
            let x = (1 + 4 * i) as f32;
            quads(&mesh)
                .filter(|(q, _)| q.iter().all(|v| (x - 0.01..x + 1.01).contains(&v.pos[0])))
                .map(|(_, l)| (l.0[0] >> 27 & 7, SHADES[(l.0[0] >> 24 & 7) as usize - 1]))
                .collect()
        };
        let torch = shades(0);
        assert!(
            torch.len() >= 5 && torch.iter().all(|&(_, s)| s == top),
            "{torch:?}"
        );
        for i in [1, 2] {
            let quads = shades(i);
            let sides = |s: f32| quads.iter().any(|&(face, shade)| face >= 2 && shade == s);
            // Shaded by face (north and south 0.8, west and east 0.6), and as the top.
            assert!(
                sides(0.8) && sides(0.6) && sides(top),
                "{}: {quads:?}",
                blocks[i].0
            );
        }
    }
}

/// `blocks` (each its name and place in the section) alone in the world's lowest section,
/// meshed and sorted as a native pack's terrain is (rays run only under one), and its solid
/// quads' compact records (as the arena holds them while every pass reads that layout): what
/// the ray scene unpacks and traces (`gfx/rt/terrain_tests.rs`).
pub(crate) fn meshed_alone(blocks: &[(&str, [i32; 3])]) -> (SectionMesh, Vec<[u32; 6]>) {
    let states: Vec<_> = blocks
        .iter()
        .map(|&(name, at)| (name, &[][..], at))
        .collect();
    meshed_alone_states(&states)
}

/// `meshed_alone` with each block's state properties (a lit candle).
pub(crate) fn meshed_alone_states(
    blocks: &[(&str, &[(&str, &str)], [i32; 3])],
) -> (SectionMesh, Vec<[u32; 6]>) {
    meshed(blocks, false, false)
}

/// Mesh stateful blocks with optional looks and Nether shading.
pub(crate) fn meshed(
    blocks: &[(&str, &[(&str, &str)], [i32; 3])],
    looks: bool,
    nether: bool,
) -> (SectionMesh, Vec<[u32; 6]>) {
    let f = fixture();
    let mut chunk = Chunk::empty(0, 0);
    for &(name, properties, [x, y, z]) in blocks {
        chunk.set(x, MIN_Y + y, z, state(&f, name, properties));
    }
    let region = [[Some(&chunk); 3]; 3];
    let mut mesher = Mesher {
        nether_shading: nether,
        shader_pack_lighting: false,
        separate_ao: false,
        shader_pack: false,
        looks,
        assets: &f.assets,
        render: &f.render,
        tables: &f.tables,
        biomes: &f.biomes,
        visibility: f.visibility.clone(),
        zoom: BiomeZoom::new(0),
        blend_radius: 0,
        smooth_lighting: true,
        cutout_leaves: true,
        opaque_ice: false,
        translucent_layers: &[],
        leaf_culling: LeafCullingMode::Off,
        hidden_states: &[],
        part_tables: Default::default(),
    };
    let mut mesh = mesher.mesh(&region, 0);
    // `sort_solid`'s order, keeping the tints it drops when the renderer doesn't read records.
    mesh.solid_tints
        .resize(mesh.solid.len() / 4, packed::UNKNOWN_TINT);
    sides::sort_sourced(
        &mut mesh.solid,
        &mut mesh.solid_tints,
        &mut mesh.solid_sources,
        &mut mesh.solid_looks,
    );
    let records = packed::layout(&mesh.solid, &mesh.solid_tints);
    (mesh, records)
}
