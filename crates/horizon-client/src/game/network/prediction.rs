//! The movement prediction's replay on the server (`lan::prediction`): each guest's client ticks
//! played again with the client's own movement code, in the guest's level.
use super::Game;
use horizon_client::lan::entity::EntityState;
use horizon_client::lan::guest::Guest;
use horizon_client::lan::prediction::{Replay, near, slack};
use horizon_client::motion::{MotionWorld, PlayerMotion};
use horizon_client::world::World;

impl Game<'_> {
    /// Guest `conn`'s client ticks since the server's last tick, played again, before the server
    /// moves the guest's own body.
    pub(super) fn guest_replay(&mut self, conn: u32) {
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        if !host.hub.recording() {
            return;
        }
        let tolerance = host
            .hub
            .anticheat
            .as_deref()
            .map_or(f64::INFINITY, |ac| ac.config.prediction.tolerance);
        // The item the guest is using (eating, a bow, a shield): its client moves at the item's
        // `use_effects` speed (ItemUseModifier, 0.2 for most), and may not sprint. The server's copy
        // of a guest doesn't follow it (Game.sync_player_body is the window's player's), so the
        // replay walked a guest eating at full speed: 0.173 blocks a tick off, the live log's most
        // common flag.
        let item_use = host
            .play
            .get(&conn)
            .and_then(|play| play.using())
            .and_then(|(slot, _)| {
                let g = host.hub.guest(conn)?;
                let stack = if slot == super::shield::OFFHAND_SLOT {
                    g.offhand.as_ref()
                } else {
                    g.inventory.get(slot)?.as_ref()
                }?;
                self.res.items().get(&stack.item)
            })
            .map(|item| (item.use_speed, !item.can_sprint));
        let Some(over) = host
            .hub
            .update_guest(conn, |g| {
                // What moves the player isn't its own movement (a vehicle, a bed), or the server holds it:
                // the replay starts again at the teleport that ends it (dismountTo, stopSleeping, respawn).
                let held = g.vehicle.is_some()
                    || g.sleeping
                    || g.dead
                    || g.spectator()
                    || !g.client_loaded();
                // A teleport not yet accepted (a setback): the client still moves on its own until it
                // hears it, and the replay follows, but nothing counts.
                let waiting = g.awaiting_teleport.is_some();
                let glider = super::glide_wear::gliding_slots(
                    self.res.items(),
                    &g.armor,
                    g.inventory.get(g.selected).and_then(Option::as_ref),
                    g.offhand.as_ref(),
                )
                .next()
                .is_some();
                if let Some(motion) = g.motion.as_deref_mut() {
                    motion.glider = glider;
                    motion.leather_boots = super::worn_boots::leather_boots(&g.armor);
                    motion.item_use_speed = item_use.map(|(speed, _)| speed);
                    motion.item_use_slows_sprint = item_use.is_some_and(|(_, slows)| slows);
                }
                let (Some(record), Some(server)) = (g.record.as_deref_mut(), g.motion.as_deref())
                else {
                    return None;
                };
                let mut run = |body: &mut PlayerMotion, keys, sprinting| {
                    // Game.sync_player_body's part that depends on the blocks.
                    body.nudge = super::closest_space_nudge(body, &self.world, self.res.facts());
                    MotionWorld {
                        entities: &[],
                        player: body,
                        scene: &mut self.world,
                        assets: &self.pack.assets,
                        collision_binding: self.collision,
                        resources: self.motion,
                        health: &self.health_resources,
                        loop_context: &self.loop_context,
                        damage_hooks: &self.damage_hooks,
                        tag_items: Some(world_items!(self)),
                        block_registries: Some(&self.pack.block_registries),
                    }
                    .replayed_tick(keys, sprinting)
                };
                if let Some(restart) = record.take_restart() {
                    // What the client keeps through a teleport (its jump delay, a glide...) is the last
                    // replay's, once it has played the ticks before it. The client's motion through the
                    // teleport is its own (an End gateway's), and a replay that was excused when the teleport
                    // came may be off in what it can't see: either way the replay finds the client again.
                    let mut excused = restart.kept_motion;
                    let mut replay = g
                        .replay
                        .take()
                        .and_then(|mut replay| {
                            let finished = replay.finish(record, server, &restart, &mut run);
                            excused |= replay.excused();
                            finished.then_some(replay)
                        })
                        .unwrap_or_else(|| Box::new(Replay::new(server, &restart)));
                    replay.restart(&restart);
                    if excused {
                        replay.excuse();
                    }
                    g.replay = Some(replay);
                }
                if held {
                    g.replay = None;
                    return None;
                }
                let replay = g.replay.as_deref_mut()?;
                // Rebase missing or unplayable history, excused until the moves agree.
                let Some(over) = replay.catch_up(record, server, tolerance, &mut run) else {
                    g.replay = Some(Box::new(Replay::rebased(server, g.position, record)));
                    return None;
                };
                if over.ticks == 0 || waiting {
                    return None;
                }
                Some(over)
            })
            .flatten()
        else {
            return;
        };
        let g = host.hub.guest(conn).expect("found above");
        // A client that hasn't said it loaded the level is still on its loading screen, where it
        // doesn't move (the server stopped waiting for it after 60 ticks): the replay falls or drifts
        // where it stands. Its first move counts as any other.
        let loading = g.loading && !over.moved;
        if loading || excused(g, &self.world, host.states.as_slice(), &host.hub.guests) {
            let _ = host.hub.update_guest(conn, |g| {
                if let Some(replay) = g.replay.as_deref_mut() {
                    replay.excuse();
                }
            });
            return;
        }
        let agreed = g.replay.as_deref().expect("played above").agreed();
        let events = host.hub.anticheat_prediction(conn, &over, agreed);
        for event in events {
            self.lan_log_event(event);
        }
    }
}

/// Whether guest `g`'s last moves may be off for a reason the replay can't play yet: an entity near
/// it (a push, a boat or shulker to stand on, a firework it flies with); a block change near it the
/// server hasn't sent yet (one sent but unseen, or an entity gone that the client may still have, the
/// replay excuses itself); a moving piston; a chunk around it the client doesn't have; a riptide
/// launch (the trident's push is the client's own).
fn excused(g: &Guest, world: &World, entities: &[EntityState], guests: &[Guest]) -> bool {
    let at = g.position;
    let slack = slack(std::array::from_fn(|i| at[i] - g.previous_position[i]));
    let unsent = world
        .block_log
        .iter()
        .flatten()
        .chain(&world.block_log_held)
        .any(|&(x, y, z)| near(at, [x, y, z], slack));
    let pistons = world
        .moving_pistons
        .iter()
        .any(|&(p, _)| near(at, p, slack));
    // A client doesn't move in a chunk it doesn't have.
    let (cx, cz) = ((at[0].floor() as i32) >> 4, (at[2].floor() as i32) >> 4);
    let unloaded =
        (-1..=1).any(|dx| (-1..=1).any(|dz| !g.sent_chunks.contains(&(cx + dx, cz + dz))));
    let riptide = g
        .motion
        .as_deref()
        .is_some_and(|m: &PlayerMotion| m.auto_spin_attack_ticks > 0);
    let entity = entity_near(entities, g.entity_id, at, slack);
    let player = player_near(g, guests, at, slack);
    unsent || pistons || unloaded || riptide || entity || player
}

/// Whether another player in guest `g`'s level is close to `at`. A spectator moves no one: it pushes
/// no one (Player.tick sets its noPhysics, and Entity.push stops at a pusher that has it), no one
/// stands on a player, and the server doesn't even show it to a client that isn't a spectator
/// (ServerPlayer.broadcastToPlayer). A client that still had the player when it turned spectator
/// lost an entity near it, which the replay excuses on its own (`Record::entity_gone`).
fn player_near(g: &Guest, guests: &[Guest], at: [f64; 3], slack: f64) -> bool {
    guests.iter().any(|o| {
        o.conn != g.conn
            && o.placed
            && !o.spectator()
            && o.dimension == g.dimension
            && close(at, slack, o.position)
    })
}

/// Whether `p` is in the box around `at` that a player's moves are excused for.
fn close(at: [f64; 3], slack: f64, p: [f64; 3]) -> bool {
    let r = 3.0 + slack;
    (p[0] - at[0]).abs() <= r
        && (p[2] - at[2]).abs() <= r
        && (-3.0 - slack..=4.0 + slack).contains(&(p[1] - at[1]))
}

/// Whether an entity in `entities` other than the guest's own (`own`) is close to `at`. The
/// distance test runs first: most entities are far, and the kind is a string compare. Items and
/// experience orbs neither push nor hold anyone up, and neither do the entities that keep `Entity`'s
/// `isPushable` and `canBeCollidedWith` (both false) and move no one on the client: paintings, item
/// frames, markers, displays, interactions and leash knots. The list holds the player itself.
fn entity_near(entities: &[EntityState], own: i32, at: [f64; 3], slack: f64) -> bool {
    entities.iter().any(|e| {
        close(at, slack, e.position)
            && e.id != own
            && !matches!(
                e.kind.as_str(),
                "minecraft:item"
                    | "minecraft:experience_orb"
                    | "minecraft:painting"
                    | "minecraft:item_frame"
                    | "minecraft:glow_item_frame"
                    | "minecraft:marker"
                    | "minecraft:block_display"
                    | "minecraft:item_display"
                    | "minecraft:text_display"
                    | "minecraft:interaction"
                    | "minecraft:leash_knot"
            )
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The scan `excused` ran over the entities before `entity_near`, verbatim.
    fn old_entity(entities: &[EntityState], own: i32, at: [f64; 3], slack: f64) -> bool {
        let close = |p: [f64; 3]| {
            let r = 3.0 + slack;
            (p[0] - at[0]).abs() <= r
                && (p[2] - at[2]).abs() <= r
                && (-3.0 - slack..=4.0 + slack).contains(&(p[1] - at[1]))
        };
        entities.iter().any(|e| {
            e.id != own
                && !matches!(
                    e.kind.as_str(),
                    "minecraft:item" | "minecraft:experience_orb"
                )
                && close(e.position)
        })
    }

    /// xorshift64*: a fixed stream, so a failure reruns the same inputs.
    struct Rng(u64);

    impl Rng {
        fn next_u64(&mut self) -> u64 {
            self.0 ^= self.0 >> 12;
            self.0 ^= self.0 << 25;
            self.0 ^= self.0 >> 27;
            self.0.wrapping_mul(0x2545_F491_4F6C_DD1D)
        }

        fn below(&mut self, n: u64) -> u64 {
            self.next_u64() % n
        }

        /// A multiple of 1/8 in `[-n/8, n/8]`: every edge of the box (multiples of 1/4) is exact.
        fn step(&mut self, n: i64) -> f64 {
            (self.below(2 * n as u64 + 1) as i64 - n) as f64 * 0.125
        }

        /// An offset from the guest: half the time on or next to an edge of the box.
        fn offset(&mut self, edges: [f64; 4]) -> f64 {
            if self.below(2) == 0 {
                edges[self.below(4) as usize] + self.step(2)
            } else {
                self.step(160)
            }
        }
    }

    fn entity(rng: &mut Rng, at: [f64; 3], slack: f64) -> EntityState {
        const KINDS: [&str; 4] = [
            "minecraft:item",
            "minecraft:experience_orb",
            "minecraft:zombie",
            "minecraft:pig",
        ];
        let r = 3.0 + slack;
        let xz = [r, -r, r, -r];
        let y = [-3.0 - slack, 4.0 + slack, -3.0 - slack, 4.0 + slack];
        let off = [rng.offset(xz), rng.offset(y), rng.offset(xz)];
        EntityState {
            id: rng.below(6) as i32,
            kind: KINDS[rng.below(4) as usize].to_string(),
            position: [at[0] + off[0], at[1] + off[1], at[2] + off[2]],
            ..Default::default()
        }
    }

    fn guest(conn: u32, position: [f64; 3]) -> Guest {
        use horizon_client::lan::{Joined, PlayerSender};
        use std::sync::{Arc, atomic::AtomicI32, mpsc};
        let (tx, _rx) = mpsc::channel();
        let mut g = Guest::new(
            Joined {
                conn,
                uuid: u128::from(conn),
                name: format!("p{conn}"),
                properties: Vec::new(),
                info: horizon_protocol::packets::configuration::ClientInformation::new(2),
                brand: "test".into(),
                sender: PlayerSender::new(tx, Arc::default()),
                latency: Arc::new(AtomicI32::new(0)),
                address: "127.0.0.1:25565".parse().unwrap(),
                expected: false,
            },
            conn as i32,
            position,
            2,
        );
        g.placed = true;
        g
    }

    /// Another player a block away excuses a guest's moves (it may push it), but not one in
    /// spectator mode: a spectator pushes no one, and the guest's client doesn't even have it.
    #[test]
    fn a_spectator_near_a_player_excuses_nothing() {
        use horizon_client::lan::guest::{SPECTATOR, SURVIVAL};
        let at = [0.5, 64.0, 0.5];
        let mut guests = vec![guest(1, at), guest(2, [1.5, 64.0, 0.5])];
        assert!(player_near(&guests[0], &guests, at, 0.0));
        guests[1].change_game_mode(SPECTATOR);
        assert!(!player_near(&guests[0], &guests, at, 0.0));
        guests[1].change_game_mode(SURVIVAL);
        assert!(player_near(&guests[0], &guests, at, 0.0));
        // Only another player counts, in the same level and placed.
        assert!(!player_near(&guests[0], &guests[..1], at, 0.0));
        guests[1].dimension = "minecraft:the_nether".into();
        assert!(!player_near(&guests[0], &guests, at, 0.0));
    }

    #[test]
    fn entity_near_matches_the_scan_it_replaced() {
        const SLACKS: [f64; 6] = [0.0, 0.25, 0.5, 1.5, 3.0, 16.0];
        let mut rng = Rng(0x9E37_79B9_7F4A_7C15);
        let (mut yes, mut no) = (0, 0);
        for seed in 0..2_000 {
            let slack = SLACKS[rng.below(6) as usize];
            let at = [rng.step(160), rng.step(160), rng.step(160)];
            let n = rng.below(40) as usize;
            let entities: Vec<EntityState> = (0..n).map(|_| entity(&mut rng, at, slack)).collect();
            for own in 0..6 {
                let new = entity_near(&entities, own, at, slack);
                let old = old_entity(&entities, own, at, slack);
                assert_eq!(new, old, "seed {seed}, own {own}, at {at:?}, slack {slack}");
                if new {
                    yes += 1;
                } else {
                    no += 1;
                }
            }
        }
        assert!(yes > 0 && no > 0, "the inputs must give both answers");
    }

    #[test]
    fn box_edges_are_inclusive_and_own_and_drops_are_skipped() {
        let at = [0.0, 0.0, 0.0];
        let zombie = |id, position| EntityState {
            id,
            kind: "minecraft:zombie".to_string(),
            position,
            ..Default::default()
        };
        let s = 0.5;
        assert!(entity_near(&[zombie(1, [3.5, 0.0, 0.0])], 0, at, s));
        assert!(!entity_near(&[zombie(1, [3.625, 0.0, 0.0])], 0, at, s));
        assert!(entity_near(&[zombie(1, [0.0, 4.5, 0.0])], 0, at, s));
        assert!(!entity_near(&[zombie(1, [0.0, 4.625, 0.0])], 0, at, s));
        assert!(entity_near(&[zombie(1, [0.0, -3.5, 0.0])], 0, at, s));
        assert!(!entity_near(&[zombie(1, [0.0, -3.625, 0.0])], 0, at, s));
        // The guest's own entity, and items and orbs, never excuse.
        assert!(!entity_near(&[zombie(7, [0.0, 0.0, 0.0])], 7, at, s));
        let item = EntityState {
            kind: "minecraft:item".to_string(),
            ..zombie(2, [0.0, 0.0, 0.0])
        };
        assert!(!entity_near(&[item], 0, at, s));
    }

    /// An entity a vanilla client can neither be pushed by (`isPushable`) nor stand on
    /// (`canBeCollidedWith`) can't move the player: beside one, the moves count. One that can, and a
    /// firework (it boosts its glider on the client), still excuses them.
    #[test]
    fn only_entities_that_can_move_a_player_excuse_its_moves() {
        let at = [0.0, 64.0, 0.0];
        let near = |kind: &str| {
            let e = EntityState {
                id: 1,
                kind: kind.to_string(),
                position: [1.0, 64.0, 0.5],
                ..Default::default()
            };
            entity_near(&[e], 0, at, 0.0)
        };
        for inert in [
            "minecraft:painting",
            "minecraft:item_frame",
            "minecraft:glow_item_frame",
            "minecraft:marker",
            "minecraft:block_display",
            "minecraft:item_display",
            "minecraft:text_display",
            "minecraft:interaction",
            "minecraft:leash_knot",
        ] {
            assert!(!near(inert), "{inert} excused the moves beside it");
        }
        for moving in [
            "minecraft:cow",
            "minecraft:armor_stand",
            "minecraft:oak_boat",
            "minecraft:minecart",
            "minecraft:shulker",
            "minecraft:firework_rocket",
            "minecraft:arrow",
        ] {
            assert!(
                near(moving),
                "{moving} no longer excuses the moves beside it"
            );
        }
    }
}
