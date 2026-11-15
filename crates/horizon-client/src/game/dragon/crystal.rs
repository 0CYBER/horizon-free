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
            motion: Vec3::ZERO,
            rotation: [0.0, 0.0],
        }
    }
    pub(in crate::game) fn to_nbt(&self, uuid: [i32; 4], raw: Option<&Tag>) -> Tag {
        let mut tag = horizon_client::entity_nbt::vehicle_to_nbt(
            "minecraft:end_crystal",
            self.position,
            self.motion,
            self.rotation,
            self.fall,
            uuid,
            raw,
        );
        if let Tag::Compound(map) = &mut tag {
            map.insert("ShowBottom".into(), Tag::Byte(i8::from(self.show_bottom)));
            if let Some(b) = self.beam {
                map.insert("beam_target".into(), Tag::IntArray(b.to_vec()));
            }
            map.insert(
                "Invulnerable".into(),
                Tag::Byte(i8::from(self.invulnerable)),
            );
        }
        tag
    }
    pub(in crate::game) fn from_nbt(tag: &Tag, time: i32) -> Option<Self> {
        let (position, motion, _) = horizon_client::entity_nbt::boat_from_nbt(tag)?;
        // Entity.load's setYRot and setXRot skip a non-finite angle and keep the pitch within
        // +-90; /summon's `snapTo` takes both `% 360` (as `ominous_item_spawner.rs` does).
        let [yaw, pitch] = horizon_client::entity_nbt::rotation(tag);
        let finite = |v: f32| if v.is_finite() { v } else { 0.0 };
        let rotation = [
            finite(yaw) % 360.0,
            (finite(pitch) % 360.0).clamp(-90.0, 90.0),
        ];
        Some(Self {
            id: 0,
            position,
            show_bottom: horizon_client::entity_nbt::flag_or(tag, "ShowBottom", true),
            time,
            beam: match tag.get("beam_target") {
                Some(Tag::IntArray(v)) if v.len() == 3 => Some([v[0], v[1], v[2]]),
                _ => None,
            },
            invulnerable: horizon_client::entity_nbt::flag(tag, "Invulnerable"),
            fall: horizon_client::entity_nbt::ground_from_nbt(tag),
            motion,
            rotation,
        })
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
