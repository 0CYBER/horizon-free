use super::*;

/// The hook points. Each does nothing in Horizon.
impl HitLevel<'_> {
    /// `Block.entityInside`: an entity's sweep crossed a block with a part for it
    /// (`precise` is `Entity.checkInsideBlocks`' flag), told mid-sweep, on the level
    /// host a hit or a landing uses. The part draws the level's random in vanilla's
    /// order and queues its writes on the event: the typed entity writes and effects
    /// apply to the swept entity inside its own tick, and the level writes land at the
    /// touched block at the batch end, guarded on the state. Generated dispatch stages
    /// every requested API `InsideWrite` before committing; an invalid state refuses
    /// the whole callback's output.
    pub(in crate::game) fn native_entity_inside(
        &mut self,
        call: super::fire::EntityInsideCall<'_>,
    ) {
        self.fire_entity_inside(call);
    }
}

#[cfg(test)]
mod entity_inside_tests {
    use super::*;

    /// Landing writes reach the level at the batch end, in the order asked, and only
    /// where the block is still the state landed on.
    #[test]
    #[ignore = "daily"]
    fn landing_writes_apply_at_the_batch_end() {
        use crate::game::api_values::{TryIntoApi, TryIntoEngine};
        use horizon_client::block_contact::{landed, take_pending};
        use horizon_world::level::LandingWrite;
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            let state = |name| game.pack.assets.registry.default_state(name).unwrap();
            let (farmland, dirt, stone, air) = (
                state("minecraft:farmland"),
                state("minecraft:dirt"),
                state("minecraft:stone"),
                state("minecraft:air"),
            );
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            game.world.set(1, 64, 1, farmland).unwrap();
            game.world.set(2, 64, 1, farmland).unwrap();
            game.world.set(3, 64, 1, stone).unwrap();
            game.world.set(4, 64, 1, dirt).unwrap();
            game.world.set(5, 64, 1, dirt).unwrap();
            let drops_before = game.dropped.len();
            let at = |x| BlockPos::new(x, 64, 1);
            let requested = vec![
                mod_api::LandingWrite::SetBlock(stone.try_into_api().unwrap()),
                mod_api::LandingWrite::SetBlock(dirt.try_into_api().unwrap()),
            ];
            // Exercise the same complete-batch boundary as generated dispatch before
            // entering the engine queue, then prove its ordered writes reach the world.
            let staged = requested.try_into_engine(&*game).unwrap();
            landed(farmland, at(1), staged);
            landed(dirt, at(4), vec![LandingWrite::DestroyBlock { drop: true }]);
            landed(
                dirt,
                at(5),
                vec![LandingWrite::DestroyBlock { drop: false }],
            );
            // The block changed after the landing: dropped.
            landed(stone, at(2), vec![LandingWrite::SetBlock(dirt)]);
            landed(
                stone,
                at(3),
                vec![LandingWrite::DestroyBlock { drop: false }],
            );
            // Nothing asked: nothing kept.
            landed(farmland, at(1), Vec::new());
            assert_eq!(
                game.world.block(1, 64, 1),
                farmland,
                "nothing writes mid-batch"
            );
            game.world.block_log = Some(Vec::new());
            game.drain_entity_inside();
            assert_eq!(game.world.block(1, 64, 1), dirt);
            assert_eq!(game.world.block(2, 64, 1), farmland);
            assert_eq!(game.world.block(3, 64, 1), air, "the stone is destroyed");
            assert_eq!(game.world.block(4, 64, 1), air);
            assert_eq!(game.world.block(5, 64, 1), air);
            assert_eq!(
                game.dropped.len(),
                drops_before + 1,
                "only drop=true creates dirt resources"
            );
            let dropped = &game.dropped[drops_before];
            assert_eq!(dropped.stack.item.as_ref(), "minecraft:dirt");
            assert_eq!(
                dropped.position.x.floor() as i32,
                4,
                "resources came from drop=true, not the neighboring drop=false block"
            );
            // Both writes reach the clients (`setBlockAndUpdate` and `destroyBlock` are
            // flag 3); the dropped one sends nothing.
            let log = game.world.block_log.take().unwrap();
            assert!(log.contains(&(1, 64, 1)), "the set block is sent: {log:?}");
            assert!(
                log.contains(&(3, 64, 1)),
                "the destroyed block is sent: {log:?}"
            );
            assert!(
                !log.contains(&(2, 64, 1)),
                "the dropped write sends nothing"
            );
            assert!(take_pending().is_empty());
        });
    }
}

#[cfg(test)]
mod sweep_tests {
    use super::*;

    /// The `entity_inside` part the player's sweep tests install: it draws the
    /// level's random once a contact (vanilla's order), queues a push and a wither
    /// effect around the freeze, and can kill or panic on later contacts.
    #[derive(Clone, Default)]
    struct InsidePart {
        contacts: std::sync::Arc<std::sync::Mutex<Vec<[i32; 3]>>>,
        swept: std::sync::Arc<std::sync::Mutex<Vec<mod_api::SweptEntity>>>,
        movement_only: bool,
        wither: std::sync::Arc<std::sync::Mutex<Option<mod_api::EffectKey>>>,
        speed: std::sync::Arc<std::sync::Mutex<Option<mod_api::EffectKey>>>,
        kill_after: Option<usize>,
        panic_after: Option<usize>,
    }
    #[mod_api::horizon_part]
    impl mod_api::BlockPart for InsidePart {
        fn entity_inside(
            &self,
            host: &mut dyn mod_api::LevelRandomHost,
            event: &mut mod_api::EntityInsideEvent<'_>,
        ) {
            let mut contacts = self.contacts.lock().unwrap();
            contacts.push([event.pos.x, event.pos.y, event.pos.z]);
            let seen = contacts.len();
            drop(contacts);
            let swept = host
                .swept_entity()
                .expect("contact must expose its own entity");
            self.swept.lock().unwrap().push(swept);
            if self.movement_only
                && (!swept.living
                    || swept.crouching
                    || swept.shift_key_down
                    || swept.position == swept.previous_position)
            {
                return;
            }
            // A level draw, in vanilla's order: between the vanilla contacts.
            let _ = host.with_random(horizon_core::random::RandomSource::next_long);
            if self.panic_after.is_some_and(|after| seen > after) {
                panic!("an entity_inside part panics");
            }
            if self.kill_after.is_some_and(|after| seen > after) {
                event.run_after(
                    mod_api::InsideEffect::Freeze,
                    mod_api::EntityWrite::Hurt {
                        source: None,
                        amount: 1000.0,
                    },
                );
            }
            event.run_after(
                mod_api::InsideEffect::Freeze,
                mod_api::EntityWrite::Push([0.5, 0.0, 0.0]),
            );
            if let Some(wither) = *self.wither.lock().unwrap() {
                event.run_after(
                    mod_api::InsideEffect::Freeze,
                    mod_api::EntityWrite::AddEffect {
                        effect: wither,
                        duration: 40,
                        amplifier: 0,
                    },
                );
            }
            if let Some(speed) = *self.speed.lock().unwrap() {
                event.run_after(
                    mod_api::InsideEffect::Extinguish,
                    mod_api::EntityWrite::AddEffect {
                        effect: speed,
                        duration: 20,
                        amplifier: 0,
                    },
                );
            }
        }
    }
    struct InsideMod(InsidePart);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for InsideMod {
        fn register(&self, r: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
            *self.0.wither.lock().unwrap() = Some(r.effect_key("minecraft:wither"));
            *self.0.speed.lock().unwrap() = Some(r.effect_key("minecraft:speed"));
            let stone = r.block_key("minecraft:stone");
            r.block_part(stone, self.0.clone());
        }
    }

    /// The part's mods, installed with their parts as a game's open does, before
    /// `with_game`: the game that opens then publishes the block index itself (the
    /// arismoko/horizon#824 parts-resolution order).
    fn install_inside_mod(name: &'static str, part: &InsidePart) -> mod_api::Mods {
        let entry =
            |part: &InsidePart| mod_api::ModEntry::new(name, Box::new(InsideMod(part.clone())));
        crate::game::api_values::install_parts(&[entry(part)]).unwrap();
        vec![entry(part)].into()
    }

    /// The game's player sweeps its move through two stone blocks, over the game's
    /// own world and assets, through the production `BLOCK_CONTACT_HOOKS` fire.
    /// Returns the player's velocity x before and after, and whether it died.
    fn sweep_game_player(game: &mut Game<'_>) -> (f64, f64, bool) {
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        game.world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
        );
        for x in [1, 2] {
            game.world.set(x, 100, 0, stone).unwrap();
        }
        if game.player.is_none() {
            game.player = Some(
                horizon_client::motion::PlayerMotion::new(
                    game.motion,
                    horizon_core::geometry::Vec3::new(0.5, 100.0, 0.5),
                )
                .unwrap(),
            );
        }
        game.player
            .as_mut()
            .unwrap()
            .history
            .add(horizon_world::entity_move::MovementRecord {
                from: horizon_core::geometry::Vec3::new(0.5, 100.0, 0.5),
                to: horizon_core::geometry::Vec3::new(2.5, 100.0, 0.5),
                original: None,
            });
        sweep_game_player_runs(game, |w| w.apply_effects_from_blocks().map(|_| ()))
    }

    /// The same world and player setup, with the caller's passes run in the level
    /// scope: what a server turn walks, or the client's own tick.
    fn sweep_game_player_runs(
        game: &mut Game<'_>,
        runs: impl FnOnce(&mut horizon_client::motion::MotionWorld) -> crate::Result<()>,
    ) -> (f64, f64, bool) {
        use horizon_client::motion::MotionWorld;
        let before = game.player.as_ref().unwrap().velocity.x;
        let crate::game::Game {
            world,
            pack,
            motion,
            health_resources,
            damage_hooks,
            loop_context,
            collision,
            player,
            ..
        } = game;
        let entities: [horizon_core::geometry::Aabb; 0] = [];
        horizon_client::block_contact::in_level(Some("minecraft:overworld"), || {
            let mut w = MotionWorld {
                tag_items: None,
                block_registries: Some(&pack.block_registries),
                loop_context,
                damage_hooks,
                entities: &entities,
                player: player.as_mut().unwrap(),
                scene: world,
                assets: &pack.assets,
                collision_binding: collision,
                resources: motion,
                health: health_resources,
            };
            runs(&mut w).unwrap();
        });
        let player = game.player.as_ref().unwrap();
        (before, player.velocity.x, player.health.dead_or_dying())
    }

    /// A listening part is told mid-walk, in walk order, and its typed writes reach
    /// the player in this tick: both pushes add, and the wither effect event lands
    /// for the server's `give_player_effect`, which runs before this tick's effect
    /// tick.
    #[test]
    #[ignore = "daily"]
    fn a_part_pushes_the_player_and_its_effect_lands() {
        let part = InsidePart::default();
        let mods = install_inside_mod("inside-push", &part);
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                let _ = horizon_client::block_contact::HOOKS.set(super::BLOCK_CONTACT_HOOKS);
                let (before, after, dead) = sweep_game_player(game);
                assert!(!dead);
                assert_eq!(
                    *part.contacts.lock().unwrap(),
                    vec![[1, 100, 0], [2, 100, 0]],
                    "both contacts, in walk order"
                );
                assert!(
                    (after - before - 1.0).abs() < 1e-9,
                    "both pushes reached the player: {after}"
                );
                let effect = game
                    .player
                    .as_ref()
                    .unwrap()
                    .events
                    .iter()
                    .find_map(|event| match event {
                        horizon_client::motion::MotionEvent::InsideEffect {
                            id, duration, ..
                        } => Some((*id, *duration)),
                        _ => None,
                    })
                    .expect("the wither effect crossed to the server");
                assert_eq!(effect, ("minecraft:wither", 40));
                assert!(horizon_client::block_contact::take_pending().is_empty());
            });
        });
    }

    #[test]
    #[ignore = "daily"]
    fn a_part_distinguishes_movement_pose_and_sneak_during_a_player_sweep() {
        use crate::game::native_entity_refs::tests::with_game;
        use horizon_client::{motion::PlayerMotion, pose::Pose};
        use horizon_core::geometry::Vec3;

        let part = InsidePart {
            movement_only: true,
            ..InsidePart::default()
        };
        let mods = install_inside_mod("sweep-reads-pose", &part);
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                let _ = horizon_client::block_contact::HOOKS.set(super::BLOCK_CONTACT_HOOKS);
                let at = Vec3::new(2.5, 100.0, 0.5);
                for (moved, crouching, shift_key_down) in [
                    (true, false, false),
                    (true, true, false),
                    (true, false, true),
                    (false, false, false),
                ] {
                    let mut player = PlayerMotion::new(game.motion, at).unwrap();
                    player.old_position = Vec3::new(if moved { 0.5 } else { 2.5 }, 100.0, 0.5);
                    player.velocity = Vec3::new(0.25, -0.5, 0.125);
                    player.pose = if crouching {
                        Pose::Crouching
                    } else {
                        Pose::Standing
                    };
                    player.shift_key_down = shift_key_down;
                    player.fall_distance = 3.25;
                    game.player = Some(player);
                    part.swept.lock().unwrap().clear();

                    let (before, after, dead) = sweep_game_player(game);
                    let reads = part.swept.lock().unwrap();
                    assert_eq!(reads.len(), 2, "inspect both contacted blocks");
                    for read in &*reads {
                        assert_eq!(read.position, [2.5, 100.0, 0.5]);
                        assert_eq!(
                            read.previous_position,
                            [if moved { 0.5 } else { 2.5 }, 100.0, 0.5]
                        );
                        assert_eq!(read.velocity, [0.25, -0.5, 0.125]);
                        assert_eq!(read.fall_distance, 3.25);
                        assert_eq!(
                            (read.crouching, read.shift_key_down, read.living),
                            (crouching, shift_key_down, true)
                        );
                    }
                    assert!(!dead);
                    let push = if moved && !crouching && !shift_key_down {
                        1.0
                    } else {
                        0.0
                    };
                    assert_eq!(after - before, push);
                }
            });
        });
    }

    /// A part that kills the player with its first write: the walk's later contacts
    /// still happen, but the flush stops at the death, so the later writes and
    /// effects never apply.
    #[test]
    #[ignore = "daily"]
    fn a_killing_write_stops_the_flush() {
        let part = InsidePart {
            kill_after: Some(0),
            ..InsidePart::default()
        };
        let mods = install_inside_mod("inside-kill", &part);
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                let _ = horizon_client::block_contact::HOOKS.set(super::BLOCK_CONTACT_HOOKS);
                let (before, after, dead) = sweep_game_player(game);
                assert_eq!(
                    part.contacts.lock().unwrap().len(),
                    2,
                    "the walk itself reaches both contacts"
                );
                assert!(dead, "the killing write applied");
                assert!(
                    (after - before).abs() < 1e-9,
                    "the writes after the death never apply"
                );
            });
        });
    }

    /// A part that panics on its second contact: the panic is contained, the walk
    /// and the player's tick go on, and the panicking callback's writes are refused
    /// with it.
    #[test]
    #[ignore = "daily"]
    fn a_panicking_part_leaves_the_players_tick_restored() {
        // The panic lands on every contact: each one is caught in the staged fire,
        // and the mod keeps receiving the walk's contacts.
        let part = InsidePart {
            panic_after: Some(0),
            ..InsidePart::default()
        };
        let mods = install_inside_mod("inside-panic", &part);
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                let _ = horizon_client::block_contact::HOOKS.set(super::BLOCK_CONTACT_HOOKS);
                let (before, after, dead) = sweep_game_player(game);
                assert!(!dead);
                assert_eq!(
                    part.contacts.lock().unwrap().len(),
                    2,
                    "both contacts reached the part; each panic was contained"
                );
                assert!(
                    (after - before).abs() < 1e-9,
                    "the panicking callback's staged writes never applied"
                );
                assert!(
                    game.player
                        .as_ref()
                        .unwrap()
                        .events
                        .iter()
                        .any(|event| matches!(
                            event,
                            horizon_client::motion::MotionEvent::EnterBlock { .. }
                        )),
                    "the vanilla moments still ran"
                );
            });
        });
    }

    /// Without a listening part the walk runs its vanilla moments and leaves the
    /// player's writes and the pending queue alone: no part is installed, so the
    /// fire's gate answers false and its DamageLevel is never built.
    #[test]
    #[ignore = "daily"]
    fn without_a_part_the_walk_costs_no_mod_work() {
        // The production binding, as every sweep test sets it: nothing is installed
        // on this thread, so its gate answers false for every state and the fire's
        // DamageLevel is never built. (A dummy binding here would race the
        // process-wide `block_contact::HOOKS` OnceLock — first setter wins —
        // against the other tests' real one.)
        let _ = horizon_client::block_contact::HOOKS.set(super::BLOCK_CONTACT_HOOKS);
        crate::game::native_entity_refs::tests::with_game(|game| {
            let (before, after, dead) = sweep_game_player(game);
            assert!(!dead);
            assert!((after - before).abs() < 1e-9, "no writes without a part");
            assert!(
                game.player
                    .as_ref()
                    .unwrap()
                    .events
                    .iter()
                    .any(|event| matches!(
                        event,
                        horizon_client::motion::MotionEvent::EnterBlock { .. }
                    )),
                "the vanilla moments still ran"
            );
            assert!(horizon_client::block_contact::take_pending().is_empty());
        });
    }

    /// A part's `AddEffect` keeps its collector position across the event channel:
    /// one write asked after FREEZE and one after EXTINGUISH land in that order,
    /// `applyAndClear`'s own, ready for the server's `give_player_effect` before
    /// this tick's effect tick.
    #[test]
    #[ignore = "daily"]
    fn an_added_effect_keeps_its_collector_position() {
        let part = InsidePart::default();
        let mods = install_inside_mod("inside-order", &part);
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                let _ = horizon_client::block_contact::HOOKS.set(super::BLOCK_CONTACT_HOOKS);
                let (_, _, dead) = sweep_game_player(game);
                assert!(!dead);
                let effects: Vec<_> = game
                    .player
                    .as_ref()
                    .unwrap()
                    .events
                    .iter()
                    .filter_map(|event| match event {
                        horizon_client::motion::MotionEvent::InsideEffect {
                            id, duration, ..
                        } => Some((*id, *duration)),
                        _ => None,
                    })
                    .collect();
                assert_eq!(
                    effects,
                    vec![
                        ("minecraft:wither", 40),
                        ("minecraft:speed", 20),
                        ("minecraft:wither", 40),
                        ("minecraft:speed", 20),
                    ],
                    "each contact's writes flush in InsideEffect order: the FREEZE slot before the EXTINGUISH slot"
                );
            });
        });
    }

    /// A server's player is told once a tick: the early `ServerStuck` pass keeps
    /// the moves for `server_block_damage`, and only the damage pass tells the
    /// part — two blocks, two contacts, not four, as vanilla's `entityInside`
    /// runs once a move.
    #[test]
    #[ignore = "daily"]
    fn a_servers_player_is_told_once_a_tick() {
        let part = InsidePart::default();
        let mods = install_inside_mod("inside-once", &part);
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                let _ = horizon_client::block_contact::HOOKS.set(super::BLOCK_CONTACT_HOOKS);
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
                );
                for x in [1, 2] {
                    game.world.set(x, 100, 0, stone).unwrap();
                }
                if game.player.is_none() {
                    game.player = Some(
                        horizon_client::motion::PlayerMotion::new(
                            game.motion,
                            horizon_core::geometry::Vec3::new(0.5, 100.0, 0.5),
                        )
                        .unwrap(),
                    );
                }
                game.player.as_mut().unwrap().history.add(
                    horizon_world::entity_move::MovementRecord {
                        from: horizon_core::geometry::Vec3::new(0.5, 100.0, 0.5),
                        to: horizon_core::geometry::Vec3::new(2.5, 100.0, 0.5),
                        original: None,
                    },
                );
                // The server turn's own order (lan_actions/tick.rs): the stuck
                // pass before the landing is checked, then the damage pass over
                // the same moves.
                sweep_game_player_runs(game, |w| {
                    w.server_stuck_in_blocks(horizon_core::geometry::Vec3::new(0.5, 100.0, 0.5))?;
                    w.server_block_damage(horizon_core::geometry::Vec3::new(0.5, 100.0, 0.5))
                        .map(|_| ())
                });
                assert_eq!(
                    *part.contacts.lock().unwrap(),
                    vec![[1, 100, 0], [2, 100, 0]],
                    "one contact a block, the damage pass's, not the stuck pass's too"
                );
            });
        });
    }

    /// Without a part the two server passes leave the player as vanilla has it:
    /// the walk still reports its blocks and nothing is queued.
    #[test]
    #[ignore = "daily"]
    fn the_server_passes_without_a_part_keep_vanilla_behavior() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            for x in [1, 2] {
                game.world.set(x, 100, 0, stone).unwrap();
            }
            if game.player.is_none() {
                game.player = Some(
                    horizon_client::motion::PlayerMotion::new(
                        game.motion,
                        horizon_core::geometry::Vec3::new(0.5, 100.0, 0.5),
                    )
                    .unwrap(),
                );
            }
            game.player
                .as_mut()
                .unwrap()
                .history
                .add(horizon_world::entity_move::MovementRecord {
                    from: horizon_core::geometry::Vec3::new(0.5, 100.0, 0.5),
                    to: horizon_core::geometry::Vec3::new(2.5, 100.0, 0.5),
                    original: None,
                });
            let _ = horizon_client::block_contact::HOOKS.set(super::BLOCK_CONTACT_HOOKS);
            let velocity = game.player.as_ref().unwrap().velocity.x;
            sweep_game_player_runs(game, |w| {
                w.server_stuck_in_blocks(horizon_core::geometry::Vec3::new(0.5, 100.0, 0.5))?;
                w.server_block_damage(horizon_core::geometry::Vec3::new(0.5, 100.0, 0.5))
                    .map(|_| ())
            });
            assert_eq!(
                game.player.as_ref().unwrap().velocity.x,
                velocity,
                "no writes without a part"
            );
            let entered = game
                .player
                .as_ref()
                .unwrap()
                .events
                .iter()
                .filter(|event| {
                    matches!(
                        event,
                        horizon_client::motion::MotionEvent::EnterBlock { .. }
                    )
                })
                .count();
            assert_eq!(entered, 2, "each block still reported, vanilla's moments");
            assert!(horizon_client::block_contact::take_pending().is_empty());
        });
    }

    /// The item's batch sweep tests: the same part, the same assertions as the
    /// player's, over `Game::sweep_item_inside`.
    fn item_at_blocks(game: &mut Game<'_>) -> horizon_client::entities::ItemEntity {
        let stone = game
            .pack
            .assets
            .registry
            .default_state("minecraft:stone")
            .unwrap();
        game.world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
        );
        for x in [1, 2] {
            game.world.set(x, 100, 0, stone).unwrap();
        }
        let mut item = horizon_client::entities::ItemEntity::new(
            7,
            horizon_client::items::ItemStack::new("minecraft:stone", 1),
            horizon_core::geometry::Vec3::new(2.5, 100.0, 0.5),
            horizon_core::geometry::Vec3::ZERO,
            0.0,
        );
        item.previous = horizon_core::geometry::Vec3::new(0.5, 100.0, 0.5);
        item
    }

    /// A part's pushes reach the item in its own tick, one contact a block.
    #[test]
    #[ignore = "daily"]
    fn a_part_pushes_an_item_once_a_block() {
        let part = InsidePart::default();
        let mods = install_inside_mod("inside-item", &part);
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                let _ = horizon_client::block_contact::HOOKS.set(super::BLOCK_CONTACT_HOOKS);
                let mut item = item_at_blocks(game);
                let before = item.velocity.x;
                horizon_client::block_contact::in_level(Some("minecraft:overworld"), || {
                    game.sweep_item_inside(&mut item);
                });
                assert_eq!(
                    *part.contacts.lock().unwrap(),
                    vec![[1, 100, 0], [2, 100, 0]],
                    "one contact a block, in walk order"
                );
                assert!(
                    (item.velocity.x - before - 1.0).abs() < 1e-9,
                    "both pushes reached the item"
                );
                let swept = part.swept.lock().unwrap();
                assert_eq!(swept.len(), 2);
                for read in &*swept {
                    assert_eq!(read.position, [2.5, 100.0, 0.5]);
                    assert_eq!(read.previous_position, [0.5, 100.0, 0.5]);
                    assert!(!read.living);
                }
                assert!(horizon_client::block_contact::take_pending().is_empty());
            });
        });
    }

    /// A killing write stops the item's flush: the walk still reaches both blocks,
    /// the writes after the destruction never apply.
    #[test]
    #[ignore = "daily"]
    fn a_killing_write_stops_the_items_flush() {
        let part = InsidePart {
            kill_after: Some(0),
            ..InsidePart::default()
        };
        let mods = install_inside_mod("inside-item-kill", &part);
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                let _ = horizon_client::block_contact::HOOKS.set(super::BLOCK_CONTACT_HOOKS);
                let mut item = item_at_blocks(game);
                let before = item.velocity.x;
                horizon_client::block_contact::in_level(Some("minecraft:overworld"), || {
                    game.sweep_item_inside(&mut item);
                });
                assert_eq!(part.contacts.lock().unwrap().len(), 2);
                assert!(item.health <= 0, "the killing write applied");
                assert!(
                    (item.velocity.x - before).abs() < 1e-9,
                    "the writes after the destruction never apply"
                );
            });
        });
    }

    /// A panicking part inside the item batch: the staged fire contains it, the
    /// lent level goes back, and the lifted row the batch holds returns to the
    /// level's list (arismoko/horizon#2194's and arismoko/horizon#2213's carrier discipline).
    #[test]
    #[ignore = "daily"]
    fn a_panicking_part_leaves_the_item_batch_restored() {
        let part = InsidePart {
            panic_after: Some(0),
            ..InsidePart::default()
        };
        let mods = install_inside_mod("inside-item-panic", &part);
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                let _ = horizon_client::block_contact::HOOKS.set(super::BLOCK_CONTACT_HOOKS);
                let item = item_at_blocks(game);
                game.dropped.push(item);
                let rows = game.dropped.len();
                let mut items = game.dropped.lift();
                let before = items[0].velocity.x;
                horizon_client::block_contact::in_level(Some("minecraft:overworld"), || {
                    game.sweep_item_inside(&mut items[0]);
                });
                game.put_back_dropped(items);
                assert_eq!(
                    part.contacts.lock().unwrap().len(),
                    2,
                    "both contacts reached the part; each panic was contained"
                );
                assert_eq!(game.dropped.len(), rows, "the lifted row returned");
                let item = game.dropped.first().unwrap();
                assert!(
                    (item.velocity.x - before).abs() < 1e-9,
                    "the panicking callback's staged writes never applied"
                );
                // The level went back: its blocks read as they were.
                assert!(
                    game.world.block(1, 100, 0).0 != 0,
                    "the level reads as it was"
                );
                assert!(horizon_client::block_contact::take_pending().is_empty());
            });
        });
    }

    /// Without a part the item's sweep costs one gate test: nothing is written,
    /// nothing queued, exactly what main's recorded route did with no listener.
    #[test]
    #[ignore = "daily"]
    fn an_item_without_a_part_is_left_alone() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            let _ = horizon_client::block_contact::HOOKS.set(super::BLOCK_CONTACT_HOOKS);
            let mut item = item_at_blocks(game);
            let before = (item.velocity.x, item.health, item.fire_ticks);
            horizon_client::block_contact::in_level(Some("minecraft:overworld"), || {
                game.sweep_item_inside(&mut item);
            });
            assert_eq!(
                (item.velocity.x, item.health, item.fire_ticks),
                before,
                "no writes without a part"
            );
            assert!(horizon_client::block_contact::take_pending().is_empty());
        });
    }

    /// The projectile's batch sweep: a part's push reaches it, one contact a
    /// block, and a removed projectile's flush stops.
    #[test]
    #[ignore = "daily"]
    fn a_part_pushes_a_projectile_and_removal_stops_the_flush() {
        let part = InsidePart::default();
        let mods = install_inside_mod("inside-shot", &part);
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                let _ = horizon_client::block_contact::HOOKS.set(super::BLOCK_CONTACT_HOOKS);
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
                );
                for x in [1, 2] {
                    game.world.set(x, 100, 0, stone).unwrap();
                }
                let mut shot = horizon_client::projectiles::Projectile::new(
                    horizon_client::projectiles::ProjectileKind::Snowball,
                    horizon_core::geometry::Vec3::new(2.5, 100.0, 0.5),
                    None,
                );
                shot.id = 9;
                shot.previous = horizon_core::geometry::Vec3::new(0.5, 100.0, 0.5);
                let before = shot.velocity.x;
                horizon_client::block_contact::in_level(Some("minecraft:overworld"), || {
                    game.sweep_projectile_inside(&mut shot);
                });
                assert_eq!(
                    *part.contacts.lock().unwrap(),
                    vec![[1, 100, 0], [2, 100, 0]],
                    "one contact a block, in walk order"
                );
                assert!(
                    (shot.velocity.x - before - 1.0).abs() < 1e-9,
                    "both pushes reached the projectile"
                );
                // A removed arrow's flush stops: nothing applies past its death.
                part.contacts.lock().unwrap().clear();
                let mut arrow = horizon_client::projectiles::Projectile::new(
                    horizon_client::projectiles::ProjectileKind::Arrow {
                        base_damage: 1.0,
                        critical: false,
                        effect: None,
                    },
                    horizon_core::geometry::Vec3::new(2.5, 100.0, 0.5),
                    None,
                );
                arrow.id = 10;
                arrow.previous = horizon_core::geometry::Vec3::new(0.5, 100.0, 0.5);
                arrow.velocity = horizon_core::geometry::Vec3::ZERO;
                let arrow_state = &mut arrow.arrow;
                if let Some(a) = arrow_state.as_mut() {
                    a.removed = true;
                }
                let before = arrow.velocity.x;
                horizon_client::block_contact::in_level(Some("minecraft:overworld"), || {
                    game.sweep_projectile_inside(&mut arrow);
                });
                assert!(
                    part.contacts.lock().unwrap().is_empty(),
                    "a removed projectile is not walked"
                );
                assert!((arrow.velocity.x - before).abs() < 1e-9);
            });
        });
    }

    /// Without a part the projectile's sweep costs one gate test and nothing else,
    /// exactly what main's recorded route did with no listener.
    #[test]
    #[ignore = "daily"]
    fn a_projectile_without_a_part_is_left_alone() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            let _ = horizon_client::block_contact::HOOKS.set(super::BLOCK_CONTACT_HOOKS);
            let stone = game
                .pack
                .assets
                .registry
                .default_state("minecraft:stone")
                .unwrap();
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            game.world.set(1, 100, 0, stone).unwrap();
            let mut shot = horizon_client::projectiles::Projectile::new(
                horizon_client::projectiles::ProjectileKind::Snowball,
                horizon_core::geometry::Vec3::new(1.5, 100.0, 0.5),
                None,
            );
            shot.previous = horizon_core::geometry::Vec3::new(0.5, 100.0, 0.5);
            let before = (shot.velocity.x, shot.fire_ticks, shot.fall_distance);
            horizon_client::block_contact::in_level(Some("minecraft:overworld"), || {
                game.sweep_projectile_inside(&mut shot);
            });
            assert_eq!(
                (shot.velocity.x, shot.fire_ticks, shot.fall_distance),
                before,
                "no writes without a part"
            );
            assert!(horizon_client::block_contact::take_pending().is_empty());
        });
    }

    /// An arrow's contacts, on the same batch sweep the other projectiles':
    /// a flying arrow meets its blocks in walk order, once each, and a stuck
    /// arrow keeps the contact of the block it sits in.
    #[test]
    #[ignore = "daily"]
    fn an_arrow_is_told_once_a_block_stuck_and_flying() {
        let part = InsidePart::default();
        let mods = install_inside_mod("inside-arrow", &part);
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                let _ = horizon_client::block_contact::HOOKS.set(BLOCK_CONTACT_HOOKS);
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
                );
                for x in [1, 2] {
                    game.world.set(x, 100, 0, stone).unwrap();
                }
                let flying = arrow_shot(9, (0.5, 100.0, 0.5), (2.5, 100.0, 0.5));
                let before = flying.velocity.x;
                let mut flying = flying;
                horizon_client::block_contact::in_level(Some("minecraft:overworld"), || {
                    game.sweep_projectile_inside(&mut flying);
                });
                assert_eq!(
                    *part.contacts.lock().unwrap(),
                    vec![[1, 100, 0], [2, 100, 0]],
                    "the flying arrow meets its blocks in walk order, once each"
                );
                assert!(
                    (flying.velocity.x - before - 1.0).abs() < 1e-9,
                    "the part's pushes applied to the arrow"
                );
                part.contacts.lock().unwrap().clear();
                // A stuck arrow: no move, the block it sits in still tells the part.
                let mut stuck = arrow_shot(10, (1.5, 100.0, 0.5), (1.5, 100.0, 0.5));
                stuck.velocity = horizon_core::geometry::Vec3::ZERO;
                horizon_client::block_contact::in_level(Some("minecraft:overworld"), || {
                    game.sweep_projectile_inside(&mut stuck);
                });
                assert_eq!(
                    *part.contacts.lock().unwrap(),
                    vec![[1, 100, 0]],
                    "the stuck arrow keeps the contact of its block"
                );
                assert!(horizon_client::block_contact::take_pending().is_empty());
            });
        });
    }

    /// A panicking part inside the arrow batch: the listener's panic is contained
    /// in the staged fire, the lifted projectile row returns to the level's list
    /// exactly once, and the arrow keeps its state.
    #[test]
    #[ignore = "daily"]
    fn a_panicking_part_leaves_the_arrow_list_whole() {
        let part = InsidePart {
            panic_after: Some(0),
            ..InsidePart::default()
        };
        let mods = install_inside_mod("inside-arrow-panic", &part);
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                let _ = horizon_client::block_contact::HOOKS.set(BLOCK_CONTACT_HOOKS);
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
                );
                for x in [1, 2] {
                    game.world.set(x, 100, 0, stone).unwrap();
                }
                // The arrow joins the level's own list: the batch's lifted row.
                let shot = arrow_shot(11, (0.5, 100.0, 0.5), (2.5, 100.0, 0.5));
                let velocity = shot.velocity.x;
                let id = shot.id;
                game.creatures.projectiles.push(shot);
                let rows = game.creatures.projectiles.len();
                let mut projectiles = game.creatures.projectiles.lift();
                horizon_client::block_contact::in_level(Some("minecraft:overworld"), || {
                    projectiles.update(0, |p| game.sweep_projectile_inside(p));
                });
                game.creatures.projectiles.restore(
                    projectiles,
                    horizon_client::join_seq::RestoreOrder::SpawnsFirst,
                );
                assert_eq!(
                    part.contacts.lock().unwrap().len(),
                    2,
                    "both contacts reached the part; each panic was contained"
                );
                assert_eq!(
                    game.creatures.projectiles.len(),
                    rows,
                    "the lifted row returned, exactly once"
                );
                let arrow = game
                    .creatures
                    .projectiles
                    .iter()
                    .next()
                    .expect("the arrow is still in the list");
                assert_eq!(arrow.id, id, "the same arrow, not lost nor duplicated");
                assert!(
                    (arrow.velocity.x - velocity).abs() < 1e-9,
                    "the panicking callback's staged writes never applied"
                );
                assert!(horizon_client::block_contact::take_pending().is_empty());
            });
        });
    }

    /// An arrow for the sweep tests: an arrow state, its move recorded.
    fn arrow_shot(
        id: u32,
        from: (f64, f64, f64),
        to: (f64, f64, f64),
    ) -> horizon_client::projectiles::Projectile {
        let mut shot = horizon_client::projectiles::Projectile::new(
            horizon_client::projectiles::ProjectileKind::Arrow {
                base_damage: 1.0,
                critical: false,
                effect: None,
            },
            horizon_core::geometry::Vec3::new(to.0, to.1, to.2),
            None,
        );
        shot.id = id;
        shot.previous = horizon_core::geometry::Vec3::new(from.0, from.1, from.2);
        shot
    }
}
