//! An end crystal's construction and the blocks its tick reads.
use super::*;

impl Crystal {
    /// `new EndCrystal(level, x, y, z)`: no beam, not invulnerable, on no
    /// ground with no fall (what `Entity`'s constructor leaves).
    pub(in crate::game) fn new(id: u32, position: Vec3, show_bottom: bool, time: i32) -> Self {
        Self {
            id,
            position,
            show_bottom,
            time,
            beam: None,
            invulnerable: false,
            fall: (false, 0.0),
        }
    }
    /// The start of `time` (the spin and bob) for a crystal at `position`: the random draw
    /// vanilla makes, or under `HORIZON_STILL` one seeded from the position alone. Which crystal
    /// gets which draw, and a server's entity ids, follow the order chunks load in, which differs
    /// between launches; the position is the same in every copy of the world. The draw is made
    /// either way, so the random stays where it would be.
    pub(in crate::game) fn start_time(position: Vec3, drawn: i32, still: bool) -> i32 {
        if still {
            horizon_client::mobs::entity_random(0, position)
                .next_int_bound(100_000)
                .unwrap_or(0)
        } else {
            drawn
        }
    }
}

impl Game<'_> {
    /// `EndCrystal.tick`'s `applyEffectsFromBlocks` for crystal `i`, which never
    /// moves: a cobweb or powder snow in its box (2 x 2 x 2, deflated by
    /// `1.0E-5F`) calls `makeStuckInBlock`, which resets its fall distance.
    /// Powder snow's inside shape for a fall over 2.5 is only 0.9 tall
    /// (`FALLING_COLLISION_SHAPE`). A bubble column's reset and `stepOn` for a
    /// grounded crystal are not ported.
    pub(super) fn crystal_block_effects(&mut self, i: usize) {
        let (p, fall) = (self.crystals[i].position, self.crystals[i].fall.1);
        let e = f64::from(1.0e-5_f32);
        let lo = [p.x - 1.0 + e, p.y + e, p.z - 1.0 + e].map(|v| v.floor() as i32);
        let hi = [p.x + 1.0 - e, p.y + 2.0 - e, p.z + 1.0 - e].map(|v| v.floor() as i32);
        let facts = self.res.facts();
        let stuck = (lo[0]..=hi[0]).any(|x| {
            (lo[1]..=hi[1]).any(|y| {
                (lo[2]..=hi[2]).any(|z| match &*facts.get(self.world.block(x, y, z)).name {
                    "minecraft:cobweb" => true,
                    "minecraft:powder_snow" => fall <= 2.5 || p.y + e < f64::from(y) + 0.9,
                    _ => false,
                })
            })
        });
        if stuck {
            self.crystals[i].fall.1 = 0.0;
        }
    }
}
