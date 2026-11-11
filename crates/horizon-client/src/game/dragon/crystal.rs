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
    /// moves: a cobweb or powder snow in its box (2 x 2 x 2, deflated by 1e-5)
    /// calls `makeStuckInBlock`, which resets its fall distance.
    pub(super) fn crystal_block_effects(&mut self, i: usize) {
        let p = self.crystals[i].position;
        let lo = [p.x - 1.0 + 1e-5, p.y + 1e-5, p.z - 1.0 + 1e-5].map(|v| v.floor() as i32);
        let hi = [p.x + 1.0 - 1e-5, p.y + 2.0 - 1e-5, p.z + 1.0 - 1e-5].map(|v| v.floor() as i32);
        let facts = self.res.facts();
        let stuck = (lo[0]..=hi[0]).any(|x| {
            (lo[1]..=hi[1]).any(|y| {
                (lo[2]..=hi[2]).any(|z| {
                    let name = &facts.get(self.world.block(x, y, z)).name;
                    &**name == "minecraft:cobweb" || &**name == "minecraft:powder_snow"
                })
            })
        });
        if stuck {
            self.crystals[i].fall.1 = 0.0;
        }
    }
}
