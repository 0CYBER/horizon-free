//! Viewed levels, the client's worlds (#3716, 9c): a world for each level a server opens on
//! `horizon:levels`, as Immersive Portals' `ClientWorldLoader` keeps a `ClientLevel` per
//! dimension. The server wraps each packet of such a level with it (`PacketRedirection`); here
//! a `run`'s packets change that level's world and nothing of the player's own: its `World`,
//! its renderer and its sounds stay as they are. A client mod reads a viewed level's blocks
//! through `host.level` (gap 10, `modding/native_viewed_levels.rs`) and never writes it.
//! A viewed level's sound plays only where a mod's `viewed_level_sound` puts it (`sounds`).
//!
//! The client views at most `viewed_levels_net::MAX_LEVELS` levels besides its own, the number
//! its capability answer gives. A travel into one of them swaps it with the player's world
//! (`switch`); a Login, any other Respawn or a new configuration phase drops every viewed world,
//! and the server opens again what it still views. Nothing of a viewed level is drawn yet:
//! drawing is gap 13's world view. Its entities live in its world (`entities`), apart from the
//! player's own.
mod entities;
mod routing;
mod sounds;
mod switch;
mod worlds;
pub(in crate::game) use worlds::ViewedWorlds;

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::routing::RouteError;
    use super::sounds::{ViewedPlay, ViewedSound};
    use crate::game::{Game, game_tests::tests::server_game};
    use horizon_protocol::{
        buf::Reader,
        chunk::{ChunkData, LightData, Paletted, Section, Strategy},
        packets::play::{self, Clientbound as C},
        viewed_levels,
    };
    use horizon_world::registry::BlockStateId;
    use mod_api::SoundSource;
    use std::sync::{Arc, Mutex};

    pub(super) const STONE: BlockStateId = BlockStateId(1);
    pub(super) const AIR: BlockStateId = BlockStateId(0);

    pub(in crate::game) fn spawn(dimension: &str) -> play::SpawnInfo {
        play::SpawnInfo {
            dimension_type: 0,
            dimension: dimension.into(),
            seed: 1,
            game_type: 0,
            previous_game_type: None,
            debug: false,
            flat: false,
            last_death: None,
            portal_cooldown: 0,
            sea_level: 63,
        }
    }

    /// A client of a server whose Login names four levels, the player in the Overworld.
    pub(in crate::game) fn client(
        resources: &mut Option<std::sync::Arc<crate::game::Resources>>,
    ) -> Game<'_> {
        let mut game = server_game(resources);
        game.options.execution_role = crate::game::execution_role::ExecutionRole::Client;
        let (pipe, _server) = horizon_protocol::connection::Pipe::pair();
        game.connect_local(pipe, "Steve").unwrap();
        game.handle_packet(C::Login(play::Login {
            player_id: 7,
            hardcore: false,
            levels: [
                "minecraft:overworld",
                "minecraft:the_nether",
                "minecraft:the_end",
                "test:void",
            ]
            .map(String::from)
            .to_vec(),
            max_players: 20,
            chunk_radius: 8,
            simulation_distance: 8,
            reduced_debug_info: false,
            show_death_screen: true,
            limited_crafting: false,
            spawn: spawn("minecraft:overworld"),
            online_mode: false,
            enforces_secure_chat: false,
        }))
        .unwrap();
        // The dimension types the server synced: 0 the Overworld's (what `spawn` names), 1 the
        // Nether's, of another shape.
        let mut registries = horizon_protocol::registry::Dynamic::default();
        registries.registries.insert(
            "minecraft:dimension_type".into(),
            ["minecraft:overworld", "minecraft:the_nether"]
                .map(|name| (name.to_owned(), None))
                .to_vec(),
        );
        game.remote.as_mut().unwrap().registries = Arc::new(registries);
        EPOCHS.with(|epochs| *epochs.borrow_mut() = Default::default());
        game
    }

    thread_local! {
        /// This test's connection as a server keeps it: the next epoch, and the one each slot was
        /// last bound with, so that `run` and `close` follow `open` and `swap`.
        static EPOCHS: std::cell::RefCell<(i32, std::collections::HashMap<i32, i32>)> =
            Default::default();
    }

    /// Binds `slot` with the next epoch, as a server's `open` or `swap` does.
    pub(in crate::game) fn bind(slot: i32) -> viewed_levels::Level {
        EPOCHS.with(|epochs| {
            let (next, slots) = &mut *epochs.borrow_mut();
            let epoch = *next;
            *next += 1;
            slots.insert(slot, epoch);
            viewed_levels::Level::Bound { slot, epoch }
        })
    }

    /// `slot` with the epoch it was last bound with.
    pub(in crate::game) fn bound_slot(slot: i32) -> viewed_levels::Level {
        let epoch = EPOCHS.with(|epochs| epochs.borrow().1.get(&slot).copied().unwrap_or(0));
        viewed_levels::Level::Bound { slot, epoch }
    }

    /// A column of stone at `x, z`, unlit.
    pub(in crate::game) fn column(x: i32, z: i32) -> C {
        layered(x, z, 24)
    }

    /// A column at `x, z` whose lowest `stone` of its 24 sections are stone and the rest air,
    /// unlit.
    pub(in crate::game) fn layered(x: i32, z: i32, stone: usize) -> C {
        let section = |state: BlockStateId| Section {
            non_empty: if state == AIR { 0 } else { 4096 },

            fluids: 0,
            states: Paletted::from_values(&[state.0 as i32; 4096], Strategy::Blocks, 16),
            biomes: Paletted::from_values(&[1; 64], Strategy::Biomes, 2),
        };
        let sections: Vec<_> = (0..24)
            .map(|i| section(if i < stone { STONE } else { AIR }))
            .collect();
        C::LevelChunkWithLight(play::LevelChunkWithLight {
            x,
            z,
            chunk: ChunkData {
                heightmaps: Vec::new(),
                buffer: ChunkData::from_sections(&sections),
                block_entities: Vec::new(),
            },
            light: LightData {
                sky_mask: Vec::new(),
                block_mask: Vec::new(),
                empty_sky_mask: Vec::new(),
                empty_block_mask: Vec::new(),
                sky: Vec::new(),
                block: Vec::new(),
            },
        })
    }

    /// The custom payload the client's net thread hands the game for a whole packet body.
    pub(in crate::game) fn payload(body: &[u8]) -> C {
        let mut r = Reader::new(body);
        let id = r.varint().unwrap();
        C::decode(id, r.rest()).unwrap()
    }

    /// An `open` of the level `dimension` in `slot`, with the next epoch and a cap of 2048
    /// columns.
    pub(in crate::game) fn open(slot: i32, dimension: &str) -> C {
        payload(&viewed_levels::open(bind(slot), 2048, &spawn(dimension)))
    }

    /// A `run` of `slot` at the epoch it was last bound with.
    pub(in crate::game) fn run(slot: i32, packets: &[C]) -> C {
        run_at(bound_slot(slot), packets)
    }

    /// A `run` of `level`, its slot and epoch as given.
    pub(in crate::game) fn run_at(level: viewed_levels::Level, packets: &[C]) -> C {
        let mut run = viewed_levels::Run::new(level, &packets[0].encode()).unwrap();
        for packet in &packets[1..] {
            run.push(&packet.encode()).unwrap();
        }
        payload(&run.finish())
    }

    /// A `close` of `slot` at the epoch it was last bound with.
    pub(in crate::game) fn close(slot: i32) -> C {
        payload(&viewed_levels::close(bound_slot(slot)))
    }

    /// The server's `swap` after a travel's Respawn: `slot` with the next epoch.
    pub(in crate::game) fn swap(slot: i32) -> C {
        payload(&viewed_levels::swap(bind(slot)))
    }

    pub(in crate::game) fn data(packet: &C) -> &[u8] {
        let C::CustomPayload(p) = packet else {
            panic!("not a payload")
        };
        &p.data
    }

    pub(super) fn viewed<'a>(
        game: &'a Game,
        level: i32,
    ) -> Option<&'a horizon_client::world::World> {
        Some(&game.remote.as_ref()?.viewed.get(level)?.world)
    }

    /// A wrapped `BlockUpdate` changes the viewed level's block, and the player's level keeps
    /// its own at the same position.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_wrapped_block_update_changes_the_viewed_world_not_the_players() {
        let mut resources = None;
        let mut game = client(&mut resources);
        game.handle_packet(column(0, 0)).unwrap();
        game.handle_packet(open(1, "minecraft:the_nether")).unwrap();
        game.handle_packet(run(1, &[column(0, 0)])).unwrap();
        let block = C::BlockUpdate(play::BlockUpdate {
            pos: [3, 70, 5],
            state: AIR.0 as i32,
        });
        game.handle_packet(run(1, &[block])).unwrap();
        assert_eq!(viewed(&game, 1).unwrap().block(3, 70, 5), AIR);
        assert_eq!(viewed(&game, 1).unwrap().block(3, 71, 5), STONE);
        assert_eq!(game.world.block(3, 70, 5), STONE);
    }

    /// An `open` makes a world for the level, a `run` fills it, and a `close` drops it and
    /// releases the slot: a late `run` of that binding is dropped, and makes no world.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn open_makes_a_world_and_close_drops_it() {
        let mut resources = None;
        let mut game = client(&mut resources);
        game.handle_packet(open(0, "minecraft:the_end")).unwrap();
        let world = game.remote.as_ref().unwrap().viewed.get(0).unwrap();
        assert_eq!(world.level, "minecraft:the_end");
        assert_eq!(world.world.chunks.len(), 0);
        game.handle_packet(run(0, &[column(4, -2), column(5, -2)]))
            .unwrap();
        assert_eq!(viewed(&game, 0).unwrap().chunks.len(), 2);
        let forget = C::ForgetLevelChunk(play::ForgetLevelChunk { pos: [4, -2] });
        game.handle_packet(run(0, &[forget])).unwrap();
        assert_eq!(viewed(&game, 0).unwrap().chunks.len(), 1);
        game.handle_packet(close(0)).unwrap();
        assert!(viewed(&game, 0).is_none());
        assert_eq!(
            game.route_viewed_levels(data(&run(0, &[column(4, -2)]))),
            Ok(())
        );
        assert!(viewed(&game, 0).is_none());
    }

    /// The client views two levels besides its own, in slots 0 and 1: a slot past them closes the
    /// connection, and an `open` of a bound slot starts it afresh.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn an_open_past_the_clients_cap_closes_the_connection() {
        let mut resources = None;
        let mut game = client(&mut resources);
        game.handle_packet(open(0, "minecraft:the_nether")).unwrap();
        game.handle_packet(open(1, "minecraft:the_end")).unwrap();
        assert_eq!(
            game.route_viewed_levels(data(&open(2, "test:void"))),
            Err(RouteError::OverCap { level: 2, cap: 2 })
        );
        game.handle_packet(run(1, &[column(0, 0)])).unwrap();
        game.handle_packet(open(1, "minecraft:the_end")).unwrap();
        assert_eq!(viewed(&game, 1).unwrap().chunks.len(), 0, "afresh");
        assert!(game.remote.is_some());
        game.handle_packet(open(2, "test:void")).unwrap();
        assert!(game.remote.is_none());
    }

    /// Each `open` refusal has a fixed reason that shows none of the server's text: a dimension
    /// that is no level id (the codec's), a dimension type the server did not sync (no Overworld
    /// fallback), a level viewed already in another slot, the player's own level, a cap past the
    /// client's ceiling, an epoch not newer than the last and a negative slot (the codec's).
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn each_open_refusal_has_a_fixed_reason() {
        use viewed_levels::Level;
        let mut resources = None;
        let mut game = client(&mut resources);
        let refused = game
            .route_viewed_levels(data(&open(1, "Evil: Click Here!")))
            .unwrap_err();
        assert_eq!(
            refused,
            RouteError::Codec(viewed_levels::ViewedLevelsError::BadOpen)
        );
        assert_eq!(
            refused.to_string(),
            "horizon:levels: an open whose level does not read"
        );
        assert!(!refused.to_string().contains("Evil"));
        assert!(viewed(&game, 1).is_none());
        let unsynced = play::SpawnInfo {
            dimension_type: 7,
            ..spawn("evil:elsewhere")
        };
        let refused = game
            .route_viewed_levels(data(&payload(&viewed_levels::open(
                bind(1),
                2048,
                &unsynced,
            ))))
            .unwrap_err();
        assert_eq!(refused, RouteError::UnsyncedType);
        assert!(!refused.to_string().contains("evil"));
        game.handle_packet(open(0, "test:void")).unwrap();
        assert_eq!(
            game.route_viewed_levels(data(&open(1, "test:void"))),
            Err(RouteError::ViewedTwice(1))
        );
        assert_eq!(
            game.route_viewed_levels(data(&open(1, "minecraft:overworld"))),
            Err(RouteError::OwnLevel(1))
        );
        let past = viewed_levels::open(bind(1), 4490, &spawn("minecraft:the_end"));
        assert_eq!(
            game.route_viewed_levels(data(&payload(&past))),
            Err(RouteError::CapPastCeiling(4490))
        );
        let old = Level::Bound { slot: 1, epoch: 0 };
        let old = viewed_levels::open(old, 2048, &spawn("minecraft:the_end"));
        assert_eq!(
            game.route_viewed_levels(data(&payload(&old))),
            Err(RouteError::OldEpoch(0))
        );
        let minus_one = Level::Bound { slot: -1, epoch: 9 };
        let negative = viewed_levels::open(minus_one, 2048, &spawn("minecraft:the_end"));
        assert_eq!(
            game.route_viewed_levels(data(&payload(&negative))),
            Err(RouteError::Codec(
                viewed_levels::ViewedLevelsError::Negative
            ))
        );
    }

    /// A level the Login did not name (a runtime level opened since) is viewed in the slot its
    /// `open` binds: its `run`s fill it.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_level_not_named_at_login_is_viewed_in_its_slot() {
        let mut resources = None;
        let mut game = client(&mut resources);
        game.handle_packet(open(1, "test:instance/1")).unwrap();
        game.handle_packet(run(1, &[column(2, 3)])).unwrap();
        let world = game.remote.as_ref().unwrap().viewed.get(1).unwrap();
        assert_eq!(world.level, "test:instance/1");
        assert_eq!(viewed(&game, 1).unwrap().chunks.len(), 1);
    }

    /// A slot closed and bound again to another level: a late chunk or entity of the old binding
    /// is dropped by its epoch, never reaching the new world, and the new binding's packets go to
    /// it. A `run` at an epoch no `open` gave closes the connection.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_late_packet_of_a_slot_bound_again_is_dropped() {
        let mut resources = None;
        let mut game = client(&mut resources);
        game.handle_packet(open(0, "minecraft:the_nether")).unwrap();
        let old = bound_slot(0);
        game.handle_packet(close(0)).unwrap();
        game.handle_packet(open(0, "minecraft:the_end")).unwrap();
        let pig = C::AddEntity(play::AddEntity {
            id: 40,
            uuid: 40,
            kind: 0,
            position: [0.5, 70.0, 0.5],
            velocity: [0.0; 3],
            pitch: 0,
            yaw: 0,
            head_yaw: 0,
            data: 0,
        });
        game.handle_packet(run_at(old, &[column(9, 9), pig]))
            .unwrap();
        let end = game.remote.as_ref().unwrap().viewed.get(0).unwrap();
        assert_eq!(end.level, "minecraft:the_end");
        assert!(end.world.chunks.is_empty(), "the Nether's late column");
        assert!(end.entities.get(40).is_none(), "the Nether's late entity");
        game.handle_packet(run(0, &[column(1, 1)])).unwrap();
        assert!(viewed(&game, 0).unwrap().chunks.contains_key(&(1, 1)));
        let ahead = viewed_levels::Level::Bound { slot: 0, epoch: 99 };
        assert_eq!(
            game.route_viewed_levels(data(&run_at(ahead, &[column(2, 2)]))),
            Err(RouteError::UnknownEpoch {
                level: 0,
                epoch: 99
            })
        );
    }

    /// An `open`'s column cap is the world's: a new column past it closes the connection.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn an_opens_column_cap_bounds_its_world() {
        let mut resources = None;
        let mut game = client(&mut resources);
        let small = viewed_levels::open(bind(0), 2, &spawn("minecraft:the_end"));
        game.handle_packet(payload(&small)).unwrap();
        game.handle_packet(run(0, &[column(0, 0), column(1, 0)]))
            .unwrap();
        assert_eq!(
            game.route_viewed_levels(data(&run(0, &[column(2, 0)]))),
            Err(RouteError::TooManyColumns(2))
        );
    }

    /// A viewed level's chunks, time and weather leave the player's level as it was.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn the_players_world_stays_as_it_was() {
        let mut resources = None;
        let mut game = client(&mut resources);
        game.handle_packet(column(0, 0)).unwrap();
        let (game_time, day_time) = (game.world.game_time, game.world.day_time);
        game.handle_packet(open(1, "minecraft:the_nether")).unwrap();
        let time = C::SetTime(play::SetTime {
            game_time: 24_000,
            clocks: Vec::new(),
        });
        let rain = C::GameEvent(play::GameEvent {
            event: 1,
            param: 0.0,
        });
        game.handle_packet(run(1, &[column(1, 0), time, rain]))
            .unwrap();
        let nether = viewed(&game, 1).unwrap();
        assert_eq!(nether.game_time, 24_000);
        assert!(nether.weather.raining);
        assert!(nether.chunks.contains_key(&(1, 0)));
        assert_eq!(game.world.chunks.len(), 1);
        assert!(!game.world.chunks.contains_key(&(1, 0)));
        assert_eq!(game.world.game_time, game_time);
        assert_eq!(game.world.day_time, day_time);
        assert!(!game.world.weather.raining);
        assert_eq!(game.remote.as_ref().unwrap().level, "minecraft:overworld");
    }

    /// A wrapped `AddEntity` and its move put the entity in the viewed world, where it moves; the
    /// player's own world holds nothing of it. A wrapped `RemoveEntities` takes it out.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_wrapped_entity_lives_in_the_viewed_world_not_the_players() {
        let mut resources = None;
        let mut game = client(&mut resources);
        game.handle_packet(open(1, "minecraft:the_nether")).unwrap();
        let add = C::AddEntity(play::AddEntity {
            id: 50,
            uuid: 50,
            kind: 0,
            position: [8.5, 70.0, 8.5],
            velocity: [0.0; 3],
            pitch: 0,
            yaw: 64,
            head_yaw: 0,
            data: 0,
        });
        let step = C::MoveEntityPos(play::MoveEntityPos(play::MoveEntity {
            id: 50,
            delta: Some(play::VecDelta::Linear([4096, 0, -2048])),
            rotation: None,
            on_ground: true,
        }));
        game.handle_packet(run(1, &[add, step])).unwrap();
        let nether = &game
            .remote
            .as_ref()
            .unwrap()
            .viewed
            .get(1)
            .unwrap()
            .entities;
        let pig = nether.get(50).expect("in the viewed world");
        assert_eq!((pig.uuid, pig.kind, pig.data), (50, 0, 0));
        assert_eq!(pig.position, [9.5, 70.0, 8.0]);
        assert_eq!(pig.rotation.0, 90.0);
        assert!(pig.on_ground);
        let r = game.remote.as_ref().unwrap();
        assert!(r.tracked_entity(50).is_none(), "not in the player's world");
        assert!(game.mobs.iter().all(|m| m.id != 50));
        let gone = C::RemoveEntities(play::RemoveEntities { ids: vec![50] });
        game.handle_packet(run(1, &[gone])).unwrap();
        let nether = &game
            .remote
            .as_ref()
            .unwrap()
            .viewed
            .get(1)
            .unwrap()
            .entities;
        assert!(nether.get(50).is_none());
    }

    /// A death's Respawn drops the viewed worlds: the server opens again what it still views.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_death_respawn_drops_the_viewed_worlds() {
        let mut resources = None;
        let mut game = client(&mut resources);
        game.handle_packet(open(1, "minecraft:the_nether")).unwrap();
        game.handle_packet(run(1, &[column(0, 0)])).unwrap();
        game.handle_packet(C::Respawn(play::Respawn {
            spawn: spawn("minecraft:overworld"),
            keep: 0,
        }))
        .unwrap();
        assert_eq!(game.remote.as_ref().unwrap().viewed.len(), 0);
    }

    /// A sound of `name` at fixed-point `pos` (eighths of a block), in the blocks' source.
    fn sound(name: &str, pos: [i32; 3], volume: f32, pitch: f32, seed: i64) -> C {
        C::Sound(play::Sound {
            sound: (-1, Some((name.into(), None))),
            source: SoundSource::Blocks.index() as i32,
            x: pos[0],
            y: pos[1],
            z: pos[2],
            volume,
            pitch,
            seed,
        })
    }

    /// What `viewed_level_sound` was asked: the level's name, the sound, its source, position,
    /// volume, pitch and seed.
    type Asked = (
        Option<&'static str>,
        String,
        SoundSource,
        [f64; 3],
        f32,
        f32,
        i64,
    );

    /// Plays the anvil at (1.5, 70, -2.5) with volume 0.25 and pitch 1.5, leaves the bell
    /// unplayed and puts the harp at a position that is not finite; logs what it was asked.
    struct Portal(Arc<Mutex<Vec<Asked>>>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Portal {
        fn viewed_level_sound(
            &self,
            _: &mut dyn mod_api::ClientHost,
            e: &mut mod_api::ViewedLevelSoundEvent<'_>,
        ) {
            let sound = e.sound.to_string();
            self.0.lock().unwrap().push((
                mod_api::__private::dimension_id(e.level.dimension()),
                sound.clone(),
                e.source,
                e.position,
                e.volume,
                e.pitch,
                e.seed,
            ));
            match sound.as_str() {
                "minecraft:block.anvil.land" => {
                    *e.play_at = Some([1.5, 70.0, -2.5]);
                    (e.volume, e.pitch) = (0.25, 1.5);
                }
                "minecraft:block.note_block.harp" => *e.play_at = Some([f64::NAN, 70.0, 0.0]),
                _ => {}
            }
        }
    }

    /// A viewed level's sound asks `viewed_level_sound` with the viewed level's handle, the
    /// sound's position there and the server's seed, and plays where the listener put it, with
    /// the listener's volume and pitch and the same seed. Left None, or put at a position that
    /// is not finite, it plays nothing. One bound to an entity of the viewed level is asked at that
    /// entity. A sound of the player's own level does not ask it.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_viewed_sound_plays_where_a_listener_puts_it() {
        let asked = Arc::default();
        let mods = vec![mod_api::ModEntry::new(
            "portal",
            Box::new(Portal(Arc::clone(&asked))),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            let mut resources = None;
            let mut game = client(&mut resources);
            game.handle_packet(open(1, "minecraft:the_nether")).unwrap();
            game.audio.capture = Some(Vec::new());
            let anvil = sound("minecraft:block.anvil.land", [452, 512, 66], 2.0, 0.5, 42);
            game.handle_packet(run(1, std::slice::from_ref(&anvil)))
                .unwrap();
            let played = game.audio.capture.replace(Vec::new()).unwrap();
            assert_eq!(played.len(), 1);
            assert_eq!(played[0].event, "minecraft:block.anvil.land");
            assert_eq!(played[0].position, [1.5, 70.0, -2.5]);
            assert_eq!((played[0].volume, played[0].pitch), (0.25, 1.5));
            assert_eq!(played[0].source, SoundSource::Blocks.index());
            let C::Sound(p) = &anvil else { unreachable!() };
            assert_eq!(
                game.ask_viewed_level_sound(1, ViewedSound::at_position(p)),
                Some(ViewedPlay {
                    name: "minecraft:block.anvil.land".into(),
                    source: SoundSource::Blocks,
                    at: [1.5, 70.0, -2.5],
                    volume: 0.25,
                    pitch: 1.5,
                    seed: 42,
                }),
                "the server's seed"
            );
            let bell = sound("minecraft:block.bell.use", [0, 512, 0], 1.0, 1.0, 5);
            let harp = sound("minecraft:block.note_block.harp", [8, 512, 8], 1.0, 1.0, 6);
            game.handle_packet(run(1, &[bell, harp])).unwrap();
            let played = game.audio.capture.replace(Vec::new()).unwrap();
            assert_eq!(played.len(), 0, "the bell left None, the harp not finite");
            // Bound to entity 40 of the viewed level: asked at the entity there, played where
            // the listener put it; bound to an entity the world does not hold, not asked.
            let pig = C::AddEntity(play::AddEntity {
                id: 40,
                uuid: 40,
                kind: 7,
                position: [12.5, 70.0, -3.25],
                velocity: [0.0; 3],
                pitch: 0,
                yaw: 0,
                head_yaw: 0,
                data: 0,
            });
            let bound = |id| {
                C::SoundEntity(play::SoundEntity {
                    sound: (-1, Some(("minecraft:block.anvil.land".into(), None))),
                    source: SoundSource::Blocks.index() as i32,
                    id,
                    volume: 1.0,
                    pitch: 1.0,
                    seed: 9,
                })
            };
            game.handle_packet(run(1, &[pig, bound(40), bound(41)]))
                .unwrap();
            let played = game.audio.capture.replace(Vec::new()).unwrap();
            assert_eq!(played.len(), 1);
            assert_eq!(played[0].position, [1.5, 70.0, -2.5]);
            // The player's own level: played where it is, without asking.
            game.handle_packet(sound(
                "minecraft:block.anvil.land",
                [8, 512, 8],
                1.0,
                1.0,
                7,
            ))
            .unwrap();
            let played = game.audio.capture.take().unwrap();
            assert_eq!(played.len(), 1);
            assert_eq!(played[0].position, [1.0, 64.0, 1.0]);
            let nether = Some("minecraft:the_nether");
            let blocks = SoundSource::Blocks;
            let anvil = |volume| {
                (
                    nether,
                    "minecraft:block.anvil.land".to_owned(),
                    blocks,
                    [56.5, 64.0, 8.25],
                    volume,
                    0.5,
                    42,
                )
            };
            assert_eq!(
                *asked.lock().unwrap(),
                [
                    anvil(2.0),
                    anvil(2.0),
                    (
                        nether,
                        "minecraft:block.bell.use".into(),
                        blocks,
                        [0.0, 64.0, 0.0],
                        1.0,
                        1.0,
                        5
                    ),
                    (
                        nether,
                        "minecraft:block.note_block.harp".into(),
                        blocks,
                        [1.0, 64.0, 1.0],
                        1.0,
                        1.0,
                        6
                    ),
                    (
                        nether,
                        "minecraft:block.anvil.land".into(),
                        blocks,
                        [12.5, 70.0, -3.25],
                        1.0,
                        1.0,
                        9
                    ),
                ]
            );
        });
    }

    /// With no listener a run of a viewed level's sounds, as the client's own decoder reads it,
    /// costs one `listened` check, allocates nothing and plays nothing: both sounds carry their
    /// event inline, which decoding them would allocate.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn with_no_listener_a_viewed_sound_is_dropped_after_one_check() {
        let mut resources = None;
        let mut game = client(&mut resources);
        game.handle_packet(open(1, "minecraft:the_nether")).unwrap();
        game.audio.capture = Some(Vec::new());
        let anvil = sound("minecraft:block.anvil.land", [452, 512, 66], 2.0, 0.5, 42);
        let bound = C::SoundEntity(play::SoundEntity {
            sound: (-1, Some(("minecraft:block.bell.use".into(), None))),
            source: SoundSource::Blocks.index() as i32,
            id: 40,
            volume: 1.0,
            pitch: 1.0,
            seed: 9,
        });
        let payload = run(1, &[anvil, bound]);
        let checks = || super::sounds::LISTENED_CHECKS.with(std::cell::Cell::get);
        let before = checks();
        let (routed, allocations) =
            horizon_threads::allocation_count::measure(|| game.route_viewed_levels(data(&payload)));
        assert_eq!(routed, Ok(()));
        assert_eq!(allocations, 0);
        assert_eq!(checks() - before, 1);
        assert_eq!(
            game.audio.capture.take().map(|played| played.len()),
            Some(0)
        );
    }
}
