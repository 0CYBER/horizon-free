//! Fluid surfaces following `FluidRenderer.tesselate`: corner heights, face
//! occlusion against neighbour shapes, flow-rotated top UVs, the water overlay
//! beside glass and leaves, back faces, cardinal shading and max(self, above)
//! light.
use super::{
    Info, MeshTables, Padded, SectionMesh, TerrainVertex, as_fluid, lighting, pack_coords,
    pack_rgb, part_tables::PartTables, pi,
};
use horizon_core::{
    geometry::{Axis, Direction, VoxelShape, trig},
    position::BlockPos,
};
use horizon_world::{
    fluid::{FluidKind, FluidState},
    fluid_flow::{FluidFlowWorld, flow_at},
};
use std::{convert::Infallible, sync::OnceLock};

/// `FluidRenderer.MAX_FLUID_HEIGHT`.
const MAX_HEIGHT: f32 = 0.888_888_9;
const EPSILON: f32 = 0.001;
/// `super::step_index` of the unit steps of the padded index: one block east
/// (+x), one south (+z), one up (+y).
const X_STEP: isize = 1;
const P_STEP: isize = super::P as isize;
const UP: isize = super::step_index([0, 1, 0]);

/// For the occlusion face toward `direction`: the height from the bottom over
/// which its slice flush with the `direction` side covers the whole width
/// (vertical faces: 1 when the whole square is covered, else 0). This is what
/// `Shapes.blockOccludes` decides for a fluid box of a given height.
pub(super) fn coverage(shape: &VoxelShape, direction: Direction) -> f32 {
    let (axis, u, v) = match direction {
        Direction::Down | Direction::Up => (Axis::Y, Axis::X, Axis::Z),
        Direction::North | Direction::South => (Axis::Z, Axis::X, Axis::Y),
        Direction::West | Direction::East => (Axis::X, Axis::Z, Axis::Y),
    };
    let positive = matches!(
        direction,
        Direction::Up | Direction::South | Direction::East
    );
    let rects: Vec<[f64; 4]> = shape
        .boxes()
        .iter()
        .filter(|b| {
            if positive {
                b.max(axis) >= 1.0 - 1e-7
            } else {
                b.min(axis) <= 1e-7
            }
        })
        .map(|b| [b.min(u), b.max(u), b.min(v), b.max(v)])
        .collect();
    let mut cuts: Vec<f64> = rects
        .iter()
        .flat_map(|r| [r[2], r[3]])
        .filter(|c| (0.0..=1.0).contains(c))
        .chain([0.0, 1.0])
        .collect();
    cuts.sort_by(f64::total_cmp);
    cuts.dedup();
    let mut covered = 0.0;
    for w in cuts.windows(2) {
        let middle = (w[0] + w[1]) / 2.0;
        let mut spans: Vec<(f64, f64)> = rects
            .iter()
            .filter(|r| r[2] <= middle && middle <= r[3])
            .map(|r| (r[0], r[1]))
            .collect();
        spans.sort_by(|a, b| a.0.total_cmp(&b.0));
        let mut reach = 0.0f64;
        for (lo, hi) in spans {
            if lo > reach + 1e-7 {
                break;
            }
            reach = reach.max(hi);
        }
        if reach < 1.0 - 1e-7 {
            break;
        }
        covered = w[1];
    }
    if matches!(direction, Direction::Down | Direction::Up) && covered < 1.0 {
        return 0.0;
    }
    covered as f32
}
/// `isFaceOccludedByState` for a fluid box `height` tall against `state`'s
/// face opposite `direction`.
fn occluded_by(direction: Direction, height: f32, state: &Info) -> bool {
    let opposite = direction.opposite() as usize;
    match state.face[opposite] {
        0 => false,
        1 => direction != Direction::Up || height == 1.0,
        _ => match direction {
            Direction::Up => {
                f64::from(height) >= 1.0 - 1e-7 && state.cover[Direction::Down as usize] >= 1.0
            }
            Direction::Down => state.cover[Direction::Up as usize] >= 1.0,
            _ => f64::from(height) <= f64::from(state.cover[opposite]) + 1e-7,
        },
    }
}
/// `isFaceOccludedBySelf`: the cell's own occlusion face toward `direction`.
fn occluded_by_self(state: &Info, direction: Direction) -> bool {
    match state.face[direction as usize] {
        0 => false,
        1 => true,
        _ => state.cover[direction as usize] >= 1.0,
    }
}
/// `Mth.atan2`: the client's table-driven arctangent.
fn atan2(mut y: f64, mut x: f64) -> f64 {
    static TABLES: OnceLock<(Vec<f64>, Vec<f64>)> = OnceLock::new();
    let (asin, cos) = TABLES.get_or_init(|| {
        (0..257)
            .map(|i| {
                let a = (f64::from(i) / 256.0).asin();
                (a, a.cos())
            })
            .unzip()
    });
    let frac_bias = f64::from_bits(4_805_340_802_404_319_232);
    let d2 = x * x + y * y;
    if d2.is_nan() {
        return f64::NAN;
    }
    let negative_y = y < 0.0;
    if negative_y {
        y = -y;
    }
    let negative_x = x < 0.0;
    if negative_x {
        x = -x;
    }
    let steep = y > x;
    if steep {
        std::mem::swap(&mut x, &mut y);
    }
    // Mth.fastInvSqrt
    let half = 0.5 * d2;
    let mut r = f64::from_bits((6_910_469_410_427_058_090i64 - (d2.to_bits() as i64 >> 1)) as u64);
    r *= 1.5 - half * r * r;
    x *= r;
    y *= r;
    let yp = frac_bias + y;
    let index = yp.to_bits() as i32 as usize;
    let s_phi = yp - frac_bias;
    let sd = y * cos[index] - x * s_phi;
    let mut theta = asin[index] + (6.0 + sd * sd) * sd * 0.166_666_666_666_666_66;
    if steep {
        theta = std::f64::consts::FRAC_PI_2 - theta;
    }
    if negative_x {
        theta = std::f64::consts::PI - theta;
    }
    if negative_y {
        theta = -theta;
    }
    theta
}
/// The padded section as `FlowingFluid.getFlow` sees it (section-relative positions).
struct FlowView<'a> {
    pad: &'a Padded,
    info: &'a [Info],
}
impl FlowView<'_> {
    fn id(&self, p: BlockPos) -> u32 {
        self.pad.blocks[pi(p.x, p.y, p.z)]
    }
}
impl FluidFlowWorld for FlowView<'_> {
    type Error = Infallible;
    type Block = u32;
    fn fluid(&mut self, p: BlockPos) -> Result<FluidState, Infallible> {
        Ok(self.info[self.id(p) as usize].fluid)
    }
    fn block(&mut self, p: BlockPos) -> Result<u32, Infallible> {
        Ok(self.id(p))
    }
    fn blocks_fluid_flow(&self, state: u32) -> bool {
        self.info[state as usize].blocks_flow
    }
    fn is_ice_class(&self, state: u32) -> bool {
        self.info[state as usize].ice
    }
    fn is_face_sturdy(
        &mut self,
        state: u32,
        _: BlockPos,
        d: Direction,
    ) -> Result<bool, Infallible> {
        Ok(self.info[state as usize].sturdy & 1 << d as u8 != 0)
    }
}
/// Whether a `shaders/` pack's fluid top with these corner heights is split into triangles along
/// its north-east to south-west diagonal, as Iris 1.11.7 draws it, rather than along vanilla's
/// north-west to south-east one (`FluidRenderer.tesselate` puts its vertices NW, SW, SE, NE, and
/// `RenderSystem`'s quad indices join vertex 0 to 2).
///
/// Our probe packs' readings of Iris (#8464) fit this: a top is split along the diagonal whose
/// corners differ more in height, and a top whose north-west and south-east corners are level
/// (a still, flat top among them) along north-east to south-west. It is a fit, not Iris's code:
/// it covers every lava top we read and every still top but one, and misses 26 of the 343 water
/// tops we read (undiagnosed).
pub(super) fn splits_north_east(nw: f32, ne: f32, sw: f32, se: f32) -> bool {
    let (along_nw, along_ne) = ((nw - se).abs(), (ne - sw).abs());
    along_nw < along_ne || along_nw == 0.0
}
/// Emits one fluid cell at section-relative `[x, y, z]`. `parts` holds the client `BlockPart`
/// answers read here: a face a neighbour hides (`should_hide_adjacent_fluid_face`) is not drawn,
/// and a side uses the overlay sprite where its neighbour says so (`should_display_fluid_overlay`).
/// `(nether, shader_pack_lighting, separate_ao, shader_pack)`: the Nether's face shading, only the
/// up-face value when a shader pack lights the faces (`Mesher::shader_pack_lighting`), whether to
/// keep pure alpha for separate AO, and whether a `shaders/` pack draws the terrain
/// (`Mesher::shader_pack`: tops split as Iris splits them, [`splits_north_east`], and the alpha
/// as that path reads it, `mesher::for_shaders_format`).
pub(super) fn tesselate(
    tables: &MeshTables,
    parts: &PartTables,
    (nether, shader_pack_lighting, separate_ao, shader_pack, looks): (bool, bool, bool, bool, bool),
    pad: &Padded,
    water_color: u32,
    out: &mut SectionMesh,
    [x, y, z]: [i32; 3],
) {
    use horizon_assets::model::Direction as D;
    let cardinal = |d: D| {
        if shader_pack_lighting {
            lighting::pack_flat(nether)
        } else {
            lighting::cardinal(nether, d)
        }
    };
    let info = &tables.info;
    // The cell's own padded index: every other cell sampled here is a constant
    // step from it (`super::step_index`), so no `pi` per sample.
    let cell = pi(x, y, z) as isize;
    let at = |step: isize| &info[pad.blocks[(cell + step) as usize] as usize];
    let me = at(0);
    let kind = me.fluid.kind();
    let same = |s: &Info| s.fluid.kind().same_family(kind);
    let neighbor_id = |d: Direction| pad.blocks[(cell + super::OFFSETS[d as usize]) as usize];
    let neighbor = |d: Direction| &info[neighbor_id(d) as usize];
    let (down, up) = (neighbor(Direction::Down), neighbor(Direction::Up));
    // Empty with no part answering: vanilla's tests alone.
    let hides = !parts.hidden_fluid_faces.is_empty();
    let hidden = |d: Direction| hides && parts.hides_fluid_face(neighbor_id(d), kind, d);
    let should_render = |d: Direction| !same(neighbor(d)) && !occluded_by_self(me, d) && !hidden(d);
    let render_up = !same(up) && !hidden(Direction::Up);
    let render_down =
        should_render(Direction::Down) && !occluded_by(Direction::Down, MAX_HEIGHT, down);
    let sides = [
        should_render(Direction::North),
        should_render(Direction::South),
        should_render(Direction::West),
        should_render(Direction::East),
    ];
    if !render_up && !render_down && !sides.iter().any(|&s| s) {
        return;
    }
    // A mod's fluid draws as water does, with the sprites and tint of its own model.
    let model = kind
        .mod_fluid()
        .and_then(|fluid| tables.mod_fluids.get(fluid.index()).copied().flatten());
    let water = matches!(
        kind,
        FluidKind::Water | FluidKind::FlowingWater | FluidKind::Mod(_) | FluidKind::ModFlowing(_)
    );
    let tint = match model.and_then(|model| model.tint) {
        Some(tint) => tint,
        None if water => water_color & 0xFF_FFFF,
        None => 0xFF_FFFF,
    };
    let emission = if water {
        0
    } else {
        crate::mesher::hdr_tier("lava", 15)
    };
    // What the fluid itself gives off, on every face and not the block holding it: lava's block
    // 15 at every level, water none, also around a waterlogged light; a mod fluid its own
    // block's (a mod fluid waterlogging another block reads that block's).
    let level = match kind {
        FluidKind::Lava | FluidKind::FlowingLava => 15,
        FluidKind::Mod(_) | FluidKind::ModFlowing(_) => me.emission,
        _ => 0,
    };
    let color = |by: f32| {
        let rgb = [16, 8, 0].map(|shift| lighting::scale((tint >> shift & 255) as i32, by) as u32);
        if separate_ao {
            pack_rgb(rgb)
        } else {
            crate::mesher::giving_off(crate::mesher::emitting(pack_rgb(rgb), emission), level)
        }
    };
    let height = |step: isize| -> f32 {
        let s = at(step);
        if same(s) {
            if same(at(step + UP)) {
                1.0
            } else {
                s.fluid.own_height()
            }
        } else if !s.legacy_solid {
            0.0
        } else {
            -1.0
        }
    };
    let own = height(0);
    let (mut ne, mut nw, mut se, mut sw) = (1.0f32, 1.0f32, 1.0f32, 1.0f32);
    if own < 1.0 {
        let n = height(-P_STEP);
        let s = height(P_STEP);
        let e = height(X_STEP);
        let w = height(-X_STEP);
        // calculateAverageHeight(self, height2, height1, corner)
        let average = |h2: f32, h1: f32, corner: isize| -> f32 {
            if h1 >= 1.0 || h2 >= 1.0 {
                return 1.0;
            }
            let mut sum = [0f32; 2];
            let mut add = |h: f32| {
                if h >= 0.8 {
                    sum[0] += h * 10.0;
                    sum[1] += 10.0;
                } else if h >= 0.0 {
                    sum[0] += h;
                    sum[1] += 1.0;
                }
            };
            if h1 > 0.0 || h2 > 0.0 {
                let hc = height(corner);
                if hc >= 1.0 {
                    return 1.0;
                }
                add(hc);
            }
            add(own);
            add(h1);
            add(h2);
            sum[0] / sum[1]
        };
        ne = average(n, e, X_STEP - P_STEP);
        nw = average(n, w, -X_STEP - P_STEP);
        se = average(s, e, X_STEP + P_STEP);
        sw = average(s, w, -X_STEP + P_STEP);
    }
    let light = |step: isize| {
        let i = (cell + step) as usize;
        let above = (cell + step + UP) as usize;
        pack_coords(lighting::max_coords(
            lighting::coords_at(pad, lighting::state_at(pad, info, i), i),
            lighting::coords_at(pad, lighting::state_at(pad, info, above), above),
        ))
    };
    let (still, flow, overlay) = if let Some(model) = model {
        (model.still, model.flow, model.overlay)
    } else if water {
        (
            tables.water_still,
            tables.water_flow,
            Some(tables.water_overlay),
        )
    } else {
        (tables.lava_still, tables.lava_flow, None)
    };
    let (target, target_looks, sprite_layers) = if water {
        (
            &mut out.translucent,
            &mut out.translucent_looks,
            &mut out.sprite_layers,
        )
    } else {
        (&mut out.solid, &mut out.solid_looks, &mut out.sprite_layers)
    };
    let mut quad = |v: [([f32; 3], [f32; 2]); 4],
                    layer: u16,
                    shade: f32,
                    light: u16,
                    back: bool,
                    face: usize| {
        if tables.is_animated(layer) && !sprite_layers.contains(&layer) {
            sprite_layers.push(layer);
        }
        if looks {
            let look = super::QuadLook::new(tint, shade, face as u32, [1.0; 4]);
            target_looks.resize(target.len() / 4, super::QuadLook::default());
            target_looks.extend(std::iter::repeat_n(look, 1 + usize::from(back)));
        }
        let color = color(shade);
        let color = if separate_ao {
            color
        } else if shader_pack {
            crate::mesher::for_shaders_format(color, Some(face))
        } else {
            as_fluid(color)
        };
        let vertex = |(pos, uv)| TerrainVertex::new(pos, uv, layer, color, light);
        target.extend([vertex(v[0]), vertex(v[1]), vertex(v[2]), vertex(v[3])]);
        if back {
            target.extend([vertex(v[0]), vertex(v[3]), vertex(v[2]), vertex(v[1])]);
        }
    };
    let (fx, fy, fz) = (x as f32, y as f32, z as f32);
    let bottom = if render_down { EPSILON } else { 0.0 };
    if render_up && !occluded_by(Direction::Up, nw.min(sw).min(se.min(ne)), up) {
        nw -= EPSILON;
        sw -= EPSILON;
        se -= EPSILON;
        ne -= EPSILON;
        let flow_vector = flow_at(
            &mut FlowView { pad, info },
            me.fluid,
            BlockPos::new(x, y, z),
        )
        .unwrap_or_else(|e| match e {});
        let (layer, uv) = if flow_vector.x == 0.0 && flow_vector.z == 0.0 {
            (still, [[0.0, 0.0], [0.0, 1.0], [1.0, 1.0], [1.0, 0.0]])
        } else {
            let angle =
                atan2(flow_vector.z, flow_vector.x) as f32 - std::f64::consts::FRAC_PI_2 as f32;
            let s = trig::sin(f64::from(angle)) * 0.25;
            let c = trig::cos(f64::from(angle)) * 0.25;
            (
                flow,
                [
                    [0.5 + (-c - s), 0.5 + (-c + s)],
                    [0.5 + (-c + s), 0.5 + (c + s)],
                    [0.5 + (c + s), 0.5 + (c - s)],
                    [0.5 + (c - s), 0.5 + (-c - s)],
                ],
            )
        };
        // FluidState.shouldRenderBackwardUpFace over the 3x3 cells above.
        let backward = (-1..=1).any(|ox| {
            (-1..=1).any(|oz| {
                let s = at(UP + ox as isize * X_STEP + oz as isize * P_STEP);
                !same(s) && !s.solid
            })
        });
        let mut corners = [
            ([fx, fy + nw, fz], uv[0]),
            ([fx, fy + sw, fz + 1.0], uv[1]),
            ([fx + 1.0, fy + se, fz + 1.0], uv[2]),
            ([fx + 1.0, fy + ne, fz], uv[3]),
        ];
        // Starting from the south-west corner joins it to the north-east one (0 to 2), with the
        // same winding.
        if shader_pack && splits_north_east(nw, ne, sw, se) {
            corners.rotate_left(1);
        }
        quad(
            corners,
            layer,
            cardinal(D::Up),
            light(0),
            backward,
            Direction::Up as usize,
        );
    }
    if render_down {
        let y0 = fy + bottom;
        quad(
            [
                ([fx, y0, fz], [0.0, 0.0]),
                ([fx + 1.0, y0, fz], [1.0, 0.0]),
                ([fx + 1.0, y0, fz + 1.0], [1.0, 1.0]),
                ([fx, y0, fz + 1.0], [0.0, 1.0]),
            ],
            still,
            cardinal(D::Down),
            light(-UP),
            false,
            Direction::Down as usize,
        );
    }
    let side_light = light(0);
    let up_shade = cardinal(D::Up);
    for (i, d) in [
        Direction::North,
        Direction::South,
        Direction::West,
        Direction::East,
    ]
    .into_iter()
    .enumerate()
    {
        let (h0, h1, [x0, z0], [x1, z1]) = match d {
            Direction::North => (nw, ne, [fx, fz + EPSILON], [fx + 1.0, fz + EPSILON]),
            Direction::South => (
                se,
                sw,
                [fx + 1.0, fz + 1.0 - EPSILON],
                [fx, fz + 1.0 - EPSILON],
            ),
            Direction::West => (sw, nw, [fx + EPSILON, fz + 1.0], [fx + EPSILON, fz]),
            _ => (
                ne,
                se,
                [fx + 1.0 - EPSILON, fz],
                [fx + 1.0 - EPSILON, fz + 1.0],
            ),
        };
        let face_state = neighbor(d);
        if !sides[i] || occluded_by(d, h0.max(h1), face_state) {
            continue;
        }
        let (layer, is_overlay) = match overlay {
            Some(layer) if parts.fluid_overlay(neighbor_id(d), kind, face_state.overlay) => {
                (layer, true)
            }
            _ => (flow, false),
        };
        // Vanilla's side is the up value times the side's; a shader pack's has no side factor.
        let side = if shader_pack_lighting {
            1.0
        } else if matches!(d, Direction::North | Direction::South) {
            cardinal(D::North)
        } else {
            cardinal(D::West)
        };
        let (v0, v1) = ((1.0 - h0) * 0.5, (1.0 - h1) * 0.5);
        quad(
            [
                ([x0, fy + h0, z0], [0.0, v0]),
                ([x1, fy + h1, z1], [0.5, v1]),
                ([x1, fy + bottom, z1], [0.5, 0.5]),
                ([x0, fy + bottom, z0], [0.0, 0.5]),
            ],
            layer,
            up_shade * side,
            side_light,
            !is_overlay,
            d as usize,
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn fluid_models_emit_distinct_layers_and_stale_tables_fall_back() {
        use super::super::{ModFluidLayers, Tint, mod_tint::ModTints};
        use horizon_world::fluid::{ModFluidRules, ModFluidType, register_mod_fluid};
        let fluid = register_mod_fluid(ModFluidType {
            id: "mesh_fixture:oil".into(),
            flowing_id: "mesh_fixture:flowing_oil".into(),
            bucket: None,
            rules: ModFluidRules {
                tick_delay: 5,
                drop_off: 1,
                slope_find_distance: 4,
                converts_to_source: false,
            },
        })
        .unwrap();
        let air = Info {
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
        };
        let mut tables = MeshTables {
            info: vec![
                air,
                air,
                Info {
                    overlay: true,
                    ..air
                },
            ],
            solid: vec![],
            full_faces: vec![],
            visibility: horizon_world::block_render::FaceVisibility::empty(),
            water_still: 11,
            water_flow: 12,
            water_overlay: 13,
            lava_still: 21,
            lava_flow: 22,
            mod_fluids: vec![None; fluid.index() + 1],
            animated_layers: vec![],
            mod_tints: ModTints::default(),
            section_geometry: std::sync::Arc::new([]),
            colormaps: Vec::new(),
            water_colormap: None,
        };
        let mut pad = Padded {
            blocks: vec![0; super::super::P.pow(3)],
            light: vec![0xF0; super::super::P.pow(3)],
        };
        pad.blocks[pi(0, 0, 0)] = 1;
        pad.blocks[pi(1, 0, 0)] = 2; // glass-like neighbour selects overlay
        let model = ModFluidLayers {
            still: 31,
            flow: 32,
            overlay: Some(33),
            tint: Some(0xFF0000),
        };
        for kind in [FluidKind::Mod(fluid), FluidKind::Water, FluidKind::Lava] {
            tables.info[1].fluid = FluidState::new(kind, 8, false);
            tables.mod_fluids[fluid.index()] = Some(model);
            let mut mesh = SectionMesh::default();
            tesselate(
                &tables,
                &PartTables::NONE,
                (false, false, true, false, false),
                &pad,
                0x00FF00,
                &mut mesh,
                [0, 0, 0],
            );
            let (vertices, expected) = if kind == FluidKind::Lava {
                assert!(mesh.translucent.is_empty());
                (&mesh.solid, vec![21, 22])
            } else {
                assert!(mesh.solid.is_empty());
                (
                    &mesh.translucent,
                    if kind == FluidKind::Water {
                        vec![11, 12, 13]
                    } else {
                        vec![31, 32, 33]
                    },
                )
            };
            let mut layers: Vec<_> = vertices.iter().map(|v| v.layer).collect();
            layers.sort_unstable();
            layers.dedup();
            assert_eq!(layers, expected);
            assert!(!vertices.is_empty());
            if kind == FluidKind::Mod(fluid) {
                assert_eq!(vertices[0].color & 0xFFFFFF, 0x0000FF); // packed red
            }
        }
        tables.info[1].fluid = FluidState::new(FluidKind::Mod(fluid), 8, false);
        tables.mod_fluids.clear();
        let mut mesh = SectionMesh::default();
        tesselate(
            &tables,
            &PartTables::NONE,
            (false, false, true, false, false),
            &pad,
            0x00FF00,
            &mut mesh,
            [0, 0, 0],
        );
        let mut layers: Vec<_> = mesh.translucent.iter().map(|v| v.layer).collect();
        layers.sort_unstable();
        layers.dedup();
        assert_eq!(layers, vec![11, 12, 13]);
    }

    /// A fluid side's look has the shade its colour has, the top's times the side's (0.72 and
    /// 0.54 in the Nether), and so does a far tile's look of that side: a pack's `shade`
    /// doesn't change where the far tiles begin.
    #[test]
    fn a_fluid_sides_look_has_the_far_tiles_shade() {
        use horizon_assets::model::Direction as D;
        let mut tables = super::super::part_tables_tests::tables(false);
        let mut pad = Padded {
            blocks: vec![0; super::super::P.pow(3)],
            light: vec![0xF0; super::super::P.pow(3)],
        };
        pad.blocks[pi(0, 0, 0)] = 1;
        for nether in [false, true] {
            for kind in [FluidKind::Water, FluidKind::Lava] {
                tables.info[1].fluid = FluidState::new(kind, 8, false);
                let mut mesh = SectionMesh::default();
                let keys = (nether, false, false, false, true);
                tesselate(
                    &tables,
                    &PartTables::NONE,
                    keys,
                    &pad,
                    0x3F76E4,
                    &mut mesh,
                    [0; 3],
                );
                let looks = [mesh.solid_looks, mesh.translucent_looks].concat();
                for side in [D::North, D::South, D::West, D::East] {
                    let far = crate::lod::far_look(1 + u8::from(nether), 0xFFFFFF, side, true);
                    let shades: Vec<u32> = looks
                        .iter()
                        .filter(|l| l.0[0] >> 27 & 7 == side as u32)
                        .map(|l| l.0[0] >> 24 & 7)
                        .collect();
                    assert!(!shades.is_empty(), "{kind:?} {side:?}");
                    assert!(
                        shades.iter().all(|&s| s == far[0] >> 24 & 7),
                        "{kind:?} {side:?} nether {nether}: near {shades:?}, far {}",
                        far[0] >> 24 & 7
                    );
                }
            }
        }
    }

    /// With a `shaders/` pack, a fluid's alpha is main's byte from before the light level and the
    /// fluid flag: the tier's `15 - code` in the low four bits, 8 plus the quad's face in the high
    /// four (`fluid_face`); else 6c's.
    #[test]
    fn a_fluids_alpha_is_the_old_byte_for_a_shaders_format_pack() {
        let mut tables = super::super::part_tables_tests::tables(false);
        let mut pad = Padded {
            blocks: vec![0; super::super::P.pow(3)],
            light: vec![0xF0; super::super::P.pow(3)],
        };
        pad.blocks[pi(0, 0, 0)] = 1;
        for (kind, tier, level) in [
            (FluidKind::Water, 0, 0u32),
            (FluidKind::Lava, crate::mesher::hdr_tier("lava", 15), 15),
        ] {
            tables.info[1].fluid = FluidState::new(kind, 8, false);
            let old = crate::mesher::emitting(0, tier) >> 24 | 0xF0;
            for shader_pack in [false, true] {
                let mut mesh = SectionMesh::default();
                // With looks, for each quad's face; the vertices are the same without.
                let keys = (false, false, false, shader_pack, true);
                tesselate(
                    &tables,
                    &PartTables::NONE,
                    keys,
                    &pad,
                    0x3F76E4,
                    &mut mesh,
                    [0; 3],
                );
                let quads = (mesh.solid.as_chunks::<4>().0.iter().zip(&mesh.solid_looks)).chain(
                    mesh.translucent
                        .as_chunks::<4>()
                        .0
                        .iter()
                        .zip(&mesh.translucent_looks),
                );
                let mut faces = Vec::new();
                for (quad, look) in quads {
                    let face = look.0[0] >> 27 & 7;
                    faces.push(face);
                    // Main's `fluid_face(emitting(rgb, tier), face)`.
                    let want = if shader_pack {
                        0x80 | face << 4 | old & 0xF
                    } else {
                        (15 - level) << 4 | old & 0x7
                    };
                    let alphas = quad.map(|v| v.color >> 24);
                    assert!(
                        alphas.iter().all(|&a| a == want),
                        "{kind:?} shader pack {shader_pack}, face {face}: {alphas:x?}, want {want:x}"
                    );
                }
                faces.sort_unstable();
                faces.dedup();
                assert_eq!(faces, [0, 1, 2, 3, 4, 5], "{kind:?}");
            }
        }
    }

    #[test]
    fn atan2_follows_the_table_approximation() {
        for (y, x) in [
            (1.0, 0.0),
            (0.0, 1.0),
            (-0.5, 0.25),
            (0.3, -0.9),
            (-1.0, -1.0),
            (0.1, 0.7),
        ] {
            let exact = f64::atan2(y, x);
            assert!((atan2(y, x) - exact).abs() < 1e-6, "{y} {x}");
        }
        assert!(atan2(f64::NAN, 1.0).is_nan());
    }
    /// Corner heights read back from Iris 1.11.7 with a probe pack of ours (#8464), from water
    /// and lava spreading over a floor and from a still lake, and whether Iris split each top
    /// along north-east to south-west. Each height is the vanilla average it reads as (for
    /// example 13/27 for 0.48047 + 0.001), less `FluidRenderer`'s 0.001.
    #[test]
    fn fluid_tops_split_where_iris_splits_them() {
        let h = |num: f32, den: f32| num / den - 0.001;
        // (north-west, north-east, south-west, south-east)
        let cases = [
            // Still and flat: a lake's water, a lava pool.
            ([h(8.0, 9.0); 4], true),
            ([h(96.0, 117.0); 4], true),
            // Level along one axis: both diagonals equally steep.
            (
                [h(5.0, 9.0), h(13.0, 27.0), h(5.0, 9.0), h(13.0, 27.0)],
                false,
            ),
            (
                [h(100.0, 117.0), h(100.0, 117.0), h(2.0, 3.0), h(2.0, 3.0)],
                false,
            ),
            // North-west to south-east steeper.
            ([h(5.0, 9.0), h(4.0, 9.0), h(4.0, 9.0), h(7.0, 18.0)], false),
            ([h(7.0, 18.0), h(4.0, 9.0), h(4.0, 9.0), h(5.0, 9.0)], false),
            ([h(5.0, 6.0), h(4.0, 9.0), h(2.0, 9.0), h(2.0, 9.0)], false),
            // North-east to south-west steeper.
            ([h(1.0, 2.0), h(5.0, 9.0), h(7.0, 18.0), h(4.0, 9.0)], true),
            (
                [h(4.0, 9.0), h(2.0, 9.0), h(96.0, 117.0), h(4.0, 9.0)],
                true,
            ),
            // North-west and south-east level: a saddle.
            ([h(4.0, 9.0), h(7.0, 18.0), h(7.0, 18.0), h(4.0, 9.0)], true),
        ];
        for ([nw, ne, sw, se], iris) in cases {
            assert_eq!(
                splits_north_east(nw, ne, sw, se),
                iris,
                "{:?}",
                [nw, ne, sw, se]
            );
        }
    }
    #[test]
    fn coverage_of_full_and_partial_faces() {
        assert_eq!(coverage(&VoxelShape::block(), Direction::North), 1.0);
        assert_eq!(coverage(&VoxelShape::block(), Direction::Up), 1.0);
        assert_eq!(coverage(&VoxelShape::empty(), Direction::East), 0.0);
        // A bottom slab covers the lower half of each side and the whole floor.
        let slab = VoxelShape::from_cube([1, 2, 1], &[true, false]).unwrap();
        assert_eq!(coverage(&slab, Direction::South), 0.5);
        assert_eq!(coverage(&slab, Direction::Down), 1.0);
        assert_eq!(coverage(&slab, Direction::Up), 0.0);
    }
}
