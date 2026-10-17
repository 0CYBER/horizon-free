//! Far meshes share the terrain arena and passes, with their actual bounds
//! rather than the 16³ boxes used by the near-section residency walk.
use super::{Frustum, Renderer, SectionGpu};
use crate::{lod::TileKey, mesher::SectionMesh};
use glam::{DVec3, Vec3};

pub(super) struct Tile {
    pub mesh: SectionGpu,
    bounds: Bounds,
}

#[derive(Clone, Copy)]
struct Bounds {
    min: Vec3,
    max: Vec3,
}

impl Bounds {
    fn mesh(mesh: &SectionMesh) -> Option<Self> {
        let mut positions = mesh
            .solid
            .iter()
            .chain(&mesh.translucent)
            .map(|v| Vec3::from(v.pos));
        let first = positions.next()?;
        let mut bounds = Self {
            min: first,
            max: first,
        };
        for pos in positions {
            bounds.min = bounds.min.min(pos);
            bounds.max = bounds.max.max(pos);
        }
        Some(bounds)
    }

    fn visible(self, origin: DVec3, distance: f32, frustum: &Frustum) -> Option<u64> {
        let min = origin + self.min.as_dvec3();
        let max = origin + self.max.as_dvec3();
        let nearest = DVec3::ZERO.clamp(min, max);
        if nearest.x * nearest.x + nearest.z * nearest.z > f64::from(distance).powi(2)
            || !frustum.intersects(min.as_vec3(), max.as_vec3())
        {
            return None;
        }
        Some(((min + max) * 0.5).length_squared().to_bits())
    }
}

pub(super) struct Visible {
    pub key: TileKey,
    pub distance: u64,
    pub origin: [f32; 3],
}

impl Tile {
    pub fn visible(
        &self,
        key: TileKey,
        camera: DVec3,
        distance: f32,
        frustum: &Frustum,
    ) -> Option<Visible> {
        let origin = DVec3::from(key.origin().map(f64::from)) - camera;
        Some(Visible {
            key,
            distance: self.bounds.visible(origin, distance, frustum)?,
            origin: origin.as_vec3().to_array(),
        })
    }
}

impl Renderer {
    pub fn resident_lods(&self) -> usize {
        self.lods.len()
    }

    pub fn upload_lod(&mut self, key: TileKey, mesh: &SectionMesh) {
        self.remove_lod(key);
        if let Some(bounds) = Bounds::mesh(mesh)
            && let Some(mesh) = self.upload_mesh(mesh)
        {
            self.lods.insert(key, Tile { mesh, bounds });
        }
    }

    pub fn remove_lod(&mut self, key: TileKey) {
        if let Some(tile) = self.lods.remove(&key) {
            self.release_mesh(tile.mesh);
        }
    }

    pub fn clear_lods(&mut self) {
        for (_, tile) in std::mem::take(&mut self.lods) {
            self.release_mesh(tile.mesh);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::mesher::TerrainVertex;

    #[test]
    fn large_tile_bounds_include_water_and_do_not_cull_by_origin() {
        let vertex = |pos| TerrainVertex {
            pos,
            uv: [0.0; 2],
            layer: 0,
            light: 0,
            color: 0,
        };
        let mesh = SectionMesh {
            solid: vec![vertex([0.0, -64.0, 0.0]), vertex([1024.0, 100.0, 1024.0])],
            translucent: vec![vertex([512.0, 120.0, 512.0])],
            ..Default::default()
        };
        let bounds = Bounds::mesh(&mesh).unwrap();
        assert_eq!(bounds.min, Vec3::new(0.0, -64.0, 0.0));
        assert_eq!(bounds.max, Vec3::new(1024.0, 120.0, 1024.0));
        let view = Frustum {
            planes: super::super::frustum(
                super::super::world_projection(70.0, 16.0 / 9.0, 8192.0)
                    * crate::camera::view_matrix(0.0, 0.0),
            ),
        };
        // The tile origin is farther than the range, but its far edge reaches
        // the camera. Negative world origins therefore remain visible.
        assert!(
            bounds
                .visible(DVec3::new(-1000.0, -80.0, -512.0), 256.0, &view)
                .is_some()
        );
        assert!(
            bounds
                .visible(DVec3::new(-2000.0, -80.0, -512.0), 256.0, &view)
                .is_none()
        );
        assert!(Bounds::mesh(&SectionMesh::default()).is_none());
    }
}
