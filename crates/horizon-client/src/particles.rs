//! Block particles (`TerrainParticle`): destroy bursts and mining crumbs that
//! sample a random quarter of the block's texture, fall with gravity and
//! expire after a randomized lifetime.
use crate::world::World;
use horizon_core::random::{LegacyRandom, RandomSource};

#[derive(Clone, Debug)]
pub struct Particle {
    pub position: [f64; 3],
    pub previous: [f64; 3],
    pub velocity: [f64; 3],
    pub age: u32,
    pub lifetime: u32,
    pub size: f32,
    pub sprite: String,
    /// Sub-rectangle of the sprite in 0..1 (u0, v0, u1, v1).
    pub uv: [f32; 4],
    pub tint: u32,
    pub gravity: f64,
    pub on_ground: bool,
}
impl Particle {
    /// TerrainParticle(level, x, y, z, xa, ya, za, state)
    pub fn terrain(random: &mut LegacyRandom, position: [f64; 3], velocity: [f64; 3], sprite: String, tint: u32) -> Self {
        // Particle constructor: random jitter of velocity, normalized speed.
        let mut v = [
            velocity[0] + f64::from(random.next_float() * 2.0 - 1.0) * 0.4,
            velocity[1] + f64::from(random.next_float() * 2.0 - 1.0) * 0.4,
            velocity[2] + f64::from(random.next_float() * 2.0 - 1.0) * 0.4,
        ];
        let speed = f64::from(random.next_float() + random.next_float() + 1.0) * 0.15;
        let length = (v[0] * v[0] + v[1] * v[1] + v[2] * v[2]).sqrt().max(1e-9);
        for (i, c) in v.iter_mut().enumerate() {
            *c = *c / length * speed * 0.4;
            if i == 1 {
                *c += 0.1;
            }
        }
        let u = random.next_float() * 3.0 / 4.0;
        let w = random.next_float() * 3.0 / 4.0;
        // SingleQuadParticle quadSize: 0.1 * (rand*0.5+0.5) * 2, halved for terrain.
        let size = 0.1 * (random.next_float() * 0.5 + 0.5);
        let lifetime = (4.0 / (random.next_float() * 0.9 + 0.1)) as u32;
        Self {
            position,
            previous: position,
            velocity: v,
            age: 0,
            lifetime,
            size,
            sprite,
            uv: [u, w, u + 0.25, w + 0.25],
            tint,
            gravity: 1.0,
            on_ground: false,
        }
    }
    pub fn tick(&mut self, world: &World, solid: impl Fn(u16) -> bool) -> bool {
        self.previous = self.position;
        self.age += 1;
        if self.age >= self.lifetime {
            return false;
        }
        self.velocity[1] -= 0.04 * self.gravity;
        for axis in 0..3 {
            let mut next = self.position;
            next[axis] += self.velocity[axis];
            let block = world.block(next[0].floor() as i32, next[1].floor() as i32, next[2].floor() as i32);
            if solid(block.0 as u16) {
                if axis == 1 && self.velocity[1] < 0.0 {
                    self.on_ground = true;
                }
                self.velocity[axis] = 0.0;
            } else {
                self.position = next;
            }
        }
        for c in &mut self.velocity {
            *c *= 0.98;
        }
        if self.on_ground {
            self.velocity[0] *= 0.7;
            self.velocity[2] *= 0.7;
        }
        true
    }
}
