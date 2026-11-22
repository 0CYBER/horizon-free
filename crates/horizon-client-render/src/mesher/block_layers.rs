//! Shader-pack layer routing after vanilla face generation and before sorting/upload.
//! Source-state metadata, not a sprite, identifies each quad's selection.

use super::{MeshTables, OPAQUE_LAYER, SectionMesh, TerrainSource, packed};
use horizon_shaders::{BlockLayers, RenderLayer};
use horizon_world::{
    fluid::{FluidKind, FluidState},
    registry::{BlockRegistry, BlockStateId},
};

/// Model/fluid overrides by source state, plus default-solid lava sprites.
/// Workers apply snapshots to fresh quads; table changes require remeshing.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct BlockLayerTable {
    states: Vec<[Option<RenderLayer>; 2]>,
    lava_layers: Vec<u16>,
}

/// The name of the block of a fluid's source form (`minecraft:water` for flowing water).
fn source_name(kind: FluidKind) -> &'static str {
    match kind {
        FluidKind::Empty => FluidKind::Empty.name(),
        FluidKind::FlowingWater | FluidKind::Water => FluidKind::Water.name(),
        FluidKind::FlowingLava | FluidKind::Lava => FluidKind::Lava.name(),
        FluidKind::Mod(fluid) | FluidKind::ModFlowing(fluid) => &fluid.get().id,
    }
}

impl BlockLayerTable {
    pub fn resolve(registry: &BlockRegistry, layers: &BlockLayers) -> Self {
        let pick = |state: &horizon_world::chunk::BlockState| {
            layers.layer(&state.name, |key| {
                state.properties.get(key).map(String::as_str)
            })
        };
        let entries: Vec<_> = (0..registry.len())
            .map(|index| {
                let Some(state) = registry.state(BlockStateId(index as u32)) else {
                    return [None; 2];
                };
                let fluid = FluidState::from_block(state).unwrap_or(FluidState::EMPTY);
                let fluid_layer = if fluid.kind() == FluidKind::Empty {
                    None
                } else if state.name == source_name(fluid.kind()) {
                    pick(state)
                } else {
                    registry
                        .default_state(source_name(fluid.kind()))
                        .and_then(|id| registry.state(id))
                        .and_then(pick)
                };
                [pick(state), fluid_layer]
            })
            .collect();
        if entries.iter().all(|entry| entry == &[None; 2]) {
            Self::default()
        } else {
            Self {
                states: entries,
                lava_layers: Vec::new(),
            }
        }
    }

    /// The loader's exact default-solid sprite list, before explicit state overrides.
    pub fn with_lava_layers(mut self, layers: Vec<u16>) -> Self {
        self.lava_layers = layers;
        self
    }

    fn layer(&self, source: TerrainSource) -> Option<RenderLayer> {
        if source.cell & (1 << 13) == 0 {
            return None;
        }
        self.states.get(source.state as usize)?[(source.cell >> 12 & 1) as usize]
    }
}

impl SectionMesh {
    pub(crate) fn apply_block_layers(&mut self, layers: &BlockLayerTable, tables: &MeshTables) {
        self.route_layers(layers, |state| tables.opaque_cube(BlockStateId(state)));
    }

    fn route_layers(&mut self, layers: &BlockLayerTable, opaque_cube: impl Fn(u32) -> bool) {
        // Preserve the accepted shader's lava-sprite default for model and fluid faces.
        // Encode it before explicit overrides, so a shared sprite cannot override a state.
        for vertex in &mut self.solid {
            if layers.lava_layers.contains(&vertex.layer) {
                vertex.layer |= OPAQUE_LAYER;
            }
        }
        let pick = |source: TerrainSource| {
            // A waterlogged model and its fluid have different selections. The opaque-cube
            // restriction applies to the model, not to the water occupying its fluid space.
            let layer = layers.layer(source)?;
            if source.cell & (1 << 12) == 0 && opaque_cube(source.state) {
                None
            } else {
                Some(layer)
            }
        };
        if layers.states.is_empty()
            || !self
                .solid_sources
                .iter()
                .chain(&self.translucent_sources)
                .any(|&source| pick(source).is_some())
        {
            return;
        }
        let solid = std::mem::take(&mut self.solid);
        let translucent = std::mem::take(&mut self.translucent);
        let solid_sources = std::mem::take(&mut self.solid_sources);
        let translucent_sources = std::mem::take(&mut self.translucent_sources);
        let tints = std::mem::take(&mut self.solid_tints);
        // Looks are one per quad and written only while the mesher keeps them: they travel
        // with their quad, and stay empty when there were none.
        let solid_looks = std::mem::take(&mut self.solid_looks);
        let translucent_looks = std::mem::take(&mut self.translucent_looks);
        let keep_looks = !solid_looks.is_empty() || !translucent_looks.is_empty();
        for (was_translucent, vertices, sources, looks) in [
            (false, &solid, &solid_sources, &solid_looks),
            (true, &translucent, &translucent_sources, &translucent_looks),
        ] {
            for (index, vertices) in vertices.as_chunks::<4>().0.iter().enumerate() {
                let source = sources.get(index).copied().unwrap_or_default();
                let layer = pick(source);
                let translucent =
                    layer.map_or(was_translucent, |layer| layer == RenderLayer::Translucent);
                let mut quad = *vertices;
                if let Some(layer) = layer {
                    for vertex in &mut quad {
                        vertex.layer &= !OPAQUE_LAYER;
                        if layer == RenderLayer::Solid {
                            vertex.layer |= OPAQUE_LAYER;
                        }
                    }
                }
                let look = looks.get(index).copied().unwrap_or_default();
                if translucent {
                    self.translucent.extend(quad);
                    self.translucent_sources.push(source);
                    if keep_looks {
                        self.translucent_looks.push(look);
                    }
                } else {
                    self.solid.extend(quad);
                    self.solid_sources.push(source);
                    if keep_looks {
                        self.solid_looks.push(look);
                    }
                    self.solid_tints.push(if was_translucent {
                        packed::UNKNOWN_TINT
                    } else {
                        tints.get(index).copied().unwrap_or(packed::UNKNOWN_TINT)
                    });
                }
            }
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesher::{QuadLook, TerrainVertex};
    use horizon_shaders::{Macros, Properties};

    fn table(states: Vec<[Option<RenderLayer>; 2]>) -> BlockLayerTable {
        BlockLayerTable {
            states,
            lava_layers: Vec::new(),
        }
    }

    fn registry() -> BlockRegistry {
        let json = br#"{"defaults":{"minecraft:air":0,"example:panel":1,"minecraft:water":3,"example:other":4},"states":[
            ["minecraft:air",true,false,"x","minecraft:air"],
            ["example:panel",false,false,"x",{"id":"example:panel","properties":{"half":"upper","waterlogged":"false"}}],
            ["example:panel",false,false,"x",{"id":"example:panel","properties":{"half":"lower","waterlogged":"true"}}],
            ["minecraft:water",false,true,"x",{"id":"minecraft:water","properties":{"level":"0"}}],
            ["example:other",false,false,"x","example:other"]
        ]}"#;
        BlockRegistry::from_facts_json(&json[..]).unwrap()
    }

    #[test]
    fn resolves_filtered_models_separately_from_their_water() {
        let registry = registry();
        let layers = BlockLayers::parse(&Properties::parse(
            "layer.solid=water\nlayer.translucent=example:panel:half=upper\nlayer.cutout=example:panel:half=lower",
            &Macros::standard(),
        ));
        let table = BlockLayerTable::resolve(&registry, &layers);
        assert_eq!(table.states[0], [None, None]);
        assert_eq!(table.states[1], [Some(RenderLayer::Translucent), None]);
        assert_eq!(
            table.states[2],
            [Some(RenderLayer::Cutout), Some(RenderLayer::Solid)]
        );
        assert_eq!(
            table.states[3],
            [Some(RenderLayer::Solid), Some(RenderLayer::Solid)]
        );
        let reset = BlockLayerTable::resolve(&registry, &BlockLayers::default());
        assert_eq!(reset, BlockLayerTable::default());
        assert_ne!(table, reset);
    }

    fn quad(layer: u16, x: f32) -> [TerrainVertex; 4] {
        std::array::from_fn(|i| {
            TerrainVertex::new(
                [x, (i & 1) as f32, (i >> 1) as f32],
                [0.25, 0.75],
                layer,
                0x12345678,
                0x9876,
            )
        })
    }

    #[test]
    fn routes_passes_and_alpha_by_source_state_preserving_metadata() {
        let registry = registry();
        let layers = BlockLayers::parse(&Properties::parse(
            "layer.translucent=example:panel:half=upper\n\
             layer.cutout=example:panel:half=lower\n\
             layer.solid=water",
            &Macros::standard(),
        ));
        let table = BlockLayerTable::resolve(&registry, &layers);
        let a = TerrainSource::new(1, [1, 2, 3], 4, false);
        let b = TerrainSource::new(2, [4, 5, 6], 7, false);
        let c = TerrainSource::new(3, [7, 8, 9], 10, true);
        let d = TerrainSource::new(4, [10, 11, 12], 13, false);
        let mut mesh = SectionMesh::default();
        // All four states share a sprite; alpha and pass must still differ.
        mesh.solid.extend(quad(11 | OPAQUE_LAYER, 1.0));
        mesh.solid.extend(quad(11 | OPAQUE_LAYER, 2.0));
        mesh.solid.extend(quad(11 | OPAQUE_LAYER, 4.0));
        mesh.solid_sources.extend([a, b, d]);
        mesh.solid_tints.extend([0xaabbcc, 0x112233, 0x778899]);
        mesh.translucent.extend(quad(11, 3.0));
        mesh.translucent_sources.push(c);
        mesh.route_layers(&table, |_| false);
        assert_eq!(mesh.solid_sources, [b, d, c]);
        assert_eq!(mesh.translucent_sources, [a]);
        assert_eq!(mesh.solid_tints, [0x112233, 0x778899, packed::UNKNOWN_TINT]);
        assert_eq!(mesh.solid.len(), 12);
        assert_eq!(mesh.translucent.len(), 4);
        let quads = mesh.solid.as_chunks::<4>().0;
        for (quad, expected) in quads.iter().zip([11, 11 | OPAQUE_LAYER, 11 | OPAQUE_LAYER]) {
            assert!(
                quad.iter()
                    .all(|v| v.layer == expected && v.color == 0x12345678 && v.light == 0x9876)
            );
        }
        assert!(
            mesh.translucent
                .iter()
                .all(|v| v.layer == 11 && v.pos[0] == 1.0)
        );
        let before = (
            mesh.solid.clone(),
            mesh.translucent.clone(),
            mesh.solid_sources.clone(),
            mesh.translucent_sources.clone(),
        );
        mesh.route_layers(&BlockLayerTable::default(), |_| false);
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&mesh.solid),
            bytemuck::cast_slice::<_, u8>(&before.0)
        );
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&mesh.translucent),
            bytemuck::cast_slice::<_, u8>(&before.1)
        );
        assert_eq!(mesh.solid_sources, before.2);
        assert_eq!(mesh.translucent_sources, before.3);
    }

    /// Each quad's look moves with it when its pass changes, so the looks stay one per quad
    /// and beside the right quad.
    #[test]
    fn looks_travel_with_their_quads_between_passes() {
        let registry = registry();
        let layers = BlockLayers::parse(&Properties::parse(
            "layer.translucent=example:panel:half=upper\n\
             layer.cutout=example:panel:half=lower\n\
             layer.solid=water",
            &Macros::standard(),
        ));
        let table = BlockLayerTable::resolve(&registry, &layers);
        let a = TerrainSource::new(1, [1, 2, 3], 4, false);
        let b = TerrainSource::new(2, [4, 5, 6], 7, false);
        let c = TerrainSource::new(3, [7, 8, 9], 10, true);
        let d = TerrainSource::new(4, [10, 11, 12], 13, false);
        let look = |n: u32| QuadLook([n, n + 100]);
        let mut mesh = SectionMesh::default();
        mesh.solid.extend(quad(11 | OPAQUE_LAYER, 1.0));
        mesh.solid.extend(quad(11 | OPAQUE_LAYER, 2.0));
        mesh.solid.extend(quad(11 | OPAQUE_LAYER, 4.0));
        mesh.solid_sources.extend([a, b, d]);
        mesh.solid_tints.extend([1, 2, 3]);
        mesh.solid_looks.extend([look(1), look(2), look(4)]);
        mesh.translucent.extend(quad(11, 3.0));
        mesh.translucent_sources.push(c);
        mesh.translucent_looks.push(look(3));
        mesh.route_layers(&table, |_| false);
        // Quad `a` went translucent and `c` solid, as in the pass-routing test above.
        assert_eq!(mesh.solid_sources, [b, d, c]);
        assert_eq!(mesh.solid_looks, [look(2), look(4), look(3)]);
        assert_eq!(mesh.translucent_sources, [a]);
        assert_eq!(mesh.translucent_looks, [look(1)]);
        // A mesh made without looks keeps none.
        let mut plain = SectionMesh::default();
        plain.solid.extend(quad(11 | OPAQUE_LAYER, 1.0));
        plain.solid_sources.push(a);
        plain.solid_tints.push(1);
        plain.route_layers(&table, |_| false);
        assert!(plain.solid_looks.is_empty() && plain.translucent_looks.is_empty());
    }

    #[test]
    fn opaque_full_cubes_and_unknown_sources_keep_original_geometry() {
        let table = table(vec![[Some(RenderLayer::Translucent), None]]);
        let mut mesh = SectionMesh::default();
        mesh.solid.extend(quad(9 | OPAQUE_LAYER, 1.0));
        mesh.solid.extend(quad(9 | OPAQUE_LAYER, 2.0));
        mesh.solid_sources.extend([
            TerrainSource::new(0, [0, 0, 0], 0, false),
            TerrainSource::default(),
        ]);
        mesh.solid_tints.extend([7, 8]);
        mesh.visibility = 0x123;
        let before = mesh.solid.clone();
        mesh.route_layers(&table, |_| true);
        assert_eq!(
            bytemuck::cast_slice::<_, u8>(&mesh.solid),
            bytemuck::cast_slice::<_, u8>(&before)
        );
        assert!(mesh.translucent.is_empty());
        assert_eq!(mesh.solid_tints, [7, 8]);
        assert_eq!(mesh.visibility, 0x123);
    }

    #[test]
    fn lava_sprite_defaults_cover_models_and_fluids_but_state_overrides_win() {
        let sources = [
            TerrainSource::new(0, [1, 0, 0], 0, false),
            TerrainSource::new(1, [2, 0, 0], 0, false),
            TerrainSource::new(2, [3, 0, 0], 0, true),
            TerrainSource::new(3, [4, 0, 0], 0, true),
        ];
        let mut mesh = SectionMesh::default();
        for (i, source) in sources.iter().enumerate() {
            mesh.solid.extend(quad(7, i as f32));
            mesh.solid_sources.push(*source);
        }
        let defaults = BlockLayerTable::default().with_lava_layers(vec![7]);
        mesh.route_layers(&defaults, |_| false);
        assert!(
            mesh.solid
                .iter()
                .all(|vertex| vertex.layer == (7 | OPAQUE_LAYER))
        );
        let selections = table(vec![
            [None, None],
            [Some(RenderLayer::Cutout), None],
            [None, Some(RenderLayer::Cutout)],
            [None, None],
        ])
        .with_lava_layers(vec![7]);
        mesh.route_layers(&selections, |_| false);
        assert_eq!(mesh.solid_sources, sources);
        assert!(mesh.translucent.is_empty());
        let quads = mesh.solid.as_chunks::<4>().0;
        for (quad, opaque) in quads.iter().zip([true, false, false, true]) {
            assert!(quad.iter().all(|v| (v.layer & OPAQUE_LAYER != 0) == opaque));
        }
    }
}
