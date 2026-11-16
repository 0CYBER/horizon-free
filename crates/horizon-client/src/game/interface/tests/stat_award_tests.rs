//! Statistic rewrites use the same counter/dirty/criteria path as ordinary awards.
use super::*;
use mod_api::{Flow, Hooks, PlayerStatAwardEvent, ServerHost};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

struct Rewrite(Arc<AtomicUsize>);
#[mod_api::__subscriptions]
impl Hooks for Rewrite {
    fn register(&self, registrar: &mut dyn mod_api::Registrar, _: &mod_api::RegistrationContext) {
        registrar.register_custom_stat("steps").unwrap();
    }
    fn player_stat_award(
        &self,
        _: &mut dyn ServerHost,
        event: &mut PlayerStatAwardEvent<'_>,
    ) -> Flow {
        match self.0.load(Ordering::Relaxed) {
            0 => event.value *= 2,
            1 => {
                *event.stat_type = "minecraft:custom".into();
                *event.stat = "rewrite:steps".into();
                event.value = 3;
            }
            2 => {
                *event.stat = "test:missing".into();
                event.value = 99;
            }
            3 => {
                *event.stat_type = "test:missing".into();
                event.value = 99;
            }
            4 => {
                *event.stat = "rewrite:steps".into();
                event.value = -1;
            }
            6 | 7 => {
                *event.stat = if self.0.load(Ordering::Relaxed) == 6 {
                    "stone"
                } else {
                    ":stone"
                }
                .into();
                event.value = 99;
            }
            _ => return Flow::Cancel,
        }
        Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn rewritten_awards_update_counts_criteria_and_dirty_values_atomically() {
    let mode = Arc::new(AtomicUsize::new(0));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "rewrite",
        Box::new(Rewrite(mode.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.build_registries();
            assert!(
                game.ui
                    .registries
                    .ids("minecraft:custom_stat")
                    .iter()
                    .any(|id| id == "rewrite:steps")
            );
            game.lan_acting_uuid = Some(12);
            game.lan_acting_name = Some("player".into());
            game.award_stat("mined", "minecraft:stone", 1);
            assert_eq!(game.stats.get("mined", "minecraft:stone"), 2);
            assert_eq!(
                game.stats.events.pop(),
                Some(("minecraft:mined".into(), "minecraft:stone".into(), Some(2)))
            );
            assert_eq!(
                game.stats.take_dirty(),
                [("minecraft:mined".into(), "minecraft:stone".into(), 2)]
            );
            mode.store(1, Ordering::Relaxed);
            game.award_stat("custom", "minecraft:jump", 1);
            assert_eq!(game.stats.get("custom", "rewrite:steps"), 3);
            assert_eq!(game.stats.get("custom", "minecraft:jump"), 0);
            assert_eq!(
                game.stats.events.pop(),
                Some(("minecraft:custom".into(), "rewrite:steps".into(), Some(3)))
            );
            assert_eq!(
                game.stats.take_dirty(),
                [("minecraft:custom".into(), "rewrite:steps".into(), 3)]
            );
            for invalid in 2..=4 {
                mode.store(invalid, Ordering::Relaxed);
                game.award_stat("custom", "minecraft:jump", 1);
                assert_eq!(
                    game.stats.get("custom", "minecraft:jump"),
                    (invalid - 1) as i32
                );
                assert_eq!(game.stats.get("custom", "rewrite:steps"), 3);
                assert_eq!(
                    game.stats.events.pop(),
                    Some(("minecraft:custom".into(), "minecraft:jump".into(), Some(1)))
                );
            }
            mode.store(5, Ordering::Relaxed);
            game.award_stat("custom", "minecraft:jump", 1);
            assert_eq!(game.stats.get("custom", "minecraft:jump"), 3);
        });
    });
}

#[test]
#[ignore = "daily"]
fn lan_guest_awards_and_leave_game_apply_the_edited_amount() {
    let mode = Arc::new(AtomicUsize::new(0));
    let mods: mod_api::Mods =
        vec![mod_api::ModEntry::new("rewrite", Box::new(Rewrite(mode)))].into();
    mod_platform::testing::with_mods(mods, || {
        crate::game::native_guest_turn_tests::with_guests(|game| {
            game.lan
                .as_mut()
                .unwrap()
                .play
                .insert(12, Default::default());
            game.as_guest(12, |game| {
                game.award_stat("mined", "minecraft:stone", 1);
                assert_eq!(game.stats.get("mined", "minecraft:stone"), 2);
                assert_eq!(
                    game.stats.take_dirty(),
                    [("minecraft:mined".into(), "minecraft:stone".into(), 2)]
                );
            })
            .unwrap();
            game.award_leave_game(12, 12, "guest12");
            game.award_leave_game(12, 12, "guest12");
            let play = game.lan.as_mut().unwrap().play.get_mut(&12).unwrap();
            assert_eq!(play.stats.get("custom", "minecraft:leave_game"), 2);
            assert!(play.stats.take_dirty().contains(&(
                "minecraft:custom".into(),
                "minecraft:leave_game".into(),
                2
            )));
        });
    });
}

#[test]
#[ignore = "daily"]
fn deferred_award_rewrites_only_after_outer_commit_and_abort_emits_nothing() {
    use crate::game::transactions::DeferredEffect;
    let mode = Arc::new(AtomicUsize::new(0));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "rewrite",
        Box::new(Rewrite(mode.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        crate::game::native_guest_turn_tests::with_guests(|game| {
            game.lan
                .as_mut()
                .unwrap()
                .play
                .insert(12, Default::default());
            let outer = game.transaction_open();
            let inner = game.transaction_open();
            game.defer_effect(DeferredEffect::CustomStat {
                player: 12,
                key: "jump",
                amount: 1,
            });
            game.transaction_close(&inner, true);
            assert_eq!(
                game.lan.as_ref().unwrap().play[&12]
                    .stats
                    .get("custom", "minecraft:jump"),
                0
            );
            // The listener's decision is taken at root commit, not at queue time.
            mode.store(5, Ordering::Relaxed);
            game.transaction_close(&outer, true);
            assert_eq!(
                game.lan.as_ref().unwrap().play[&12]
                    .stats
                    .get("custom", "minecraft:jump"),
                0
            );
            mode.store(0, Ordering::Relaxed);
            let outer = game.transaction_open();
            let inner = game.transaction_open();
            game.defer_effect(DeferredEffect::CustomStat {
                player: 12,
                key: "jump",
                amount: 1,
            });
            game.transaction_close(&inner, true);
            game.transaction_close(&outer, false);
            assert_eq!(
                game.lan.as_ref().unwrap().play[&12]
                    .stats
                    .get("custom", "minecraft:jump"),
                0
            );
            let outer = game.transaction_open();
            game.defer_effect(DeferredEffect::CustomStat {
                player: 12,
                key: "jump",
                amount: 1,
            });
            game.transaction_close(&outer, true);
            assert_eq!(
                game.lan.as_ref().unwrap().play[&12]
                    .stats
                    .get("custom", "minecraft:jump"),
                2
            );
        });
    });
}

// Same survival/tool/chunk setup as native_player_destroyed_block_tests.
fn prepare_breaker(game: &mut Game<'_>) {
    use horizon_client::{command::args::GameMode, items::ItemStack, motion::PlayerMotion};
    use horizon_core::{geometry::Vec3, position::ChunkPos};
    let _ = game.block_ticks.add_empty_chunk(ChunkPos::new(0, 0));
    game.world
        .chunks
        .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
    game.player = Some(PlayerMotion::new(game.motion, Vec3::new(4.5, 64.0, 4.5)).unwrap());
    game.set_mode(GameMode::Survival);
    game.inventory[game.selected] = Some(ItemStack::new("minecraft:diamond_pickaxe", 1));
}

fn break_stone(game: &mut Game<'_>) {
    let stone = game
        .pack
        .assets
        .registry
        .default_state("minecraft:stone")
        .unwrap();
    game.world.set(6, 64, 4, stone).unwrap();
    assert!(game.player_break_block([6, 64, 4]).unwrap());
    assert_eq!(
        game.res
            .facts()
            .get(game.world.block(6, 64, 4))
            .name
            .to_string(),
        "minecraft:air"
    );
    assert_eq!(game.stats.get("mined", "minecraft:stone"), 2);
}

#[test]
#[ignore = "daily"]
fn real_integrated_stone_break_updates_counter_score_and_dirty_award() {
    let mode = Arc::new(AtomicUsize::new(0));
    let mods = vec![mod_api::ModEntry::new(
        "rewrite",
        Box::new(Rewrite(mode.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.build_registries();
            game.set_allow_commands(true);
            prepare_breaker(game);
            game.lan_acting_uuid = Some(123);
            game.lan_acting_entity = Some(20);
            game.lan_acting_name = Some("Breaker".into());
            assert!(
                game.run_command("scoreboard objectives add mined minecraft.mined:minecraft.stone")
            );
            break_stone(game);
            game.tick_scoreboard();
            assert_eq!(
                game.score
                    .board
                    .score(crate::game::chat::PLAYER_NAME, "mined")
                    .unwrap()
                    .value,
                2
            );
            let dirty = game.stats.take_dirty();
            assert!(dirty.contains(&("minecraft:mined".into(), "minecraft:stone".into(), 2)));
            assert!(game.stats.take_dirty().is_empty());
            assert!(
                game.run_command("scoreboard objectives add steps minecraft.custom:rewrite.steps")
            );
            mode.store(1, Ordering::Relaxed);
            {
                let player = game.player.as_mut().unwrap();
                player.ground.on_ground = true;
                let mut world = horizon_client::motion::MotionWorld {
                    entities: &[],
                    player,
                    scene: &mut game.world,
                    assets: &game.pack.assets,
                    collision_binding: game.collision,
                    resources: game.motion,
                    health: &game.health_resources,
                    tag_items: None,
                    loop_context: &game.loop_context,
                    damage_hooks: &game.damage_hooks,
                    block_registries: Some(&game.pack.block_registries),
                };
                world.jump_from_ground().unwrap();
            }
            let events = std::mem::take(&mut game.player.as_mut().unwrap().events);
            game.record_combat(&events);
            game.tick_scoreboard();
            assert_eq!(game.stats.get("custom", "rewrite:steps"), 3);
            assert_eq!(game.stats.get("custom", "minecraft:jump"), 0);
            assert_eq!(
                game.score
                    .board
                    .score(crate::game::chat::PLAYER_NAME, "steps")
                    .unwrap()
                    .value,
                3
            );
            assert!(game.stats.take_dirty().contains(&(
                "minecraft:custom".into(),
                "rewrite:steps".into(),
                3
            )));
        });
    });
}

fn lan_real_break_and_jump(mods: mod_api::Mods, custom: &str, mode: Option<&AtomicUsize>) {
    mod_platform::testing::with_mods(mods, || {
        crate::game::native_guest_turn_tests::with_guests(|game| {
            game.build_registries();
            game.set_allow_commands(true);
            game.lan
                .as_mut()
                .unwrap()
                .play
                .insert(12, Default::default());
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| {
                    guest.name = "stat_award_fixture".into();
                })
                .unwrap();
            let (tx, rx) = std::sync::mpsc::channel();
            game.lan
                .as_mut()
                .unwrap()
                .hub
                .update_guest(12, |guest| {
                    guest.sender = horizon_client::lan::PlayerSender::new(tx, Arc::default());
                })
                .unwrap();
            assert!(
                game.run_command("scoreboard objectives add mined minecraft.mined:minecraft.stone")
            );
            assert!(game.run_command(&format!(
                "scoreboard objectives add steps minecraft.custom:{}",
                custom.replace(':', ".")
            )));
            game.as_guest(12, |game| {
                prepare_breaker(game);
                break_stone(game);
            })
            .unwrap();
            assert_eq!(
                game.score
                    .board
                    .score("stat_award_fixture", "mined")
                    .unwrap()
                    .value,
                2
            );
            let dirty = game
                .lan
                .as_mut()
                .unwrap()
                .play
                .get_mut(&12)
                .unwrap()
                .stats
                .take_dirty();
            assert!(dirty.contains(&("minecraft:mined".into(), "minecraft:stone".into(), 2)));
            assert!(
                game.lan
                    .as_mut()
                    .unwrap()
                    .play
                    .get_mut(&12)
                    .unwrap()
                    .stats
                    .take_dirty()
                    .is_empty()
            );
            game.lan.as_ref().unwrap().hub.send_stats(12, &dirty);
            assert_dirty_packet(
                game,
                &rx,
                "minecraft:mined",
                "minecraft:block",
                "minecraft:stone",
                2,
            );
            if let Some(mode) = mode {
                mode.store(1, Ordering::Relaxed);
            }
            game.guest_jump(12, [4.5, 64.0, 4.5], 0.0);
            game.tick_guests();
            let play = &mut game.lan.as_mut().unwrap().play.get_mut(&12).unwrap();
            assert_eq!(play.stats.get("custom", custom), 3);
            assert_eq!(play.stats.get("custom", "minecraft:jump"), 0);
            let dirty = play.stats.take_dirty();
            assert!(dirty.contains(&("minecraft:custom".into(), custom.into(), 3)));
            game.lan.as_ref().unwrap().hub.send_stats(12, &dirty);
            assert_dirty_packet(
                game,
                &rx,
                "minecraft:custom",
                "minecraft:custom_stat",
                custom,
                3,
            );
            assert_eq!(
                game.score
                    .board
                    .score("stat_award_fixture", "steps")
                    .unwrap()
                    .value,
                3
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn real_lan_break_and_jump_use_registered_rewrite_targets() {
    let mode = Arc::new(AtomicUsize::new(0));
    let mods = vec![mod_api::ModEntry::new(
        "rewrite",
        Box::new(Rewrite(mode.clone())),
    )]
    .into();
    lan_real_break_and_jump(mods, "rewrite:steps", Some(&mode));
}

#[test]
#[ignore = "daily"]
fn lan_noncanonical_rewrite_keeps_the_entire_original_award() {
    let mode = Arc::new(AtomicUsize::new(6));
    let mods = vec![mod_api::ModEntry::new(
        "rewrite",
        Box::new(Rewrite(mode.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        crate::game::native_guest_turn_tests::with_guests(|game| {
            game.lan
                .as_mut()
                .unwrap()
                .play
                .insert(12, Default::default());
            for case in [6, 7] {
                mode.store(case, Ordering::Relaxed);
                game.as_guest(12, |game| {
                    game.award_stat("mined", "minecraft:dirt", 4);
                    assert_eq!(
                        game.stats.get("mined", "minecraft:dirt"),
                        (case - 5) as i32 * 4
                    );
                    assert_eq!(game.stats.get("mined", "minecraft:stone"), 0);
                    assert_eq!(
                        game.stats.events.pop(),
                        Some(("minecraft:mined".into(), "minecraft:dirt".into(), Some(4)))
                    );
                    assert_eq!(
                        game.stats.take_dirty(),
                        [(
                            "minecraft:mined".into(),
                            "minecraft:dirt".into(),
                            (case - 5) as i32 * 4
                        )]
                    );
                })
                .unwrap();
            }
        });
    });
}

#[test]
#[ignore = "needs HORIZON_TEST_RESOURCES, HORIZON_WASM_FIXTURE and HORIZON_WASM_STD; build with horizon-mod-wasm/listener-fixture"]
fn wasm_real_break_and_jump_rewrite_awards() {
    horizon_guest_stack::run_on_guest_stack(|| {
        use horizon_mod_package::manifest::Package;
        use horizon_mod_wasm::{WasmPackage, load};
        let path = std::path::PathBuf::from(std::env::var_os("HORIZON_WASM_FIXTURE").unwrap());
        let manifest = Package::read(&path).unwrap().manifest;
        let info = mod_api::PackageInfo::new(
            manifest.id.clone(),
            manifest.version.clone(),
            mod_api::PackageKind::Code,
            mod_api::Side::Both,
            vec![manifest.id.clone()],
            vec![],
        );
        let loaded = load(
            &std::fs::read(std::env::var_os("HORIZON_WASM_STD").unwrap()).unwrap(),
            vec![WasmPackage {
                manifest,
                module: Package::read_wasm(&path).unwrap(),
            }],
        );
        assert!(loaded.refused.is_empty(), "{:?}", loaded.refused);
        assert_eq!(loaded.mods.len(), 1);
        loaded.initialize(&[info]).unwrap();
        lan_real_break_and_jump(loaded.mods.into(), "wasm_fixture:steps", None);
    });
}

fn assert_dirty_packet(
    game: &Game<'_>,
    rx: &std::sync::mpsc::Receiver<horizon_client::lan::Out>,
    ty: &str,
    registry: &str,
    key: &str,
    count: i32,
) {
    let hub = &game.lan.as_ref().unwrap().hub;
    let ty = hub
        .synced_id(
            &mod_api::Identifier::parse_namespaced("minecraft:stat_type").unwrap(),
            &mod_api::Identifier::parse_namespaced(ty).unwrap(),
        )
        .unwrap();
    let key = hub
        .synced_id(
            &mod_api::Identifier::parse_namespaced(registry).unwrap(),
            &mod_api::Identifier::parse_namespaced(key).unwrap(),
        )
        .unwrap();
    let mut found = Vec::new();
    for out in rx.try_iter() {
        let bodies = match out {
            horizon_client::lan::Out::Packet(body) => vec![body],
            horizon_client::lan::Out::Packets(bodies) => bodies,
            horizon_client::lan::Out::Disconnect(_) => panic!("unexpected disconnect"),
        };
        for body in bodies {
            let mut reader = horizon_protocol::buf::Reader::new(&body);
            if reader.varint().unwrap() != horizon_protocol::ids::play::clientbound::AWARD_STATS {
                continue;
            }
            for _ in 0..reader.varint().unwrap() {
                found.push((
                    reader.varint().unwrap(),
                    reader.varint().unwrap(),
                    reader.varint().unwrap(),
                ));
            }
            assert!(reader.rest().is_empty());
        }
    }
    assert_eq!(
        found
            .iter()
            .filter(|&&(t, k, _)| t == ty && k == key)
            .copied()
            .collect::<Vec<_>>(),
        [(ty, key, count)]
    );
}
