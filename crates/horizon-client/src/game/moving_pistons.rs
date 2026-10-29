//! `PistonHeadRenderer`: while a piston moves them, blocks are
//! `minecraft:moving_piston` block entities (drawn as nothing by the chunk
//! mesher); this draws each moved block at its interpolated offset.
use super::Game;
use horizon_client::mesher::Vertex;
use horizon_world::{piston_shape::MovingPistonState, registry::BlockStateId};

impl Game<'_> {
    /// `PistonHeadRenderer.extractRenderState` and `submit`.
    pub(super) fn moving_piston_geometry(
        &self,
        camera: glam::DVec3,
        alpha: f64,
        out: &mut Vec<Vertex>,
    ) {
        let partial = alpha as f32;
        let mut states = self.world.moving_pistons.clone();
        states.extend(self.world.moving_piston_fallback.iter().copied());
        for (p, piston) in states {
            let Some((block, base)) = self.moving_piston_blocks(piston, partial) else {
                continue;
            };
            let step = piston.movement_direction().normal();
            let from = horizon_core::position::BlockPos::new(
                p[0] - step[0],
                p[1] - step[1],
                p[2] - step[2],
            );
            let offset = piston.interpolated_offset(partial);
            let light = self.world.light(p[0], p[1], p[2]);
            let packed = (u32::from(light >> 4) * 16) | ((u32::from(light & 15) * 16) << 8);
            let corner = glam::DVec3::new(f64::from(p[0]), f64::from(p[1]), f64::from(p[2]));
            let moved = corner + glam::Vec3::from_array(offset).as_dvec3();
            self.moving_block_vertices(block, from, moved - camera, packed, out);
            if let Some(base) = base {
                let at = horizon_core::position::BlockPos::new(p[0], p[1], p[2]);
                self.moving_block_vertices(base, at, corner - camera, packed, out);
            }
        }
    }
    /// The moved block and, for a retracting piston, its extended base.
    fn moving_piston_blocks(
        &self,
        piston: MovingPistonState,
        partial: f32,
    ) -> Option<(BlockStateId, Option<BlockStateId>)> {
        let facts = self.res.facts();
        if facts.is_air(piston.moved_state) {
            return None;
        }
        let progress = piston.interpolated_progress(partial);
        let name = &*facts.get(piston.moved_state).name;
        // Vanilla tests `progress <= 4.0`: a moving head is always drawn so.
        if name == "minecraft:piston_head" {
            let short = if progress <= 0.5 { "true" } else { "false" };
            return Some((
                self.with_props(piston.moved_state, &[("short", short)])?,
                None,
            ));
        }
        if piston.source && !piston.extending {
            let registry = &self.pack.assets.registry;
            let facing = registry
                .state(piston.moved_state)?
                .properties
                .get("facing")?
                .clone();
            let kind = if name == "minecraft:sticky_piston" {
                "sticky"
            } else {
                "normal"
            };
            let short = if progress >= 0.5 { "true" } else { "false" };
            let head = self.with_props(
                registry.default_state("minecraft:piston_head")?,
                &[("type", kind), ("facing", &facing), ("short", short)],
            )?;
            let base = self.with_props(piston.moved_state, &[("extended", "true")])?;
            return Some((head, Some(base)));
        }
        Some((piston.moved_state, None))
    }
    /// One `MovingBlockRenderState`: the baked model at `at` (camera-relative),
    /// seeded by its block position and tinted like a falling block.
    fn moving_block_vertices(
        &self,
        state: BlockStateId,
        seed: horizon_core::position::BlockPos,
        at: glam::DVec3,
        light: u32,
        out: &mut Vec<Vertex>,
    ) {
        let tint = self
            .particle_sprite(state, [seed.x, seed.y, seed.z])
            .map(|(_, tint)| tint);
        let start = out.len();
        self.block_model_vertices(
            state,
            horizon_core::random::position_seed(seed),
            glam::Mat4::from_translation(at.as_vec3()),
            light,
            false,
            out,
        );
        if let Some(tint) = tint {
            let tint = horizon_client::mobs::render::abgr(tint);
            for vertex in &mut out[start..] {
                vertex.color = horizon_client::hud_layout::multiply(vertex.color, tint);
            }
        }
    }
}
