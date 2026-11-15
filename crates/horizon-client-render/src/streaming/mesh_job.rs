//! One section meshed on a pool worker: the `Job::Mesh` of the player's level and a viewed
//! level's mesh job (`viewed_meshing`) run the same mesher with the same settings.
use super::{MeshSettings, Streaming, WorkerContext};
use crate::mesher::{MeshScratch, MeshTables, Mesher, Region, SectionMesh, borrow};
use horizon_world::block_render::FaceVisibility;
use std::sync::Arc;

/// What a worker keeps between its mesh jobs: the face visibility table and its occlusion cache
/// (like vanilla's per-thread cache: cloning it per job copied every state and face shape), for
/// the tables it was made from.
pub(super) type Faces = Option<(Arc<MeshTables>, FaceVisibility)>;

/// Section `sy` of `region`'s centre column, into `recycled`'s buffers.
pub(super) fn mesh_section(
    context: &WorkerContext,
    settings: &MeshSettings,
    faces: &mut Faces,
    scratch: &mut MeshScratch,
    region: &Region,
    sy: i32,
    recycled: SectionMesh,
) -> SectionMesh {
    let visibility = match faces.take() {
        Some((tables, visibility)) if Arc::ptr_eq(&tables, &context.tables) => visibility,
        _ => context.tables.visibility.clone(),
    };
    let mut mesher = Mesher {
        nether_shading: settings.nether,
        shader_pack_lighting: settings.shader_pack_lighting,
        separate_ao: settings.separate_ao,
        shader_pack: settings.shader_pack,
        zoom: settings.zoom,
        blend_radius: settings.blend_radius,
        smooth_lighting: settings.smooth_lighting,
        cutout_leaves: settings.cutout_leaves,
        opaque_ice: settings.opaque_ice,
        translucent_layers: &settings.translucent_layers,
        leaf_culling: settings.leaf_culling,
        hidden_states: &context.hidden_states,
        assets: &context.assets,
        render: &context.render,
        tables: &context.tables,
        biomes: &context.biomes,
        visibility,
    };
    let mut mesh = mesher.mesh_reusing(&borrow(region), sy, recycled, scratch);
    mesh.sort_solid();
    *faces = Some((context.tables.clone(), mesher.visibility));
    mesh
}

impl Streaming {
    /// The settings the player's sections mesh with now.
    pub(super) fn mesh_settings(&self) -> MeshSettings {
        MeshSettings {
            nether: self.nether_shading,
            shader_pack_lighting: self.shader_pack_lighting,
            separate_ao: self.separate_ao,
            shader_pack: self.shader_pack,
            zoom: self.biome_zoom,
            blend_radius: self.biome_blend,
            smooth_lighting: self.smooth_lighting,
            cutout_leaves: self.cutout_leaves,
            opaque_ice: self.opaque_ice,
            translucent_layers: self.translucent_layers.clone(),
            leaf_culling: self.leaf_culling,
        }
    }
}
