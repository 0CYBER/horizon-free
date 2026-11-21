//! What `fluid::tesselate` draws from the client `BlockPart` tables (arismoko/horizon#3191): a water source with
//! one block beside it, east, and air everywhere else.
use super::fluid::tesselate;
use super::part_tables::{FluidTable, PartTables};
use super::{Info, MeshTables, P, Padded, SectionMesh, Tint, mod_tint::ModTints, pi};
use horizon_core::geometry::Direction;
use horizon_world::fluid::{FluidKind, FluidState};

const AIR: u32 = 0;
const WATER: u32 = 1;
/// The block beside the water: not occluding, neither half transparent nor leaves.
const BESIDE: u32 = 2;
const WATER_FLOW: u16 = 12;
const WATER_OVERLAY: u16 = 13;
/// Fluid indexes per table row: vanilla's five fluids.
const FLUIDS: usize = 5;

pub(super) fn air() -> Info {
    Info {
        solid: false,
        full: false,
        permeable: true,
        shade: 1.0,
        emission: 0,
        tier: 0,
        emissive: false,
        fluid: FluidState::EMPTY,
        model: false,
        block_entity: false,
        tints: [Tint::None; 2],
        tint_count: 0,
        colormap: None,
        offset: None,
        legacy_solid: false,
        overlay: false,
        leaves: false,
        pane: false,
        opaque_cube: false,
        blocks_flow: false,
        ice: false,
        opaque_ice: false,
        sturdy: 0,
        face: [0; 6],
        cover: [0.0; 6],
    }
}

/// The tables, with the block beside the water selecting the overlay as vanilla's test does when
/// `overlay` is true (glass, leaves).
pub(super) fn tables(overlay: bool) -> MeshTables {
    let water = Info {
        fluid: FluidState::new(FluidKind::Water, 8, false),
        ..air()
    };
    MeshTables {
        info: vec![air(), water, Info { overlay, ..air() }],
        solid: vec![],
        full_faces: vec![],
        visibility: horizon_world::block_render::FaceVisibility::empty(),
        water_still: 11,
        water_flow: WATER_FLOW,
        water_overlay: WATER_OVERLAY,
        lava_still: 21,
        lava_flow: 22,
        mod_fluids: vec![],
        animated_layers: vec![],
        mod_tints: ModTints::default(),
        section_geometry: std::sync::Arc::new([]),
        colormaps: Vec::new(),
        water_colormap: None,
    }
}

/// The quads `tesselate` emits for the water cell, by the face they lie on (`Direction as
/// usize`), as their sprite layers.
fn faces(tables: &MeshTables, parts: &PartTables) -> [Vec<u16>; 6] {
    let mut pad = Padded {
        blocks: vec![AIR; P.pow(3)],
        light: vec![0xF0; P.pow(3)],
    };
    pad.blocks[pi(0, 0, 0)] = WATER;
    pad.blocks[pi(1, 0, 0)] = BESIDE;
    let mut mesh = SectionMesh::default();
    tesselate(
        tables,
        parts,
        (false, false, true, false, false),
        &pad,
        || 0x3F76E4,
        &mut mesh,
        [0, 0, 0],
    );
    let mut faces: [Vec<u16>; 6] = Default::default();
    for quad in mesh.translucent.as_chunks::<4>().0 {
        let on = |axis: usize, at: fn(f32) -> bool| quad.iter().all(|v| at(v.pos[axis]));
        let face = if on(0, |x| x > 0.99) {
            Direction::East
        } else if on(0, |x| x < 0.01) {
            Direction::West
        } else if on(2, |z| z > 0.99) {
            Direction::South
        } else if on(2, |z| z < 0.01) {
            Direction::North
        } else if on(1, |y| y < 0.01) {
            Direction::Down
        } else {
            Direction::Up
        };
        faces[face as usize].push(quad[0].layer);
    }
    faces
}

/// The tables with one entry: the block beside the water, for water (`minecraft:water`).
fn beside_water<T: Copy + Default>(answer: T) -> FluidTable<T> {
    let mut answers = vec![T::default(); 3 * FLUIDS];
    answers[BESIDE as usize * FLUIDS + FluidKind::Water.tick_id() as usize] = answer;
    FluidTable::new(FLUIDS, answers)
}

#[test]
fn without_tables_the_water_draws_its_flow_sides_front_and_back() {
    let faces = faces(&tables(false), &PartTables::NONE);
    for side in [
        Direction::North,
        Direction::South,
        Direction::West,
        Direction::East,
    ] {
        assert_eq!(faces[side as usize], [WATER_FLOW; 2], "{side:?}");
    }
    assert!(!faces[Direction::Up as usize].is_empty());
    assert!(!faces[Direction::Down as usize].is_empty());
}

#[test]
fn a_part_hiding_the_fluid_face_removes_that_side_only() {
    let tables = tables(false);
    let vanilla = faces(&tables, &PartTables::NONE);
    let parts = PartTables {
        hidden_fluid_faces: beside_water(1 << Direction::East as u8),
        ..PartTables::NONE
    };
    let hidden = faces(&tables, &parts);
    for side in Direction::ALL {
        if side == Direction::East {
            assert!(
                hidden[side as usize].is_empty(),
                "the hidden side draws nothing"
            );
        } else {
            assert_eq!(hidden[side as usize], vanilla[side as usize], "{side:?}");
        }
    }
    // A bit for another direction, or for another fluid, hides nothing here.
    let parts = PartTables {
        hidden_fluid_faces: beside_water(1 << Direction::West as u8),
        ..PartTables::NONE
    };
    assert_eq!(faces(&tables, &parts), vanilla);
    let mut answers = vec![0; 3 * FLUIDS];
    answers[BESIDE as usize * FLUIDS + FluidKind::Lava.tick_id() as usize] = 0x3F;
    let parts = PartTables {
        hidden_fluid_faces: FluidTable::new(FLUIDS, answers),
        ..PartTables::NONE
    };
    assert_eq!(faces(&tables, &parts), vanilla);
}

#[test]
fn a_table_built_without_a_fluid_or_state_answers_nothing() {
    let table = beside_water(0x3Fu8);
    assert_eq!(table.get(BESIDE, FluidKind::Water), Some(0x3F));
    assert_eq!(table.get(BESIDE + 1, FluidKind::Water), None);
    // A fluid registered after the table was built is past the end of its rows.
    let short = FluidTable::new(2, vec![0x3Fu8; 3 * 2]);
    assert_eq!(short.get(BESIDE, FluidKind::FlowingWater), Some(0x3F));
    assert_eq!(short.get(BESIDE, FluidKind::Water), None);
    assert_eq!(FluidTable::<u8>::EMPTY.get(BESIDE, FluidKind::Water), None);
}

#[test]
fn a_part_choosing_the_overlay_changes_the_sprite_of_that_side_only() {
    // Beside a block of neither class, a part turns the overlay on: one quad, no back face.
    let plain = tables(false);
    let vanilla = faces(&plain, &PartTables::NONE);
    let parts = PartTables {
        fluid_overlay: beside_water(true),
        ..PartTables::NONE
    };
    let overlaid = faces(&plain, &parts);
    assert_eq!(overlaid[Direction::East as usize], [WATER_OVERLAY]);
    for side in Direction::ALL
        .into_iter()
        .filter(|&side| side != Direction::East)
    {
        assert_eq!(overlaid[side as usize], vanilla[side as usize], "{side:?}");
    }
    // Beside glass, vanilla's overlay; a part answering false gives the flow sprite back.
    let glass = tables(true);
    assert_eq!(
        faces(&glass, &PartTables::NONE)[Direction::East as usize],
        [WATER_OVERLAY]
    );
    let parts = PartTables {
        fluid_overlay: beside_water(false),
        ..PartTables::NONE
    };
    assert_eq!(
        faces(&glass, &parts)[Direction::East as usize],
        [WATER_FLOW; 2]
    );
}
