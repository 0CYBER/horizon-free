//! The camera's view frustum for particles: vanilla's always-on point test, and the section
//! cache behind the opt-in Particle Culling setting.

use super::{Particle, ParticleEngine, Vertex};
use glam::{DVec3, Quat, Vec3 as FVec3};
use std::array;

const PARTICLE_SECTION_CACHE_SIZE: usize = 128;
/// How far back, in blocks, the camera's frustum is pulled for the particle test (vanilla's
/// `Frustum.offset` in `ParticleEngine.extract`).
pub(super) const PARTICLE_FRUSTUM_OFFSET: f32 = 3.0;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum SectionFrustumResult {
    Outside,
    Intersects,
    Inside,
}

#[derive(Clone, Copy)]
struct CachedSectionFrustum {
    section: [i32; 3],
    radius_quarters: i32,
    result: SectionFrustumResult,
}

pub(super) struct ParticleSectionFrustumCache {
    entries: [Option<CachedSectionFrustum>; PARTICLE_SECTION_CACHE_SIZE],
}

impl Default for ParticleSectionFrustumCache {
    fn default() -> Self {
        Self {
            entries: [None; PARTICLE_SECTION_CACHE_SIZE],
        }
    }
}

impl ParticleSectionFrustumCache {
    pub(super) fn result(
        &mut self,
        world: [f64; 3],
        camera: DVec3,
        radius: f64,
        frustum: &crate::gfx::Frustum,
    ) -> SectionFrustumResult {
        let Some(section) = world_section(world) else {
            return SectionFrustumResult::Intersects;
        };
        if !radius.is_finite() || radius < 0.0 || radius * 4.0 > f64::from(i32::MAX) {
            return SectionFrustumResult::Intersects;
        }
        let radius_quarters = (radius * 4.0).ceil() as i32;
        let hash = (section[0] as u32).wrapping_mul(73_856_093)
            ^ (section[1] as u32).wrapping_mul(19_349_663)
            ^ (section[2] as u32).wrapping_mul(83_492_791)
            ^ (radius_quarters as u32).wrapping_mul(2_654_435_761);
        let slot = hash as usize & (PARTICLE_SECTION_CACHE_SIZE - 1);
        if let Some(entry) = self.entries[slot]
            && entry.section == section
            && entry.radius_quarters == radius_quarters
        {
            return entry.result;
        }

        let pad = f64::from(radius_quarters) * 0.25;
        let camera = camera.to_array();
        let min = array::from_fn(|axis| f64::from(section[axis]) * 16.0 - pad - camera[axis]);
        let max = array::from_fn(|axis| min[axis] + 16.0 + 2.0 * pad);
        let result = if min.into_iter().chain(max).any(|v| !v.is_finite()) {
            SectionFrustumResult::Intersects
        } else {
            let min = FVec3::from_array(min.map(|v| v as f32));
            let max = FVec3::from_array(max.map(|v| v as f32));
            if !min.is_finite() || !max.is_finite() {
                SectionFrustumResult::Intersects
            } else if frustum.contains(min, max) {
                SectionFrustumResult::Inside
            } else if frustum.intersects(min, max) {
                SectionFrustumResult::Intersects
            } else {
                SectionFrustumResult::Outside
            }
        };
        self.entries[slot] = Some(CachedSectionFrustum {
            section,
            radius_quarters,
            result,
        });
        result
    }
}

fn world_section(position: [f64; 3]) -> Option<[i32; 3]> {
    let mut block = [0; 3];
    for axis in 0..3 {
        let value = position[axis].floor();
        if !value.is_finite() || value < f64::from(i32::MIN) || value > f64::from(i32::MAX) {
            return None;
        }
        block[axis] = value as i32;
    }
    Some([block[0] >> 4, block[1] >> 4, block[2] >> 4])
}

/// Whether `QuadParticleGroup`'s test keeps `particle`: its position this tick (camera-relative, not
/// interpolated) is on or inside `view`, the camera's frustum pulled back [`PARTICLE_FRUSTUM_OFFSET`] blocks.
pub(super) fn in_view(view: &crate::gfx::Frustum, particle: &Particle, camera: DVec3) -> bool {
    let at = DVec3::from_array(particle.position) - camera;
    view.contains_point(at.as_vec3())
}

impl ParticleEngine {
    /// [`Self::render`], keeping only the particles inside `view`, the camera's frustum pulled
    /// back [`PARTICLE_FRUSTUM_OFFSET`] blocks (vanilla's `ParticleEngine.extract` and
    /// `QuadParticleGroup`): the test runs whatever the Particle Culling setting says. With
    /// `None`, every particle is drawn.
    pub fn render_in_view(
        &self,
        camera: DVec3,
        base: Quat,
        partial: f32,
        view: Option<&crate::gfx::Frustum>,
        light_at: &dyn Fn([i32; 3]) -> u8,
        out: &mut Vec<Vertex>,
    ) {
        self.render_inner::<false>(
            camera,
            base,
            partial,
            0.0,
            view,
            &mut |_, _| false,
            light_at,
            out,
        );
    }
}
