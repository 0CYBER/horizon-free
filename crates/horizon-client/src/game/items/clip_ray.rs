//! The block ray of `Game::clip_ray` (`BlockGetter.clip`), with or without fluids.
use super::*;

impl Game<'_> {
    /// `clip_ray`; with `fluids`, the first fluid block a hit too, at its near face (`ClipContext.Fluid.ANY`).
    pub(super) fn clip_ray_fluids(
        &self,
        origin: [f64; 3],
        dir: [f64; 3],
        reach: f64,
        fluids: bool,
    ) -> Option<Hit> {
        let mut pos = origin.map(|v| v.floor() as i32);
        let step = dir.map(|d| if d > 0.0 { 1 } else { -1 });
        let delta = dir.map(|d| {
            if d == 0.0 {
                f64::INFINITY
            } else {
                (1.0 / d).abs()
            }
        });
        let mut t_max = [0.0; 3];
        for i in 0..3 {
            let boundary = if dir[i] > 0.0 {
                f64::from(pos[i]) + 1.0
            } else {
                f64::from(pos[i])
            };
            t_max[i] = if dir[i] == 0.0 {
                f64::INFINITY
            } else {
                (boundary - origin[i]) / dir[i]
            };
        }
        // `VoxelShape.clip`'s inside test: the start nudged along the ray, and
        // the sweep that gives the face of a hit there.
        let start = Vec3::new(origin[0], origin[1], origin[2]);
        let sweep = Vec3::new(
            origin[0] + dir[0] * reach,
            origin[1] + dir[1] * reach,
            origin[2] + dir[2] * reach,
        )
        .subtract(start);
        let nudged = start.add_vector(sweep.scale(0.001));
        let mut face = [0; 3];
        let mut t = 0.0;
        while t <= reach {
            let state = self.world.block(pos[0], pos[1], pos[2]);
            let f = self.res.facts().get(state);
            if self.collision.cached_shapes().has_context_shapes(state) {
                if let Some(hit) = self.clip_declared_outline(state, pos, origin, dir, reach) {
                    return Some(hit);
                }
            } else if !self.res.facts().is_air(state)
                && (f.fluid == Fluid::None
                    || f.waterlogged
                    || &*f.name == "minecraft:kelp"
                    || f.render_shape == horizon_client::blocks::RenderShape::Model)
                && !matches!(
                    &*f.name,
                    "minecraft:water" | "minecraft:lava" | "minecraft:bubble_column"
                )
            {
                // The nearest entry into one of the outline's boxes (slab method).
                let boxes = self.outline_boxes(pos);
                // `VoxelShape.clip`: a ray that starts inside the outline hits
                // it there, facing the way the look comes from.
                if let Some(hit) = inside_hit(nudged, sweep, &boxes, pos) {
                    return Some(hit);
                }
                let mut best: Option<(f64, [i32; 3])> = None;
                for b in boxes {
                    let (mut t0, mut t1, mut axis_in, mut sign) = (0.0f64, f64::INFINITY, None, 0);
                    let mut miss = false;
                    for a in 0..3 {
                        if dir[a] == 0.0 {
                            miss |= origin[a] < b[a] || origin[a] > b[a + 3];
                            continue;
                        }
                        let (lo, hi) =
                            ((b[a] - origin[a]) / dir[a], (b[a + 3] - origin[a]) / dir[a]);
                        let (near, far) = if lo < hi { (lo, hi) } else { (hi, lo) };
                        if near > t0 {
                            t0 = near;
                            axis_in = Some(a);
                            sign = if dir[a] > 0.0 { -1 } else { 1 };
                        }
                        t1 = t1.min(far);
                    }
                    if miss || t0 > t1 || t0 > reach {
                        continue;
                    }
                    // `AABB.clipDirection` takes only a plane met strictly after the start
                    // (0.0 < t): a box the ray starts inside or on the boundary of gives no
                    // entry here — the nudged inside test of `VoxelShape.clip` above is the
                    // only hit such a start can make, never a face-less one at t = 0 (#16551).
                    let Some(a) = axis_in else {
                        continue;
                    };
                    let mut normal = [0; 3];
                    normal[a] = sign;
                    if best.is_none_or(|(bt, _)| t0 < bt) {
                        best = Some((t0, normal));
                    }
                }
                if let Some((t, mut face)) = best {
                    // `BlockGetter.clipWithInteractionOverride`: the hit stays at the
                    // outline's point, with the face of the interaction shape when
                    // the ray enters that first.
                    let name = &*f.name;
                    let facing = self
                        .pack
                        .assets
                        .registry
                        .state(state)
                        .and_then(|s| s.properties.get("facing"))
                        .map(String::as_str);
                    let inside = interaction_shapes::boxes(name, facing);
                    if !inside.is_empty() {
                        let at = pos.map(f64::from);
                        let world: Vec<[f64; 6]> = inside
                            .iter()
                            .map(|b| std::array::from_fn(|i| at[i % 3] + b[i]))
                            .collect();
                        if let Some((ti, normal)) =
                            interaction_shapes::entry(origin, dir, &world, reach, face)
                            && ti < t
                        {
                            face = normal;
                        }
                    }
                    return Some(Hit {
                        pos,
                        face,
                        point: [
                            origin[0] + dir[0] * t,
                            origin[1] + dir[1] * t,
                            origin[2] + dir[2] * t,
                        ],
                        // No face entered: the eyes are in the shape.
                        inside: face == [0; 3],
                    });
                }
            }
            if fluids
                && f.fluid != Fluid::None
                && !self.res.facts().is_air(state)
                && !self.collision.cached_shapes().has_context_shapes(state)
            {
                return Some(Hit {
                    pos,
                    face,
                    point: [
                        origin[0] + dir[0] * t,
                        origin[1] + dir[1] * t,
                        origin[2] + dir[2] * t,
                    ],
                    // A fluid hit's inside flag is never read: a placement
                    // onto a fluid aims at the block above it.
                    inside: false,
                });
            }
            let axis = if t_max[0] < t_max[1] {
                if t_max[0] < t_max[2] { 0 } else { 2 }
            } else if t_max[1] < t_max[2] {
                1
            } else {
                2
            };
            t = t_max[axis];
            t_max[axis] += delta[axis];
            pos[axis] += step[axis];
            face = [0; 3];
            face[axis] = -step[axis];
        }
        None
    }
}

/// `VoxelShape.clip`'s inside case: the start point nudged along the ray lies in
/// one of the outline's boxes, so the hit is there, facing the look.
fn inside_hit(nudged: Vec3, sweep: Vec3, boxes: &[[f64; 6]], pos: [i32; 3]) -> Option<Hit> {
    let inside = boxes.iter().any(|b| {
        Aabb::new(Vec3::new(b[0], b[1], b[2]), Vec3::new(b[3], b[4], b[5])).contains(nudged)
    });
    inside.then(|| Hit {
        pos,
        face: Direction::approximate_nearest(sweep).opposite().normal(),
        point: nudged.components(),
        inside: true,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #16551: eyes inside a sapling's outline — `VoxelShape.clip`'s inside
    /// start. Yaw -180, pitch 40 looks mostly along -Z and down, so
    /// `Direction.getApproximateNearest` rounds the sweep to NORTH and the
    /// inside hit faces SOUTH: the click plants on the sapling's south face.
    #[test]
    fn an_inside_start_hits_the_outline_facing_the_look() {
        // Entity.calculateViewVector(-180, 40): (0, -sin40, -cos40).
        let dir = [0.0, -0.642_787_609_686_539_4, -0.766_044_443_118_978];
        let reach = 4.5;
        let sweep = Vec3::new(dir[0] * reach, dir[1] * reach, dir[2] * reach);
        let eye = Vec3::new(0.5, 200.07625, 0.5);
        let nudged = eye.add_vector(sweep.scale(0.001));
        // A sapling's outline at (0, 200, 0), in world space.
        let boxes = [[0.1, 200.0, 0.1, 0.9, 200.8, 0.9]];
        let hit = inside_hit(nudged, sweep, &boxes, [0, 200, 0]).expect("inside");
        assert!(hit.inside);
        assert_eq!(hit.face, [0, 0, 1], "SOUTH, opposite the look's NORTH");
        assert_eq!(hit.beside(), [0, 200, 1], "where the click plants");
    }

    /// A nudged point outside every outline box is no inside hit: the ray goes
    /// on to the slab entry, as `VoxelShape.clip` falls to `AABB.clip`.
    #[test]
    fn a_nudged_point_outside_the_outline_is_not_an_inside_hit() {
        let sweep = Vec3::new(0.0, -2.892_543_6, -3.447_200_0);
        let boxes = [[0.1, 200.0, 0.1, 0.9, 200.8, 0.9]];
        let outside = Vec3::new(2.0, 200.5, 0.5);
        assert!(inside_hit(outside, sweep, &boxes, [0, 200, 0]).is_none());
    }
}
