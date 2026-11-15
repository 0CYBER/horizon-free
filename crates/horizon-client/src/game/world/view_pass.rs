//! The view pass (#3716): a level that only viewed `Loaded` areas name is not ticked, but its
//! chunks are loaded, generated and sent to the players whose clients view it.
//!
//! After the ticked levels, `tick_server` runs a pass for each such level: it enters the level,
//! rebuilds `Streaming.extra` (`lan_update_streaming`, where the areas are added: no `tick_lan`
//! runs here to call it), collects the finished columns (`update_world`), postprocesses them as
//! vanilla does before a chunk is sent (`postprocess_chunks`: what it schedules waits for a tick),
//! then runs only the hub's viewed-level sending, with the level's entities as they stand for its
//! viewers to track. Nothing ticks there: no block, fluid, scheduled or random ticks, no
//! entities, no weather cycle, no level tick hooks and no player work. Its time is the server's,
//! as in 26.3, where no level but the Overworld runs a clock of its own: `enter_level` gives it
//! the clocks as they are, and its viewers get its game time every 20 ticks. A level whose last
//! area was withdrawn keeps its passes, with no ticket, until its chunks were forgotten and it was
//! closed on the clients.
use super::{Dim, Game};
use horizon_client::{Result, lan::hub::TickCtx};

impl Game<'_> {
    /// What the players' viewed areas ask of level `dim`: None for nothing, else whether one of
    /// them ticks.
    pub(super) fn viewed_level(&self, dim: Dim) -> Option<bool> {
        self.lan.as_ref()?.hub.viewed_level(dim.id())
    }

    /// Whether level `dim` needs a view pass when it is not ticked: areas name it, or a client
    /// still holds chunks of it whose linger, forgets and `close` are not done.
    pub(super) fn needs_view_pass(&self, dim: Dim) -> bool {
        self.viewed_level(dim).is_some()
            || self
                .lan
                .as_ref()
                .is_some_and(|h| h.hub.viewed_open(dim.id()))
    }

    /// A view pass for each level the viewed areas name that `ticked` does not hold.
    pub(super) fn run_view_passes(&mut self, ticked: &[Dim]) -> Result<()> {
        if !self.lan.as_ref().is_some_and(|h| h.hub.viewed.active()) {
            return Ok(());
        }
        let viewed: Vec<Dim> = Dim::all()
            .into_iter()
            .filter(|d| !ticked.contains(d) && self.needs_view_pass(*d))
            .collect();
        for dim in viewed {
            let _span = horizon_client::spans::guard("view_pass");
            self.enter_level(dim)?;
            self.update_dedicated_streaming();
            self.lan_update_streaming();
            self.update_world();
            // `LevelChunk.postProcessGeneration` comes before a chunk is sent
            // (`ChunkMap.prepareTickingChunk`, then `onChunkReadyToSend`), ticked or not.
            self.postprocess_chunks()?;
            self.view_pass_send();
        }
        Ok(())
    }

    /// The hub's viewed-level sending for the level in place, without its tick.
    fn view_pass_send(&mut self) {
        let states = self.view_pass_states();
        let spawn = self.world_spawn_position();
        let clock_rate = self.network_clock_rate();
        let sea_level = self.end.locator().map_or(63, |terrain| terrain.sea_level());
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let mut ctx = TickCtx {
            sea_level,
            world: &mut self.world,
            entities: states.as_slice(),
            next_entity: &mut self.next_entity,
            spawn,
            spawn_angle: self.level.world_spawn_angle,
            difficulty: i32::from(self.ui.difficulty),
            difficulty_locked: self.level.difficulty_locked,
            clock_rate,
            tick_rate: self.clock.rate(),
            tick_frozen: self.clock.frozen(),
            tick_steps: self.clock.step_state(),
            collision: Some(self.collision),
            border: &self.border,
            streaming: Some(&self.streaming),
        };
        host.hub.viewed_pass(&mut ctx);
    }

    /// The level's entities as `tick_lan` describes them for the tracking pass: nobody is in the
    /// level, so no player rides one.
    fn view_pass_states(&mut self) -> horizon_client::lan::entity::States {
        let mut states = horizon_client::lan::entity::States::default();
        if !self.lan.as_ref().is_some_and(|h| h.hub.viewed_here()) {
            return states;
        }
        self.lan_assign_ids();
        let Some(host) = self.lan.as_mut() else {
            return states;
        };
        let mut riding = std::mem::take(&mut host.hub.riding);
        self.lan_riding(&[], &mut riding);
        if let Some(host) = self.lan.as_mut() {
            host.hub.riding = riding;
        }
        self.lan_entities(&mut states, true, 0, None);
        self.lan_hook_states(&mut states);
        self.lan_entity_uuids(&mut states);
        self.lan_syncs_looked();
        states
    }
}

#[cfg(test)]
pub(in crate::game) mod tests {
    use super::*;
    use horizon_protocol::{
        buf::Reader,
        packets::play::Clientbound as C,
        viewed_levels::{CHANNEL, Level::Index, Message, REVISION_3},
    };
    use std::time::{Duration, Instant};

    /// A server game on a scratch world, as `levels.rs`'s tests open one.
    pub(in crate::game) fn open_game<'a>(
        resources: &'a mut Option<std::sync::Arc<crate::game::Resources>>,
        root: &std::path::Path,
        home: &std::path::Path,
    ) -> Game<'a> {
        use crate::game::{Audio, Options, Progress, Settings, load_resources};
        let settings = Settings::default();
        let loaded = load_resources(
            horizon_assets::resource_source::ResourceRoot::directory(root.to_path_buf()),
            Vec::new(),
            None,
            root,
            &settings,
            Instant::now(),
            false,
            crate::game::execution_role::ExecutionRole::IntegratedServer,
            &Progress::new(),
            &mut |_, _| unreachable!(),
        )
        .unwrap();
        let resources = resources.insert(loaded.resources);
        let mut options = Options::parse(std::iter::empty()).unwrap().unwrap();
        options.resources = root.to_path_buf();
        options.game_dir = Some(home.to_path_buf());
        options.player_name = Some("Viewer".into());
        options.execution_role = crate::game::execution_role::ExecutionRole::IntegratedServer;
        let mut game = Game::new(
            resources,
            loaded.pack,
            loaded.gui.unwrap(),
            options,
            settings,
            Audio::silent(&horizon_assets::resource_source::ResourceSource::directory(
                root.to_path_buf(),
            )),
            None,
        )
        .unwrap();
        game.build_registries();
        let folder = home.join("saves/viewed").to_string_lossy().into_owned();
        game.load_world(
            crate::game::world_loading::WorldRequest::new(
                &folder,
                Some("viewed"),
                Some(3716),
                true,
                None,
                std::sync::Arc::new(crate::game::api_values::game_rule_registry()),
            )
            .unwrap(),
        )
        .unwrap();
        game
    }

    /// Opens the world to the network with one player in the Overworld whose client, of revision
    /// 3, views 2 levels; its packets.
    pub(in crate::game) fn host_a_viewer(
        game: &mut Game<'_>,
    ) -> std::sync::mpsc::Receiver<horizon_client::lan::Out> {
        use horizon_client::lan::{Joined, NetConfig, PlayerSender, guest::Guest, hub::HubConfig};
        let net = NetConfig {
            announce: false,
            ..NetConfig::lan(0, "view-pass-test".into(), 2)
        };
        let settings = crate::game::lan::LanSettings {
            port: 0,
            guest_commands: true,
            force_game_mode: false,
        };
        let hub = HubConfig {
            default_game_mode: 0,
            guest_commands: true,
            force_game_mode: false,
            hardcore: false,
            view_distance: 2,
            simulation_distance: 2,
            dimension: Dim::Overworld.id().into(),
            allow_flight: false,
            seed: 0,
            max_players: 2,
            integrated: false,
            levels: Default::default(),
        };
        game.lan_host(settings, net, hub).unwrap();
        let (tx, rx) = std::sync::mpsc::channel();
        let mut sender = PlayerSender::new(tx, std::sync::Arc::default());
        // Revision 3, still served, names a viewed level by its Login index: the Overworld 0, the
        // Nether 2.
        sender.viewed_levels = horizon_protocol::viewed_levels::Answer {
            revision: horizon_protocol::viewed_levels::REVISION_3,
            max_levels: 2,
        };
        let joined = Joined {
            conn: 2,
            uuid: 2,
            name: "Viewer".into(),
            properties: Vec::new(),
            info: horizon_protocol::packets::configuration::ClientInformation::new(2),
            brand: "horizon".into(),
            sender,
            latency: std::sync::Arc::new(std::sync::atomic::AtomicI32::new(0)),
            address: "127.0.0.1:25565".parse().unwrap(),
            expected: false,
        };
        game.next_entity += 1;
        let mut guest = Guest::new(joined, game.next_entity as i32, [0.5, 80.0, 0.5], 0);
        guest.placed = true;
        guest.loaded = true;
        guest.dimension = Dim::Overworld.id().into();
        guest.max_unacknowledged = 1000;
        let hub = &mut game.lan.as_mut().unwrap().hub;
        // Its client was told at Login the levels the server has now.
        guest.viewed.logged_in_with(hub.level_keys());
        hub.guests.push(guest);
        rx
    }

    /// What a client was sent of the Nether (level 2) on `horizon:levels`.
    #[derive(Default)]
    pub(in crate::game) struct Sent {
        pub(in crate::game) opened: bool,
        pub(in crate::game) chunks: Vec<(i32, i32)>,
        pub(in crate::game) forgotten: Vec<(i32, i32)>,
        pub(in crate::game) closed: bool,
        times: Vec<i64>,
        /// The other packets of its runs, in order.
        others: Vec<C>,
        sounds: Vec<horizon_protocol::packets::play::Sound>,
    }

    pub(in crate::game) fn sent(rx: &std::sync::mpsc::Receiver<horizon_client::lan::Out>) -> Sent {
        let mut sent = Sent::default();
        let bodies = rx.try_iter().flat_map(|o| match o {
            horizon_client::lan::Out::Packet(body) => vec![body],
            horizon_client::lan::Out::Packets(bodies) => bodies,
            horizon_client::lan::Out::Disconnect(_) => Vec::new(),
        });
        for body in bodies {
            let mut r = Reader::new(&body);
            let id = r.varint().unwrap();
            let Ok(C::CustomPayload(p)) = C::decode(id, r.rest()) else {
                continue;
            };
            if p.channel != CHANNEL {
                continue;
            }
            match Message::decode_in(&p.data, REVISION_3).unwrap() {
                Message::Open { level, .. } => sent.opened |= level == Index(2),
                Message::Close { level } => sent.closed |= level == Index(2),
                Message::Run {
                    level: Index(2),
                    packets,
                } => {
                    for (id, body) in packets {
                        match C::decode(id, body) {
                            Ok(C::LevelChunkWithLight(c)) => sent.chunks.push((c.x, c.z)),
                            Ok(C::ForgetLevelChunk(f)) => sent.forgotten.push((f.pos[0], f.pos[1])),
                            Ok(C::SetTime(t)) if t.clocks.is_empty() => {
                                sent.times.push(t.game_time)
                            }
                            Ok(C::Sound(p)) => sent.sounds.push(p),
                            Ok(other) => sent.others.push(other),
                            Err(_) => {}
                        }
                    }
                }
                _ => {}
            }
        }
        sent
    }

    /// The chunks of a radius-1 square at (40, 40).
    fn square() -> Vec<(i32, i32)> {
        (39..=41)
            .flat_map(|x| (39..=41).map(move |z| (x, z)))
            .collect()
    }

    /// A game on a scratch world in `HORIZON_HOME`'s `name`, its viewer, and a `Loaded` Nether
    /// area at (40, 40), far from where anything else streams, ticked through `tick_server` until
    /// its 9 chunks were sent: what the viewer was sent until then.
    fn stream_the_nether<'a>(
        resources: &'a mut Option<std::sync::Arc<crate::game::Resources>>,
        name: &str,
    ) -> (
        Game<'a>,
        std::sync::mpsc::Receiver<horizon_client::lan::Out>,
        Sent,
    ) {
        use horizon_client::lan::viewed_levels::areas::ChunkAreaMode;
        stream_the_nether_in(resources, name, ChunkAreaMode::Loaded)
    }

    /// [`stream_the_nether`] with an area of `mode`: a `Ticking` one ticks the Nether.
    fn stream_the_nether_in<'a>(
        resources: &'a mut Option<std::sync::Arc<crate::game::Resources>>,
        name: &str,
        mode: horizon_client::lan::viewed_levels::areas::ChunkAreaMode,
    ) -> (
        Game<'a>,
        std::sync::mpsc::Receiver<horizon_client::lan::Out>,
        Sent,
    ) {
        use horizon_client::lan::viewed_levels::areas::{ChunkArea, ChunkAreaMode};
        let ticking = mode == ChunkAreaMode::Ticking;
        let root = std::path::PathBuf::from(std::env::var_os("HORIZON_TEST_RESOURCES").unwrap());
        let home = std::path::PathBuf::from(std::env::var_os("HORIZON_HOME").unwrap()).join(name);
        let _ = std::fs::remove_dir_all(&home);
        let mut game = open_game(resources, &root, &home);
        let rx = host_a_viewer(&mut game);
        let area = ChunkArea {
            level: Dim::Nether.id().into(),
            center: (40, 40),
            radius: 1,
            mode,
        };
        let hub = &mut game.lan.as_mut().unwrap().hub;
        hub.set_viewed_areas(2, "test:portal", vec![area]).unwrap();
        let mut all = Sent::default();
        let deadline = Instant::now() + Duration::from_secs(120);
        while all.chunks.len() < 9 && Instant::now() < deadline {
            game.tick_server(false).unwrap();
            assert_eq!(
                game.levels_to_tick().contains(&Dim::Nether),
                ticking,
                "ticked only for a Ticking area"
            );
            let s = sent(&rx);
            all.opened |= s.opened;
            all.chunks.extend(s.chunks);
            all.times.extend(s.times);
            std::thread::sleep(Duration::from_millis(20));
        }
        all.chunks.sort_unstable();
        (game, rx, all)
    }

    /// The view pass generates, postprocesses and sends a `Loaded` area's chunks in a level nobody
    /// is in, keeps them loaded across ticks (`lan_update_streaming`'s rebuild keeps the area), never ticks
    /// the level, and sends its game time, the server's, every 20 ticks.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and a scratch HORIZON_HOME"]
    fn a_loaded_area_in_a_level_nobody_is_in_is_streamed_and_sent_without_ticking() {
        let mut resources = None;
        let (mut game, rx, mut all) = stream_the_nether(&mut resources, "view-pass");
        assert!(all.opened, "the Nether was opened on the client");
        assert_eq!(all.chunks, square(), "the area's chunks, wrapped");
        // Kept loaded for 40 more ticks, its clock the server's.
        for _ in 0..40 {
            game.tick_server(false).unwrap();
            all.times.extend(sent(&rx).times);
        }
        let world = game.levels.parked_world(Dim::Nether).unwrap();
        assert!(
            square().iter().all(|c| world.chunks.contains_key(c)),
            "still loaded"
        );
        assert!(
            square()
                .iter()
                .all(|c| world.chunks[c].postprocess.is_empty()),
            "postprocessed by the view pass"
        );
        assert_eq!(world.game_time, game.world.game_time);
        let times = all.times;
        assert!(times.len() >= 2, "a game time every 20 ticks: {times:?}");
        assert!(
            times.windows(2).all(|w| w[1] - w[0] == 20),
            "the server's clock: {times:?}"
        );
    }

    /// The last area of a level nobody is in, withdrawn: the level keeps its view passes, with
    /// no ticket and no loading square, until its chunks are forgotten 52 ticks later and it is
    /// closed on the client; then the passes stop.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and a scratch HORIZON_HOME"]
    fn a_withdrawn_last_area_is_forgotten_and_closed_through_tick_server() {
        let mut resources = None;
        let (mut game, rx, all) = stream_the_nether(&mut resources, "view-pass-withdraw");
        assert_eq!(all.chunks, square());
        let hub = &mut game.lan.as_mut().unwrap().hub;
        hub.set_viewed_areas(2, "test:portal", Vec::new()).unwrap();
        let nether = Dim::Nether.id();
        assert!(hub.viewed_tickets(nether).is_empty() && hub.viewed_extra(nether).is_empty());
        assert_eq!(game.viewed_level(Dim::Nether), None);
        assert!(game.needs_view_pass(Dim::Nether), "its view is still open");
        // The first pass after the withdrawal starts the linger at this hub tick.
        game.tick_server(false).unwrap();
        let start = game.lan.as_ref().unwrap().hub.ticks;
        assert!(sent(&rx).forgotten.is_empty());
        let mut forgot_at = None;
        let mut last = Sent::default();
        for _ in 0..60 {
            game.tick_server(false).unwrap();
            let s = sent(&rx);
            if !s.forgotten.is_empty() {
                forgot_at = Some(game.lan.as_ref().unwrap().hub.ticks - start);
                last = s;
                break;
            }
        }
        assert_eq!(
            forgot_at,
            Some(52),
            "forgotten 52 ticks after the linger started"
        );
        last.forgotten.sort_unstable();
        assert_eq!(last.forgotten, square());
        assert!(last.closed, "closed once nothing of it is left");
        let hub = &game.lan.as_ref().unwrap().hub;
        let viewer = hub.guests.iter().find(|g| g.uuid == 2).unwrap();
        assert!(viewer.viewed.view(nether).is_none());
        assert!(!hub.viewed.active());
        assert!(!game.needs_view_pass(Dim::Nether), "no more passes");
    }

    /// What the viewer was sent of entity `id` in the Nether's runs: where `AddEntity` put it and
    /// where each move after took it (the client's `VecDeltaCodec`), and whether it was removed.
    #[derive(Debug, Default)]
    struct Tracked {
        added: Option<(i32, [f64; 3])>,
        moves: Vec<[f64; 3]>,
        /// Its entity events (`EntityEvent`).
        events: Vec<i8>,
        removed: bool,
    }

    fn tracked(packets: &[C], id: i32) -> Tracked {
        use horizon_protocol::packets::play;
        let mut t = Tracked::default();
        let mut base = [0.0; 3];
        for packet in packets {
            match packet {
                C::AddEntity(p) if p.id == id => {
                    t.added = Some((p.kind, p.position));
                    base = p.position;
                }
                C::MoveEntityPos(play::MoveEntityPos(m))
                | C::MoveEntityPosRot(play::MoveEntityPosRot(m))
                    if m.id == id =>
                {
                    base = m.delta.as_ref().unwrap().end(base);
                    t.moves.push(base);
                }
                C::EntityPositionSync(p) if p.id == id => {
                    base = p.path.end();
                    t.moves.push(base);
                }
                C::EntityEvent(p) if p.id == id => t.events.push(p.event),
                C::RemoveEntities(p) if p.ids.contains(&id) => t.removed = true,
                _ => {}
            }
        }
        t
    }

    /// Summons `entity` in the Nether, at chunk (40, 40) above its bedrock roof, and gives its id.
    fn summon_in_the_nether(game: &mut Game<'_>, entity: &str) -> i32 {
        game.enter_level(Dim::Nether).unwrap();
        game.set_allow_commands(true);
        assert!(game.run_command(&format!("summon {entity} 648.5 130 648.5")));
        let summoned = game.mobs.iter().find(|m| m.kind.id == entity).unwrap();
        summoned.id as i32
    }

    /// An entity in a viewed `Ticking` Nether area reaches the viewer as a wrapped `AddEntity`,
    /// then its fall onto the bedrock roof (y 128) as wrapped moves, and once killed its death
    /// event (3) and, its death animation over, its removal.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and a scratch HORIZON_HOME"]
    fn an_entity_in_a_ticking_viewed_area_is_added_moved_and_removed_wrapped() {
        use horizon_client::lan::viewed_levels::areas::ChunkAreaMode;
        let mut resources = None;
        let (mut game, rx, _) =
            stream_the_nether_in(&mut resources, "view-entities", ChunkAreaMode::Ticking);
        let pig = summon_in_the_nether(&mut game, "minecraft:pig");
        let mut packets = Vec::new();
        for _ in 0..60 {
            game.tick_server(false).unwrap();
            packets.extend(sent(&rx).others);
        }
        let t = tracked(&packets, pig);
        let (_, at) = t.added.expect("added, wrapped in a Nether run");
        assert_eq!(at, [648.5, 130.0, 648.5]);
        let last = *t.moves.last().expect("its fall, wrapped");
        // It may walk off once it landed.
        assert_eq!(last[1], 128.0, "on the bedrock roof: {:?}", t.moves);
        assert!(!t.removed);
        game.enter_level(Dim::Nether).unwrap();
        assert!(game.run_command("kill @e[type=minecraft:pig]"));
        for _ in 0..40 {
            game.tick_server(false).unwrap();
            packets.extend(sent(&rx).others);
        }
        let t = tracked(&packets, pig);
        assert!(t.events.contains(&3), "its death, wrapped: {:?}", t.events);
        assert!(t.removed, "removed, wrapped");
        let viewer = game
            .lan
            .as_ref()
            .unwrap()
            .hub
            .guests
            .iter()
            .find(|g| g.uuid == 2);
        assert!(
            !viewer.unwrap().tracker.is_tracking(pig),
            "not the player's own"
        );
    }

    /// An entity in a viewed `Loaded` Nether area is added, wrapped, and stands still in the air:
    /// nothing ticks there, and a position sync, if any, gives the same place.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and a scratch HORIZON_HOME"]
    fn an_entity_in_a_loaded_viewed_area_is_added_and_stands_still() {
        let mut resources = None;
        let (mut game, rx, _) = stream_the_nether(&mut resources, "view-entities-loaded");
        let pig = summon_in_the_nether(&mut game, "minecraft:pig");
        let mut packets = Vec::new();
        for _ in 0..60 {
            game.tick_server(false).unwrap();
            packets.extend(sent(&rx).others);
        }
        let t = tracked(&packets, pig);
        assert_eq!(t.added.map(|(_, at)| at), Some([648.5, 130.0, 648.5]));
        assert!(
            t.moves.iter().all(|&at| at == [648.5, 130.0, 648.5]),
            "no move: {:?}",
            t.moves
        );
    }

    /// A packet a client was sent: a plain one, or what `horizon:levels` carried.
    #[derive(Debug)]
    enum Got {
        Plain(C),
        Open(i32),
        Wrapped(i32, C),
        Close(i32),
    }

    /// Everything the viewer was sent since the last call, in order, each packet decoded.
    fn every(rx: &std::sync::mpsc::Receiver<horizon_client::lan::Out>) -> Vec<Got> {
        let bodies = rx.try_iter().flat_map(|o| match o {
            horizon_client::lan::Out::Packet(body) => vec![body],
            horizon_client::lan::Out::Packets(bodies) => bodies,
            horizon_client::lan::Out::Disconnect(_) => Vec::new(),
        });
        let mut got = Vec::new();
        for body in bodies {
            let mut r = Reader::new(&body);
            let id = r.varint().unwrap();
            let Ok(packet) = C::decode(id, r.rest()) else {
                continue;
            };
            let C::CustomPayload(p) = &packet else {
                got.push(Got::Plain(packet));
                continue;
            };
            if p.channel != CHANNEL {
                continue;
            }
            match Message::decode_in(&p.data, REVISION_3).unwrap() {
                Message::Open { level, .. } => got.push(Got::Open(level.number())),
                Message::Close { level } => got.push(Got::Close(level.number())),
                Message::Run { level, packets } => got.extend(
                    (packets.into_iter())
                        .filter_map(|(id, body)| C::decode(id, body).ok())
                        .map(|packet| Got::Wrapped(level.number(), packet)),
                ),
                Message::Swap { .. } => unreachable!("no swap below revision 4"),
            }
        }
        got
    }

    /// The viewer, a server player.
    fn viewer<'h>(game: &'h Game<'_>) -> &'h horizon_client::lan::guest::Guest {
        let hub = &game.lan.as_ref().unwrap().hub;
        hub.guests.iter().find(|g| g.uuid == 2).unwrap()
    }

    /// Sends the viewer (connection 2) into level `dim` at `at`, as `/tp` into another level
    /// does (`ServerPlayer.teleport(TeleportTransition)`).
    fn travel(game: &mut Game<'_>, dim: Dim, at: [f64; 3]) {
        game.level_teleports
            .push(crate::game::server_travel::LevelTeleport {
                conn: 2,
                dim,
                at,
                rotation: None,
                vehicle: None,
            });
        game.run_level_teleports();
    }

    /// The Respawns the viewer was sent: their level and what they keep.
    fn respawns(got: &[Got]) -> Vec<(String, i8)> {
        (got.iter())
            .filter_map(|g| match g {
                Got::Plain(C::Respawn(p)) => Some((p.spawn.dimension.clone(), p.keep)),
                _ => None,
            })
            .collect()
    }

    /// A travel into the viewed Nether: a Respawn keeping all data, and none of the Nether's
    /// chunks sent again, plain or wrapped: they are the player's own now. The Overworld's chunks
    /// are a view of level 0 instead of being forgotten: no plain forget, no `open`; nothing
    /// covers them, so 52 ticks on they are forgotten wrapped and the level is closed.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and a scratch HORIZON_HOME"]
    fn a_travel_into_the_viewed_nether_sends_no_chunk_again_and_keeps_the_overworld_viewed() {
        let mut resources = None;
        let (mut game, rx, all) = stream_the_nether(&mut resources, "view-travel");
        assert_eq!(all.chunks, square());
        // The Overworld's chunks the player's client holds, around the spawn.
        let overworld: Vec<(i32, i32)> = (-1..=1)
            .flat_map(|x| (-1..=1).map(move |z| (x, z)))
            .collect();
        let hub = &mut game.lan.as_mut().unwrap().hub;
        let i = hub.guests.iter().position(|g| g.uuid == 2).unwrap();
        hub.guests
            .update(i, |g| g.sent_chunks = overworld.iter().copied().collect());
        let _ = every(&rx);
        travel(&mut game, Dim::Nether, [648.5, 130.0, 648.5]);
        let g = viewer(&game);
        assert_eq!(g.dimension, Dim::Nether.id());
        assert!(square().iter().all(|c| g.sent_chunks.contains(c)));
        assert!(g.viewed.view(Dim::Nether.id()).is_none(), "its own level");
        let kept = g
            .viewed
            .view(Dim::Overworld.id())
            .expect("the Overworld, viewed");
        let mut kept_sent: Vec<(i32, i32)> = kept.sent.iter().copied().collect();
        kept_sent.sort_unstable();
        assert_eq!((kept.level, kept_sent), (Index(0), overworld.clone()));
        let mut got = every(&rx);
        for _ in 0..70 {
            game.tick_server(false).unwrap();
            got.extend(every(&rx));
        }
        assert_eq!(respawns(&got), vec![(Dim::Nether.id().to_owned(), 3)]);
        let resent: Vec<(i32, i32)> = (got.iter())
            .filter_map(|g| match g {
                Got::Plain(C::LevelChunkWithLight(c))
                | Got::Wrapped(_, C::LevelChunkWithLight(c)) => Some((c.x, c.z)),
                _ => None,
            })
            .collect();
        assert!(
            square().iter().all(|c| !resent.contains(c)),
            "sent again: {resent:?}"
        );
        let plain_forgets: Vec<(i32, i32)> = (got.iter())
            .filter_map(|g| match g {
                Got::Plain(C::ForgetLevelChunk(f)) => Some((f.pos[0], f.pos[1])),
                _ => None,
            })
            .collect();
        assert!(
            overworld.iter().all(|c| !plain_forgets.contains(c)),
            "{plain_forgets:?}"
        );
        assert!(!got.iter().any(|g| matches!(g, Got::Open(0))), "no open");
        let mut forgotten: Vec<(i32, i32)> = (got.iter())
            .filter_map(|g| match g {
                Got::Wrapped(0, C::ForgetLevelChunk(f)) => Some((f.pos[0], f.pos[1])),
                _ => None,
            })
            .collect();
        forgotten.sort_unstable();
        assert_eq!(forgotten, overworld, "forgotten wrapped after the linger");
        let last_forget = got
            .iter()
            .rposition(|g| matches!(g, Got::Wrapped(0, C::ForgetLevelChunk(_))));
        let close = got.iter().position(|g| matches!(g, Got::Close(0)));
        assert!(close > last_forget, "closed once forgotten");
        assert!(viewer(&game).viewed.view(Dim::Overworld.id()).is_none());
    }

    /// A travel into a level the client does not view drops its views: the Nether is forgotten
    /// without `close` and, its area still asking for it, opened again after the Respawn and its
    /// chunks sent again; the Overworld left is not kept.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and a scratch HORIZON_HOME"]
    fn a_travel_into_a_level_not_viewed_drops_the_views() {
        let mut resources = None;
        let (mut game, rx, all) = stream_the_nether(&mut resources, "view-travel-end");
        assert_eq!(all.chunks, square());
        travel(&mut game, Dim::End, [8.5, 80.0, 8.5]);
        let g = viewer(&game);
        assert_eq!(g.dimension, Dim::End.id());
        assert!(g.viewed.view(Dim::Nether.id()).is_none(), "forgotten");
        assert!(g.viewed.view(Dim::Overworld.id()).is_none(), "not kept");
        let mut got = every(&rx);
        let deadline = Instant::now() + Duration::from_secs(120);
        let nether_chunks = |got: &[Got]| {
            let mut c: Vec<(i32, i32)> = (got.iter())
                .filter_map(|g| match g {
                    Got::Wrapped(2, C::LevelChunkWithLight(c)) => Some((c.x, c.z)),
                    _ => None,
                })
                .collect();
            c.sort_unstable();
            c
        };
        while nether_chunks(&got).len() < 9 && Instant::now() < deadline {
            game.tick_server(false).unwrap();
            got.extend(every(&rx));
            std::thread::sleep(Duration::from_millis(20));
        }
        assert_eq!(respawns(&got), vec![(Dim::End.id().to_owned(), 3)]);
        let respawn = got
            .iter()
            .position(|g| matches!(g, Got::Plain(C::Respawn(_))))
            .unwrap();
        let open = got.iter().position(|g| matches!(g, Got::Open(2)));
        assert!(
            open.is_some_and(|o| o > respawn),
            "opened again after the Respawn"
        );
        assert_eq!(nether_chunks(&got), square(), "sent again");
        assert!(
            !got.iter()
                .any(|g| matches!(g, Got::Close(_) | Got::Open(0)))
        );
    }

    /// Runs `command` in the Overworld, where the player is, then a tick: what the viewer's Nether
    /// was told of its clocks meanwhile, each update's clocks as their totals and rates.
    fn clocks_after(
        game: &mut Game<'_>,
        rx: &std::sync::mpsc::Receiver<horizon_client::lan::Out>,
        command: &str,
    ) -> Vec<Vec<(i64, f32)>> {
        game.enter_level(Dim::Overworld).unwrap();
        assert!(game.run_command(command), "{command}");
        game.tick_server(false).unwrap();
        (sent(rx).others.into_iter())
            .filter_map(|p| match p {
                C::SetTime(t) => Some(t.clocks.iter().map(|c| (c.total_ticks, c.rate)).collect()),
                _ => None,
            })
            .collect()
    }

    /// The server's clocks reach a viewer of a level nobody is in when they change, not only at
    /// `open`: `time set`, `time add` and the day stopping each send the clocks, wrapped.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and a scratch HORIZON_HOME"]
    fn clock_changes_reach_the_viewer_of_a_level_nobody_is_in() {
        let mut resources = None;
        let (mut game, rx, _) = stream_the_nether(&mut resources, "view-clocks");
        game.set_allow_commands(true);
        let told = clocks_after(&mut game, &rx, "time set 1234");
        assert_eq!(told.len(), 1, "{told:?}");
        assert!(
            told[0]
                .iter()
                .any(|&(total, rate)| (1234..=1235).contains(&total) && rate == 1.0),
            "{told:?}"
        );
        let told = clocks_after(&mut game, &rx, "time add 1000");
        assert!(
            told[0]
                .iter()
                .any(|&(total, _)| (2234..=2236).contains(&total)),
            "{told:?}"
        );
        // A clock that only runs on sends nothing but the game time every 20 ticks.
        for _ in 0..30 {
            assert_eq!(
                clocks_after(&mut game, &rx, "seed"),
                Vec::<Vec<(i64, f32)>>::new()
            );
        }
        let told = clocks_after(&mut game, &rx, "gamerule advance_time false");
        assert!(told[0].iter().any(|&(_, rate)| rate == 0.0), "{told:?}");
    }

    /// A mod's sound in the Nether, a level nobody is in, reaches the viewer whose client holds
    /// its chunk, wrapped, through `tick_server` (#3983).
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and a scratch HORIZON_HOME"]
    fn a_sound_in_a_viewed_level_reaches_its_viewer_through_tick_server() {
        let mut resources = None;
        let (mut game, rx, all) = stream_the_nether(&mut resources, "view-pass-sound");
        assert_eq!(all.chunks, square());
        assert!(all.sounds.is_empty());
        // A mod's handle needs the world's epoch, which this test world never began.
        game.native_lifecycle
            .begin_open(mod_api::Authority::IntegratedServer);
        let nether = game.level(Dim::Nether.id()).unwrap();
        let blocks = mod_api::SoundSource::Blocks;
        let at = [648.5, 70.0, 650.25];
        assert!(game.level_play_sound(nether, "minecraft:block.bell.use", at, blocks, 1.0, 0.75));
        game.tick_server(false).unwrap();
        let sounds = sent(&rx).sounds;
        assert_eq!(sounds.len(), 1, "{sounds:?}");
        let sound = &sounds[0];
        assert_eq!((sound.x, sound.y, sound.z), (5188, 560, 5202));
        assert_eq!((sound.source, sound.volume, sound.pitch), (4, 1.0, 0.75));
    }

    /// A mod's sound bound to a pig in the viewed Nether reaches the viewer whose view tracks the
    /// pig, wrapped, bound to the same entity, through `tick_server` (#3983).
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES and a scratch HORIZON_HOME"]
    fn a_sound_bound_to_a_viewed_entity_reaches_the_view_tracking_it() {
        let mut resources = None;
        let (mut game, rx, _) = stream_the_nether(&mut resources, "view-pass-bound-sound");
        let pig = summon_in_the_nether(&mut game, "minecraft:pig");
        for _ in 0..60 {
            game.tick_server(false).unwrap();
        }
        let _ = sent(&rx);
        let tracks = |game: &Game<'_>| {
            let view = viewer(game).viewed.view(Dim::Nether.id()).unwrap();
            view.tracker.is_tracking(pig)
        };
        assert!(tracks(&game), "the view tracks the pig");
        game.enter_level(Dim::Nether).unwrap();
        game.native_lifecycle
            .begin_open(mod_api::Authority::IntegratedServer);
        let emitter = game.entity(pig as u32).unwrap();
        let neutral = mod_api::SoundSource::Neutral;
        let sound = "minecraft:entity.pig.ambient";
        assert!(game.entity_play_bound_sound(emitter, sound, neutral, 1.0, 1.25));
        game.tick_server(false).unwrap();
        let bound: Vec<_> = (sent(&rx).others.into_iter())
            .filter_map(|p| match p {
                C::SoundEntity(p) => Some((p.id, p.source, p.volume, p.pitch)),
                _ => None,
            })
            .collect();
        assert_eq!(bound, [(pig, 6, 1.0, 1.25)]);
    }
}
