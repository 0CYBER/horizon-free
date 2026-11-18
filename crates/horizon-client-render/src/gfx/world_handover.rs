//! The world a renderer holds, handed to the renderer that replaces it on the same device when
//! only the frame changes (HDR output coming or going on the float frame, docs/graphics.md):
//! the sections and far tiles uploaded stay in their arenas, and the game meshes nothing again.
//! A mesh doesn't depend on the output: its HDR tier is the block's, whatever HDR is.
use super::Renderer;
use std::sync::Arc;

impl Renderer {
    /// Takes over the sections and far tiles `from` holds, with everything that names where
    /// they sit: `from` draws none of them afterwards. Only a renderer just made on `from`'s
    /// device, from the same resources, takes them (`previous` of `Renderer::new`); false
    /// otherwise, with nothing moved, and the game meshes the world again.
    ///
    /// What moves is what `clear_sections` drops, and what an upload sizes or lays out. The
    /// shader pack, GPU culling's records and the baked block entities stay behind: the game
    /// gives a new renderer those again.
    pub fn take_world(&mut self, from: &mut Self) -> bool {
        if !Arc::ptr_eq(&self.reuse, &from.reuse) || self.flat.is_some() != from.flat.is_some() {
            return false;
        }
        // The copies of the last uploads, recorded and not yet submitted.
        if let Some(copies) = from.take_section_copies() {
            from.queue.submit([copies]);
        }
        // Its records name `from`'s table: made again when the setting turns it on here.
        let _ = from.set_gpu_culling(false);
        let _ = self.set_gpu_culling(false);
        macro_rules! take {
            ($($field:ident),* $(,)?) => {
                $(std::mem::swap(&mut self.$field, &mut from.$field);)*
            };
        }
        take!(
            sections,
            layout,
            occlusion,
            arena,
            translucent_arena,
            sorted_indices,
            translucency_cursor,
            graveyard,
            // The quad indices grow with the largest section uploaded.
            indices,
            index_quads,
            packed_on,
            packed_quads,
            lods,
            lod_cull_free,
            lod_cull_next,
            lod_cutout,
            lod_retiring,
            lod_leaving,
            lod_pending,
            detail_columns,
            detail_revision,
            upload_retries,
            block_entities,
            block_entity_section_revisions,
            block_entity_membership_revisions,
            block_entity_marks,
            section_uploaded,
            newest_upload,
            newest_lod_fade,
            light_blocks,
            light_revision,
            ray_scene,
            capture_copies,
            // The arenas' pages hold their buffers of looks: their state goes with them.
            terrain_looks,
            look_groups,
            looks_layout,
        );
        // Until the game gives this renderer its pack again (`world_held`).
        self.world_held = from.external.is_some();
        // The mesher makes the layout the sections are uploaded in.
        crate::mesher::packed::pack_quads(self.packed_quads);
        if let (Some(flat), Some(old)) = (&mut self.flat, &mut from.flat) {
            flat.take_tables(old);
        }
        true
    }
}
