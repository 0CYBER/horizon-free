//! `StandRestKey`: what a resting armor stand's tick reads, and the test that a stand skipped
//! while it rests ticks like the full tick (see `armor_stands.rs`, `tick_armor_stands`).
use super::Game;
use horizon_core::geometry::Vec3;

impl Game<'_> {
    /// The key of stand `i`'s physics and environment phases, or None when they could do anything
    /// a key does not cover: a mod listening to entity moves, a cart or an obstacle entity about, a dynamic
    /// block shape in the world, worn equipment (a compass reads the world), a rider, a box of
    /// more than eight block sections, a burning stand (what `environment_tick_inner` asks the
    /// damage hooks and the saved invulnerability state about is not in the key).
    pub(super) fn stand_rest_key(
        &self,
        i: usize,
        obstacle_free: bool,
    ) -> Option<horizon_client::entities::StandRestKey> {
        let a = &self.armor_stands[i];
        if !obstacle_free
            || mod_api::__private::listened(mod_api::HookId::EntityMove)
            || !self.carts.is_empty()
            || !self.world.moving_pistons.is_empty()
            || !self.world.shulkers.is_empty()
            || a.vehicle.is_some()
            || a.fire_ticks > 0
            || (0..horizon_items::equipment::SLOT_NAMES.len()).any(|s| a.equipment[s].is_some())
        {
            return None;
        }
        // A turtle egg under the stand rolls the level's random every tick.
        let (cells, n) = a.on_pos_candidates();
        let registry = self.pack.assets.registry;
        if cells[..n].iter().any(|c| {
            registry
                .state(self.world.block(c.x, c.y, c.z))
                .is_some_and(|s| s.name == "minecraft:turtle_egg")
        }) {
            return None;
        }
        let b = a.bounds();
        let (lo, hi) = (b.min_position(), b.max_position());
        let blocks = self.world.region_stamp(
            [
                lo.x.floor() as i32 - 2,
                lo.y.floor() as i32 - 2,
                lo.z.floor() as i32 - 2,
            ],
            [
                hi.x.floor() as i32 + 2,
                hi.y.floor() as i32 + 2,
                hi.z.floor() as i32 + 2,
            ],
        )?;
        let bits = |v: Vec3| [v.x.to_bits(), v.y.to_bits(), v.z.to_bits()];
        Some(horizon_client::entities::StandRestKey {
            position: bits(a.position),
            velocity: bits(a.velocity),
            on_ground: a.on_ground,
            fire_ticks: a.fire_ticks,
            health: a.health.to_bits(),
            fall_distance: a.enchantment_fall_distance.to_bits(),
            support: a.support,
            flags: [
                a.marker,
                a.no_gravity,
                a.invisible,
                a.small,
                false,
                a.no_physics.is_some(),
                a.no_physics.unwrap_or(false),
            ],
            blocks,
            border: self.border.geometry().bounds(0.0).map(f64::to_bits),
        })
    }
}

impl Game<'_> {
    /// Every stand forgets its key: a reload (datapacks or resource packs) can change the tags
    /// and block data the phases read, which the key does not cover.
    pub(in crate::game) fn forget_stand_rests(&mut self) {
        for s in &mut self.armor_stands {
            s.rest = None;
        }
        // The levels not in place hold stands too; their tags change with the reload as well.
        self.levels.forget_stand_rests();
        // The kept network entries read reloaded data too (names, equipment attributes, defaults).
        self.stand_states.clear();
    }
}

#[cfg(test)]
mod stand_rest_tests {
    use horizon_core::{geometry::Vec3, position::BlockPos, random::RandomSource};

    type Snap = Vec<([u64; 3], [u64; 3], bool, i32, u32, u64)>;

    /// What a stand's tick can change, as bits.
    fn snap(game: &crate::game::Game<'_>) -> Snap {
        game.armor_stands
            .iter()
            .map(|a| {
                (
                    [
                        a.position.x.to_bits(),
                        a.position.y.to_bits(),
                        a.position.z.to_bits(),
                    ],
                    [
                        a.velocity.x.to_bits(),
                        a.velocity.y.to_bits(),
                        a.velocity.z.to_bits(),
                    ],
                    a.on_ground,
                    a.fire_ticks,
                    a.health.to_bits(),
                    a.enchantment_fall_distance.to_bits(),
                )
            })
            .collect()
    }

    /// A stand that is skipped while it rests ticks like the full tick, through everything that
    /// can wake it: the block under it removed, water, lava, a push, a chunk's blocks replaced.
    /// Two games run the same script, one with the rest skip forced off; the stands' states are
    /// equal after every tick and the level's random stream ends in the same place.
    #[test]
    #[ignore = "daily"]
    fn a_resting_stand_ticks_like_the_full_tick() {
        let mut res_a = None;
        let mut res_b = None;
        let mut a = crate::game::game_tests::tests::server_game(&mut res_a);
        let mut b = crate::game::game_tests::tests::server_game(&mut res_b);
        for g in [&mut a, &mut b] {
            g.build_registries();
            g.set_allow_commands(true);
            g.level.health = 20.0;
            g.player = Some(
                horizon_client::motion::PlayerMotion::new(g.motion, Vec3::new(14.5, 200.0, 14.5))
                    .unwrap(),
            );
            g.block_ticks
                .add_empty_chunk(BlockPos::new(0, 0, 0).chunk())
                .unwrap();
            assert!(g.run_command("forceload add 0 0"));
            assert!(g.run_command("fill 0 199 0 15 199 15 stone"));
            for (x, z) in [(3, 3), (8, 8), (12, 4)] {
                assert!(g.run_command(&format!(
                    "summon armor_stand {x}.5 200.0 {z}.5 {{Health:20f}}"
                )));
            }
        }
        let mut compared = 0;
        let mut run =
            |a: &mut crate::game::Game<'_>, b: &mut crate::game::Game<'_>, n: u32, what: &str| {
                for t in 0..n {
                    a.tick_armor_stands().unwrap();
                    {
                        // The full tick: the memo is cleared first, so nothing is skipped.
                        for s in &mut b.armor_stands {
                            s.rest = None;
                        }
                        b.tick_armor_stands().unwrap();
                    }
                    assert_eq!(snap(a), snap(b), "{what}, tick {t}");
                    compared += 1;
                }
            };
        run(&mut a, &mut b, 40, "resting on stone");
        assert!(
            a.armor_stands.iter().any(|s| s.rest.is_some()),
            "the skip is exercised"
        );
        for g in [&mut a, &mut b] {
            assert!(g.run_command("setblock 3 199 3 air"));
        }
        run(&mut a, &mut b, 30, "the block under one stand removed");
        for g in [&mut a, &mut b] {
            assert!(g.run_command("setblock 8 200 8 water"));
        }
        run(&mut a, &mut b, 20, "water on a stand");
        for g in [&mut a, &mut b] {
            assert!(g.run_command("setblock 8 200 8 lava"));
        }
        run(&mut a, &mut b, 25, "lava on a stand");
        // A burning stand asks the damage hooks every tick: it does not rest.
        assert!(
            a.armor_stands
                .iter()
                .filter(|s| s.fire_ticks > 0)
                .all(|s| s.rest.is_none()),
            "a burning stand rests"
        );
        for g in [&mut a, &mut b] {
            assert!(g.run_command("setblock 8 200 8 air"));
        }
        run(&mut a, &mut b, 40, "lava gone");
        // A real `/reload` in the middle: every stand forgets its key, then the script goes on.
        assert!(
            a.armor_stands.iter().any(|s| s.rest.is_some()),
            "resting before the reload"
        );
        for g in [&mut a, &mut b] {
            assert!(g.run_command("reload"));
        }
        assert!(
            a.armor_stands.iter().all(|s| s.rest.is_none()),
            "a reload forgets every key"
        );
        run(&mut a, &mut b, 30, "after a reload");
        assert!(
            a.armor_stands.iter().any(|s| s.rest.is_some()),
            "the skip resumes"
        );
        for g in [&mut a, &mut b] {
            g.armor_stands[2].velocity.x += 0.4;
        }
        run(&mut a, &mut b, 40, "a push");
        for g in [&mut a, &mut b] {
            assert!(g.run_command("fill 0 199 0 15 199 15 air"));
        }
        run(&mut a, &mut b, 20, "the floor removed");
        for g in [&mut a, &mut b] {
            assert!(g.run_command("fill 0 190 0 15 190 15 stone"));
        }
        run(&mut a, &mut b, 30, "a floor far below");
        assert!(compared > 200);
        let (da, db) = (
            a.world.random.next_int_bound(1_000_000).unwrap(),
            b.world.random.next_int_bound(1_000_000).unwrap(),
        );
        assert_eq!(da, db, "the level's random stream");
    }
}
