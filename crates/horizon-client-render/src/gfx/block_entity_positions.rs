//! Membership and revisions of block entities attached to section meshes.
use super::*;
impl Renderer {
    /// What goes with section `key`'s new mesh, before its quads go in: its old mesh is
    /// released. The fade it keeps.
    pub(super) fn before_mesh(&mut self, key: SectionKey, mesh: &MeshFacts) -> Option<Instant> {
        self.occlusion.set(key, mesh.visibility);
        let changed = if mesh.block_entities.is_empty() {
            self.block_entities.remove(&key).is_some()
        } else {
            let old = self
                .block_entities
                .insert(key, mesh.block_entities.to_vec());
            old.as_deref() != Some(mesh.block_entities)
        };
        if changed {
            self.block_entity_revision = next_block_entity_revision();
            self.block_entity_membership_revisions
                .insert(key, self.block_entity_revision);
        }
        if !mesh.block_entities.is_empty()
            || self.block_entity_membership_revisions.contains_key(&key)
        {
            // Packed light can change with an unchanged BE list. Keep a
            // tombstone for formerly occupied sections until unload so a
            // deleted last entity invalidates only its own cached section.
            self.block_entity_section_revisions
                .insert(key, next_block_entity_revision());
        }
        let lights_changed = if mesh.lights.is_empty() {
            self.light_blocks.remove(&key).is_some()
        } else {
            let old = self.light_blocks.insert(key, mesh.lights.to_vec());
            old.as_deref() != Some(mesh.lights)
        };
        if lights_changed {
            self.light_revision += 1;
        }
        if !self.lod_retiring.is_empty() {
            self.lod_retiring.remove(&key);
        }
        if mesh.translucent && !self.sections.contains(key) {
            // New translucent terrain, not an edit's remesh: large pages wait for it.
            self.shrink_gate.wait(Instant::now(), SHRINK_SETTLE);
        }
        let fading = match self.sections.find(key) {
            Some(old) => old.fading,
            None if !self.lods.is_empty() => {
                let now = std::time::Instant::now();
                self.newest_lod_fade = Some(now);
                Some(now)
            }
            None => None,
        };
        if fading.is_none() && !self.section_uploaded.contains_key(&key) {
            let now = std::time::Instant::now();
            self.section_uploaded.insert(key, now);
            self.newest_upload = Some(now);
        }
        self.release_section(key);
        fading
    }

    /// A moving payload must disappear immediately, before the asynchronous remesh arrives.
    pub fn invalidate_block_entity_position(&mut self, pos: [i32; 3]) {
        let Some(sy) = self.layout.section_index(pos[1]) else {
            return;
        };
        let key = (pos[0] >> 4, sy as i32, pos[2] >> 4);
        if let Some(entries) = self.block_entities.get_mut(&key) {
            entries.retain(|(at, _)| *at != pos);
        }
        let revision = next_block_entity_revision();
        self.block_entity_revision = revision;
        self.block_entity_membership_revisions.insert(key, revision);
        self.block_entity_section_revisions.insert(key, revision);
    }
}
