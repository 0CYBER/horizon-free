use super::super::native_client_session::tests::logged_client;
use super::super::native_entity_refs::tests::with_client_game;
use super::*;
use mod_api::{Flow, HostSide, SoundPlay, SoundSource};
use std::sync::Mutex;

#[derive(Default)]
struct Log {
    requests: Vec<&'static str>,
    played: Vec<SoundPlay>,
}
struct Tuner(Arc<Mutex<Log>>);
#[mod_api::__subscriptions]
impl mod_api::Hooks for Tuner {
    fn play_level_sound_at_position(
        &self,
        host: HostSide<'_>,
        e: &mut mod_api::PlayLevelSoundAtPositionEvent,
    ) -> Flow {
        assert!(matches!(host, HostSide::Client(_)));
        self.0.lock().unwrap().requests.push("position");
        assert_eq!(e.position, [1.0, 64.0, -2.0]);
        assert_eq!((e.original_volume, e.original_pitch), (1.0, 0.75));
        if e.sound.to_string() == "minecraft:entity.pig.ambient" {
            return Flow::Cancel;
        }
        *e.sound = mod_api::Identifier::parse("minecraft:entity.cow.ambient").unwrap();
        *e.source = SoundSource::Ambient;
        (e.volume, e.pitch) = (0.25, 1.5);
        Flow::Continue
    }
    fn play_level_sound_at_entity(
        &self,
        host: HostSide<'_>,
        e: &mut mod_api::PlayLevelSoundAtEntityEvent,
    ) -> Flow {
        assert!(matches!(host, HostSide::Client(_)));
        assert_eq!(e.entity.id(), 7);
        assert_eq!((e.original_volume, e.original_pitch), (2.0, 0.5));
        self.0.lock().unwrap().requests.push("entity");
        *e.source = SoundSource::Records;
        (e.volume, e.pitch) = (0.5, 2.0);
        Flow::Continue
    }
    fn play_sound(&self, _: &dyn mod_api::LoopContext, e: &mut mod_api::PlaySoundEvent<'_>) {
        // Active sounds also refresh their playback parameters on ticks.
        if e.starting {
            self.0.lock().unwrap().played.push(e.original.clone());
        }
    }
}

#[derive(Debug, PartialEq)]
struct Request {
    sound: String,
    source: SoundSource,
    emitter: Option<u32>,
    position: Option<[f64; 3]>,
}
struct LocalTuner(Arc<Mutex<Vec<Request>>>);
#[mod_api::__subscriptions]
impl mod_api::Hooks for LocalTuner {
    fn play_level_sound_at_position(
        &self,
        host: HostSide<'_>,
        e: &mut mod_api::PlayLevelSoundAtPositionEvent,
    ) -> Flow {
        assert!(matches!(host, HostSide::Client(_)));
        self.0.lock().unwrap().push(Request {
            sound: e.sound.to_string(),
            source: *e.source,
            emitter: None,
            position: Some(e.position),
        });
        Flow::Cancel
    }
    fn play_level_sound_at_entity(
        &self,
        host: HostSide<'_>,
        e: &mut mod_api::PlayLevelSoundAtEntityEvent,
    ) -> Flow {
        assert!(matches!(host, HostSide::Client(_)));
        let sound = e.sound.to_string();
        self.0.lock().unwrap().push(Request {
            sound: sound.clone(),
            source: *e.source,
            emitter: Some(e.entity.id()),
            position: None,
        });
        if sound == "minecraft:block.honey_block.slide" {
            Flow::Continue
        } else {
            Flow::Cancel
        }
    }
}

#[test]
#[ignore = "daily"]
fn local_level_requests_can_mute_sound_without_undoing_prediction() {
    use horizon_core::position::BlockPos;
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new(
        "local_tuner",
        Box::new(LocalTuner(requests.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_client_game(|game| {
            let _server = logged_client(game);
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(1.0, 64.0, 1.0)).unwrap());
            game.streaming
                .insert_remote(&mut game.world, Chunk::empty(0, 0));
            let old = game
                .pack
                .assets
                .registry
                .default_state("minecraft:oak_door")
                .unwrap();
            let new = game
                .pack
                .assets
                .registry
                .default_state("minecraft:birch_door")
                .unwrap();
            game.world.set(1, 64, 1, old).unwrap();
            let at = BlockPos::new(1, 64, 1);
            game.predict_use(
                &(
                    vec![(at, new)],
                    vec![(at, "block.wooden_door.open".into(), 1.0, 0.9)],
                ),
                1,
            );
            assert_eq!(
                game.world.block(1, 64, 1),
                new,
                "sound cancellation does not cancel the use"
            );
            game.player_menu_sound(horizon_client::bundle::INSERT_FAIL);
            let stack = ItemStack {
                item: "minecraft:apple".into(),
                count: 1,
                damage: 0,
                extra: None,
            };
            let consumable = game
                .items()
                .get(&stack.item)
                .unwrap()
                .consumable
                .clone()
                .unwrap();
            game.emit_consume(&consumable, &stack, 16);
            game.play_player_bound_sound("minecraft:item.goat_horn.sound.0", 2, 8.0, 1.0, true);
            assert!(
                game.audio.heard.is_empty(),
                "all four canceled requests stop before Audio"
            );
            let expected = [
                Request {
                    sound: "minecraft:block.wooden_door.open".into(),
                    source: SoundSource::Blocks,
                    emitter: None,
                    position: Some([1.5, 64.5, 1.5]),
                },
                Request {
                    sound: horizon_client::bundle::INSERT_FAIL.into(),
                    source: SoundSource::Players,
                    emitter: Some(7),
                    position: None,
                },
                Request {
                    sound: "minecraft:entity.generic.eat".into(),
                    source: SoundSource::Players,
                    emitter: Some(7),
                    position: None,
                },
                Request {
                    sound: "minecraft:item.goat_horn.sound.0".into(),
                    source: SoundSource::Records,
                    emitter: Some(7),
                    position: None,
                },
            ];
            assert_eq!(*requests.lock().unwrap(), expected);
            game.audio.engine.loaded = true;
            game.play_client_player_sound("minecraft:block.honey_block.slide", 1.0, 1.0);
            assert_eq!(
                game.audio.heard.get("minecraft:block.honey_block.slide"),
                Some(&1)
            );
            assert!(
                game.audio.engine.bound_entities().is_empty(),
                "LocalPlayer.playSound stays positional"
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn game_events_ask_client_level_before_playback() {
    let requests = Arc::new(Mutex::new(Vec::new()));
    let mods = vec![mod_api::ModEntry::new(
        "game_event_tuner",
        Box::new(LocalTuner(requests.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_client_game(|game| {
            let _server = logged_client(game);
            let mut player = PlayerMotion::new(game.motion, Vec3::new(1.0, 64.0, 1.0)).unwrap();
            player.dimensions.eye_height = 1.25;
            game.player = Some(player);
            for event in [6, 9, 10] {
                game.handle_packet(C::GameEvent(play::GameEvent { event, param: 1.0 }))
                    .unwrap();
            }
            assert_eq!(
                *requests.lock().unwrap(),
                [
                    Request {
                        sound: "minecraft:entity.arrow.hit_player".into(),
                        source: SoundSource::Players,
                        emitter: None,
                        position: Some([1.0, 65.25, 1.0])
                    },
                    Request {
                        sound: "minecraft:entity.puffer_fish.sting".into(),
                        source: SoundSource::Neutral,
                        emitter: None,
                        position: Some([1.0, 64.0, 1.0])
                    },
                    Request {
                        sound: "minecraft:entity.elder_guardian.curse".into(),
                        source: SoundSource::Hostile,
                        emitter: None,
                        position: Some([1.0, 64.0, 1.0])
                    },
                ]
            );
            assert!(
                game.audio.heard.is_empty(),
                "all three canceled requests stop before Audio"
            );
            game.handle_packet(C::GameEvent(play::GameEvent {
                event: 10,
                param: 0.0,
            }))
            .unwrap();
            assert_eq!(
                requests.lock().unwrap().len(),
                3,
                "a silent curse asks no sound hook"
            );
            let level = horizon_client::particles::ParticleLevel {
                world: &game.world,
                facts: game.res.facts(),
                collision: game.collision,
                player: None,
                cached_chunk: std::cell::Cell::new(None),
            };
            game.particles.tick(&level);
            assert_eq!(
                game.particles.elder_guardians().len(),
                2,
                "neither silent nor canceled sound suppresses the apparition"
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn packets_ask_client_level_before_playback_not_ui() {
    use horizon_world::native_load_context::{DataLoadOrigin, LoadContext, LoadInputs, LoopSide};
    let log = Arc::new(Mutex::new(Log::default()));
    let mods = vec![mod_api::ModEntry::new(
        "tuner",
        Box::new(Tuner(log.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_client_game(|game| {
            let _server = logged_client(game);
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(3.0, 64.0, 4.0)).unwrap());
            let inputs = Arc::new(LoadInputs::new(
                Default::default(),
                game.options.resources.clone(),
            ));
            let context = LoadContext::new(DataLoadOrigin::ClientPreload, inputs, []);
            game.audio
                .set_loop_context(context.loop_context(LoopSide::Client, []));
            // Exercise the real engine and mixer bookkeeping without opening a device.
            game.audio.engine.loaded = true;
            game.audio.engine.registry.add_file(
                "minecraft",
                br#"{
                "entity.cow.ambient": {"sounds": ["cow"]},
                "item.goat_horn.sound.0": {"sounds": ["horn"]}
            }"#,
                &|_, _| true,
            );
            let deliver = |game: &mut Game<'_>, packet: C| {
                let body = packet.encode();
                let mut reader = horizon_protocol::buf::Reader::new(&body);
                let id = reader.varint().unwrap();
                game.handle_packet(C::decode(id, reader.rest()).unwrap())
                    .unwrap();
            };
            let position = |name: &str| {
                C::Sound(play::Sound {
                    sound: (0, Some((name.into(), None))),
                    source: SoundSource::Blocks as i32,
                    x: 8,
                    y: 512,
                    z: -16,
                    volume: 1.0,
                    pitch: 0.75,
                    seed: 91,
                })
            };
            deliver(game, position("minecraft:block.chest.locked"));
            {
                let log = log.lock().unwrap();
                assert_eq!(log.requests, ["position"]);
                let heard = &log.played[0];
                assert_eq!(heard.identifier, "minecraft:entity.cow.ambient");
                assert_eq!(
                    (heard.source, heard.volume, heard.pitch),
                    (SoundSource::Ambient, 0.25, 1.5)
                );
                assert_eq!(heard.position, [1.0, 64.0, -2.0]);
            }
            deliver(game, position("minecraft:entity.pig.ambient"));
            assert_eq!(
                log.lock().unwrap().played.len(),
                1,
                "cancel precedes the engine hook"
            );
            let entity = |id| {
                C::SoundEntity(play::SoundEntity {
                    sound: (0, Some(("minecraft:item.goat_horn.sound.0".into(), None))),
                    source: SoundSource::Players as i32,
                    id,
                    volume: 2.0,
                    pitch: 0.5,
                    seed: -17,
                })
            };
            deliver(game, entity(7));
            {
                let log = log.lock().unwrap();
                assert_eq!(log.requests, ["position", "position", "entity"]);
                let heard = &log.played[1];
                assert_eq!(
                    (heard.source, heard.volume, heard.pitch),
                    (SoundSource::Records, 0.5, 2.0)
                );
                assert_eq!(heard.position, [3.0, 64.0, 4.0]);
            }
            let bound = game.audio.engine.bound_entities();
            assert_eq!(bound.len(), 1);
            assert_eq!(bound[0].id, 7);
            game.player.as_mut().unwrap().position.x = 9.0;
            game.tick_sound_engine();
            assert_eq!(bound[0].state().position, [9.0, 64.0, 4.0]);
            deliver(game, entity(999));
            {
                let log = log.lock().unwrap();
                assert_eq!(
                    log.requests,
                    ["position", "position", "entity"],
                    "absent emitter asks no level hook"
                );
                assert_eq!(log.played.len(), 2, "absent emitter starts no playback");
            }
            game.audio.play_button_click();
            let log = log.lock().unwrap();
            assert_eq!(log.requests.len(), 3, "UI playback adds no level request");
            assert_eq!(
                log.played.last().unwrap().identifier,
                "minecraft:ui.button.click"
            );
            assert!(
                game.bound_sounds.is_empty(),
                "client playback is never broadcast"
            );
        });
    });
}
