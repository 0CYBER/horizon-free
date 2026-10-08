//! Primed TNT (`PrimedTnt`): 80-tick fuse, gravity and drag, explosion of
//! power 4, `TntRenderer` swell and white flash, and `TntBlock.useItemOn`
//! (flint and steel / fire charge).
use super::{Game, explosion::BlockInteraction};
use horizon_client::{Result, collisions::SceneCollisions, items::ItemStack, mesher::Vertex};
use horizon_core::{
    geometry::{Aabb, Vec3},
    position::BlockPos,
    random::RandomSource,
};
use horizon_world::level::flags as level_flags;

#[derive(Clone, Debug)]
pub(super) struct PrimedTnt {
    pub position: Vec3,
    pub previous: Vec3,
    pub velocity: Vec3,
    pub fuse: i32,
    pub on_ground: bool,
}
impl PrimedTnt {
    const SIZE: f64 = 0.98;
    fn bounds(&self) -> Aabb {
        let h = Self::SIZE / 2.0;
        Aabb::new(
            Vec3::new(self.position.x - h, self.position.y, self.position.z - h),
            Vec3::new(
                self.position.x + h,
                self.position.y + Self::SIZE,
                self.position.z + h,
            ),
        )
    }
}

impl Game<'_> {
    /// `new PrimedTnt(level, x + 0.5, y, z + 0.5, owner)`; `short` applies
    /// `wasExploded`'s random short fuse. Plays `entity.tnt.primed` for primes.
    pub(super) fn spawn_primed_tnt(&mut self, pos: BlockPos, short: bool) {
        let r = &mut self.world.random;
        let rot = r.next_double() * f64::from(std::f32::consts::TAU);
        let position = Vec3::new(
            f64::from(pos.x) + 0.5,
            f64::from(pos.y),
            f64::from(pos.z) + 0.5,
        );
        let mut tnt = PrimedTnt {
            position,
            previous: position,
            velocity: Vec3::new(-rot.sin() * 0.02, f64::from(0.2f32), -rot.cos() * 0.02),
            fuse: 80,
            on_ground: false,
        };
        if short {
            // PrimedTnt.getRandomShortFuse
            tnt.fuse = r.next_int_bound((tnt.fuse / 4).max(1)).unwrap_or(0) + tnt.fuse / 8;
        } else {
            self.audio.play(
                "minecraft:entity.tnt.primed",
                Some([position.x, position.y, position.z]),
                1.0,
                1.0,
            );
        }
        self.tnts.push(tnt);
    }
    /// `PrimedTnt.tick` for every primed TNT, in order; an expired fuse
    /// explodes immediately (later TNT in the list feel its knockback this
    /// tick). TNT primed by an explosion starts ticking next tick.
    pub(super) fn tick_tnt(&mut self) -> Result<()> {
        let mut count = self.tnts.len();
        let mut i = 0;
        while i < count {
            let t = &mut self.tnts[i];
            t.previous = t.position;
            t.velocity.y -= 0.04;
            let mut scene = SceneCollisions {
                world: &self.world,
                binding: self.collision,
            };
            let wanted = t.velocity;
            let moved = scene
                .resolve(t.bounds(), wanted, 0.0, t.on_ground)
                .unwrap_or(Vec3::ZERO);
            t.position = Vec3::new(
                t.position.x + moved.x,
                t.position.y + moved.y,
                t.position.z + moved.z,
            );
            t.on_ground = wanted.y < 0.0 && moved.y != wanted.y;
            if moved.x != wanted.x {
                t.velocity.x = 0.0;
            }
            if moved.z != wanted.z {
                t.velocity.z = 0.0;
            }
            if moved.y != wanted.y {
                t.velocity.y = 0.0;
            }
            t.velocity = Vec3::new(
                t.velocity.x * 0.98,
                t.velocity.y * 0.98,
                t.velocity.z * 0.98,
            );
            if t.on_ground {
                t.velocity = Vec3::new(t.velocity.x * 0.7, t.velocity.y * -0.5, t.velocity.z * 0.7);
            }
            t.fuse -= 1;
            if t.fuse <= 0 {
                let center = [
                    t.position.x,
                    t.position.y + PrimedTnt::SIZE * 0.0625,
                    t.position.z,
                ];
                self.tnts.remove(i);
                count -= 1;
                self.server_explode(center, 4.0, BlockInteraction::Destroy, false)?;
            } else {
                i += 1;
            }
        }
        Ok(())
    }
    /// `TntBlock.useItemOn`: flint and steel or a fire charge primes TNT.
    pub(super) fn ignite_tnt(&mut self, stack: &ItemStack) -> Result<bool> {
        let item = &*stack.item;
        if item != "minecraft:flint_and_steel" && item != "minecraft:fire_charge" {
            return Ok(false);
        }
        let Some(hit) = self.pick() else {
            return Ok(false);
        };
        let state = self.world.block(hit.pos[0], hit.pos[1], hit.pos[2]);
        if &*self.res.facts.get(state).name != "minecraft:tnt" {
            return Ok(false);
        }
        let pos = BlockPos::new(hit.pos[0], hit.pos[1], hit.pos[2]);
        self.spawn_primed_tnt(pos, false);
        let air = self.world.air;
        self.with_level(|b, level, ticks| {
            b.set_block(level, ticks, pos, air, level_flags::UPDATE_ALL_IMMEDIATE)
        });
        self.process_level_events()?;
        self.swing_arm();
        if item == "minecraft:flint_and_steel" {
            self.damage_held(1);
        } else {
            self.consume_held();
        }
        Ok(true)
    }
    /// `TntRenderer`: the block centred on the entity, swelling over the last
    /// 10 ticks and flashing white every 5 ticks.
    pub(super) fn tnt_geometry(&self, camera: glam::DVec3, alpha: f64, out: &mut Vec<Vertex>) {
        if self.tnts.is_empty() {
            return;
        }
        let Some(state) = self.res.assets.registry.default_state("minecraft:tnt") else {
            return;
        };
        // Face sprites from the block model: up, down, north, south, west, east.
        let Ok(models) = self
            .res
            .assets
            .model_catalog
            .collect(state, BlockPos::new(0, 0, 0))
        else {
            return;
        };
        let mut sprites = [None; 6];
        for face in models.iter().flat_map(|(_, m)| m.faces.iter()) {
            use horizon_assets::model::Direction as D;
            let index = match face.quad.facing {
                D::Up => 0,
                D::Down => 1,
                D::North => 2,
                D::South => 3,
                D::West => 4,
                D::East => 5,
            };
            if sprites[index].is_none() {
                sprites[index] = self
                    .res
                    .layer_names
                    .get(&face.layer)
                    .and_then(|name| self.res.atlas.sprites.get(name).copied());
            }
        }
        for t in &self.tnts {
            let fuse = t.fuse as f32 - alpha as f32 + 1.0;
            let scale = if fuse < 10.0 {
                let g = (1.0 - fuse / 10.0).clamp(0.0, 1.0);
                1.0 + g * g * g * g * 0.3
            } else {
                1.0
            };
            let lit = fuse >= 0.0 && (fuse / 5.0) as i32 % 2 == 0;
            let base = glam::DVec3::new(
                t.previous.x + (t.position.x - t.previous.x) * alpha,
                t.previous.y + (t.position.y - t.previous.y) * alpha + 0.5,
                t.previous.z + (t.position.z - t.previous.z) * alpha,
            ) - camera;
            let o = base.as_vec3();
            let light = self.world.light(
                t.position.x.floor() as i32,
                (t.position.y + 0.5).floor() as i32,
                t.position.z.floor() as i32,
            );
            let packed = u32::from(light >> 4) * 16 | (u32::from(light & 15) * 16) << 8;
            let v = |x: f32, y: f32, z: f32| o + glam::Vec3::new(x - 0.5, y - 0.5, z - 0.5) * scale;
            let faces = [
                (
                    [v(0., 1., 1.), v(1., 1., 1.), v(1., 1., 0.), v(0., 1., 0.)],
                    1.0,
                ),
                (
                    [v(0., 0., 0.), v(1., 0., 0.), v(1., 0., 1.), v(0., 0., 1.)],
                    0.5,
                ),
                (
                    [v(1., 0., 0.), v(0., 0., 0.), v(0., 1., 0.), v(1., 1., 0.)],
                    0.8,
                ),
                (
                    [v(0., 0., 1.), v(1., 0., 1.), v(1., 1., 1.), v(0., 1., 1.)],
                    0.8,
                ),
                (
                    [v(0., 0., 0.), v(0., 0., 1.), v(0., 1., 1.), v(0., 1., 0.)],
                    0.6,
                ),
                (
                    [v(1., 0., 1.), v(1., 0., 0.), v(1., 1., 0.), v(1., 1., 1.)],
                    0.6,
                ),
            ];
            for ((corners, shade), sprite) in faces.into_iter().zip(sprites) {
                let Some(r) = sprite else { continue };
                let c = (255.0 * shade) as u32;
                let color = 0xFF00_0000 | c << 16 | c << 8 | c;
                for (p, uv) in
                    corners
                        .iter()
                        .zip([[r.u0, r.v1], [r.u1, r.v1], [r.u1, r.v0], [r.u0, r.v0]])
                {
                    // Overlay 4: OverlayTexture white flash.
                    out.push(Vertex {
                        pos: p.to_array(),
                        uv,
                        layer: if lit { 4 } else { 0 },
                        color,
                        light: packed,
                    });
                }
            }
        }
    }
}
