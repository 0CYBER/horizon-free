//! Events, explosions and lightning in a level for native server mods,
//! through a `LevelRef` (docs/modding/src/how-to/native-mods.md, "Entity and level handles"):
//! `Level.levelEvent`, `Level.gameEvent`, `ServerLevel.blockEvent`,
//! `Level.explode` and a summoned lightning bolt. Like the level's block
//! writes, they act on the level in place, the one the hook runs in; a parked
//! level, a stale handle or a client returns false. Reaching a parked level, as `play_sound` and the
//! reads do through `native_level_world`, is a follow-up.
use super::{Game, GameLevel};
use horizon_world::level::{BlockEventData, LevelEvent};
use mod_api::{
    BlockKey, EntityRef, ExplodeError, ExplosionInteraction, ExplosionSettings, LevelRef,
};

/// A position inside the world's horizontal and vertical bounds.
fn in_bounds(at: [f64; 3]) -> bool {
    at.iter().all(|v| v.is_finite())
        && at[0].abs() < 30_000_000.0
        && at[2].abs() < 30_000_000.0
        && at[1].abs() < 20_000_000.0
}

/// Whether level event `id` can play with `data`: the events that carry a
/// block state (2001, 2014, 3008) need `data` to be one of the `states`.
fn level_event_data_ok(id: i32, data: i32, states: usize) -> bool {
    !matches!(id, 2001 | 2014 | 3008) || usize::try_from(data).is_ok_and(|d| d < states)
}

impl GameLevel<'_> {
    /// `Level.levelEvent(null, id, pos, data)` for a hook that holds the
    /// level's write host: the event joins the ones the block behaviour
    /// emits, in order, and plays when the write that runs the hook is
    /// done (`process_level_events`), with the guests told as for any event
    /// the behaviour raises. False off an authoritative server, or for an
    /// event that carries a block state (see `level_level_event`)
    /// when `data` isn't one.
    pub(super) fn native_level_event(&mut self, id: i32, pos: [i32; 3], data: i32) -> bool {
        if !self.native_authoritative || !level_event_data_ok(id, data, self.facts.states.len()) {
            return false;
        }
        self.events.push(LevelEvent::LevelEvent {
            id,
            pos: horizon_core::position::BlockPos::new(pos[0], pos[1], pos[2]),
            data,
            source: None,
        });
        true
    }
}

impl Game<'_> {
    /// Plays level event `id` at `pos` with `data` for the players near it
    /// (`Level.levelEvent(null, id, pos, data)`): `1000`s are sounds, `2000`s
    /// particles, as vanilla's `LevelEvent` numbers them (`2001` breaks a
    /// block with state id `data`). No player is left out, the acting one
    /// included. False for an event that carries a block state (2001, 2014,
    /// 3008) when `data` isn't one.
    pub(super) fn level_level_event(
        &mut self,
        level: LevelRef,
        id: i32,
        pos: [i32; 3],
        data: i32,
    ) -> bool {
        if !self.level_in_place(level) {
            return false;
        }
        if !level_event_data_ok(id, data, self.res.facts().states.len()) {
            return false;
        }
        self.level_event_at(id, pos, data);
        true
    }
    /// Plays level event `id` for every player on the server
    /// (`ServerLevel.globalLevelEvent`), or for the players near `pos` when the
    /// `global_sound_events` rule is off. Refuses what `level_level_event` refuses.
    pub(super) fn level_global_level_event(
        &mut self,
        level: LevelRef,
        id: i32,
        pos: [i32; 3],
        data: i32,
    ) -> bool {
        if !self.level_in_place(level) {
            return false;
        }
        if !level_event_data_ok(id, data, self.res.facts().states.len()) {
            return false;
        }
        self.global_level_event(id, pos, data);
        true
    }
    /// Sends game event `event` (`minecraft:block_change`...) from `at`, as
    /// `Level.gameEvent(event, at, context)`, with validated source and state.
    pub(super) fn level_game_event(
        &mut self,
        level: LevelRef,
        event: &str,
        at: [f64; 3],
        context: mod_api::GameEventContext,
    ) -> bool {
        let Some(event) = horizon_client::sculk::intern(event) else {
            return false;
        };
        if !self.level_in_place(level) || !in_bounds(at) {
            return false;
        }
        self.post_contextual_game_event(event, at, context)
    }
    /// Queues block event `a`, `b` for `block` at `pos` in the level in place
    /// (`ServerLevel.blockEvent`): `run_block_events` hands it to the block's
    /// `triggerEvent` after this tick's scheduled ticks if `pos` still holds
    /// `block`. False for an unresolved block key, a position outside the
    /// build height (it could never reach a block), a parked level, a stale
    /// handle or a client.
    pub(super) fn level_block_event(
        &mut self,
        level: LevelRef,
        pos: [i32; 3],
        block: BlockKey,
        a: i32,
        b: i32,
    ) -> bool {
        let Some(block) = super::api_values::behavior_block(block, self.res.behaviours()) else {
            return false;
        };
        if !self.level_in_place(level) || !self.world.shape.contains_y(pos[1]) {
            return false;
        }
        self.block_ticks.queue_block_event(BlockEventData {
            pos: horizon_core::position::BlockPos::new(pos[0], pos[1], pos[2]),
            block,
            param_a: a,
            param_b: b,
            note_override: None,
        });
        true
    }
    /// Explodes at `center` as vanilla's full `Level.explode(source,
    /// damageSource, calculator, x, y, z, radius, fire, interaction, particles...,
    /// sound)` does (`native_explode`), the causing entity derived from
    /// `source`. Refused, with nothing exploded, for another level, a stale
    /// `source` or one in another level, once the entity ids are exhausted, and as
    /// `native_explode` refuses.
    pub(super) fn level_explode(
        &mut self,
        level: LevelRef,
        source: Option<EntityRef>,
        center: [f64; 3],
        radius: f32,
        fire: bool,
        interaction: ExplosionInteraction,
        settings: ExplosionSettings,
    ) -> Result<(), ExplodeError> {
        if self.entity_ids_exhausted() {
            return Err(ExplodeError::Unavailable);
        }
        if !self.level_in_place(level) {
            return Err(ExplodeError::Unavailable);
        }
        let source = match source {
            Some(entity) => Some(
                self.native_explosion_source(entity)
                    .ok_or(ExplodeError::Unavailable)?,
            ),
            None => None,
        };
        self.native_explode(source, center, radius, fire, interaction, settings)
    }
    /// Strikes lightning at `at`, as summoning a `minecraft:lightning_bolt`
    /// does: it sets fire, hurts and converts what it hits (a pig to a
    /// zombified piglin...) and every player hears the thunder. With
    /// `visual_only` it only flashes and sounds (`setVisualOnly`). The
    /// `lightning_strike` hook hears it first with the cause `mod` and may
    /// cancel it. False for a chunk that isn't loaded or a canceled strike.
    pub(super) fn level_strike_lightning(
        &mut self,
        level: LevelRef,
        at: [f64; 3],
        visual_only: bool,
    ) -> bool {
        if !self.level_in_place(level) || !in_bounds(at) {
            return false;
        }
        let chunk = ((at[0].floor() as i32) >> 4, (at[2].floor() as i32) >> 4);
        if !self.world.chunks.contains_key(&chunk) {
            return false;
        }
        let block = [
            at[0].floor() as i32,
            at[1].floor() as i32,
            at[2].floor() as i32,
        ];
        if !self.lightning_strike_allowed(None, block, "mod") {
            return false;
        }
        self.spawn_bolt(at, visual_only);
        true
    }
}

#[cfg(test)]
mod tests {
    use crate::game::LevelDimension;
    use horizon_world::level::LevelEvent;
    use mod_api::{self, BlockPart, Flow, Hooks, RandomTickEvent, Registrar};

    /// A stone part whose random tick plays level events through the write host.
    struct Events;
    #[mod_api::horizon_part]
    impl BlockPart for Events {
        fn random_tick(
            &self,
            level: &mut dyn mod_api::LevelWriteHost,
            event: &mut RandomTickEvent,
        ) -> Flow {
            // 2001 carries a block state: `data` -1 is none, so it is refused.
            assert!(!level.level_event(2001, [1, 64, 1], -1));
            assert!(level.level_event(1500, [1, 64, 1], 0));
            assert!(level.level_event(2005, [2, 64, 2], 7));
            let _ = event;
            Flow::Continue
        }
    }
    struct EventsMod;
    #[mod_api::__subscriptions]
    impl Hooks for EventsMod {
        fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            let block_key = r.block_key("minecraft:stone");
            r.block_part(block_key, Events);
        }
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_queued_block_event_runs_the_blocks_trigger_once() {
        use horizon_core::position::BlockPos;
        use mod_api::{LevelRef, WorldEpoch};
        use mod_platform::__private::part_tests::block_key;
        super::super::native_entity_refs::tests::with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            let nether = LevelRef::new(level.epoch(), mod_api::DimensionKey::NETHER);
            game.world.chunks.insert(
                (0, 0),
                std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
            );
            let note = game
                .pack
                .assets
                .registry
                .default_state("minecraft:note_block")
                .unwrap();
            game.world.set(0, 64, 0, note).unwrap();
            let (note_key, stone_key) = (block_key("minecraft:note_block"), block_key("stone"));

            // Queued twice, it runs once; one queued for a block `pos` no longer holds is
            // dropped when it runs, as `runBlockEvents` checks the block.
            assert!(level.block_event(game, [0, 64, 0], note_key, 0, 0));
            assert!(level.block_event(game, [0, 64, 0], note_key, 0, 0));
            assert!(level.block_event(game, [0, 64, 0], stone_key, 0, 0));
            assert_eq!(game.block_ticks.block_events.len(), 2);
            let (bottom, height) = level.height(game).unwrap();
            assert!(!level.block_event(game, [0, bottom - 1, 0], note_key, 0, 0));
            assert!(
                !level.block_event(game, [0, bottom + height, 0], note_key, 0, 0),
                "above the build height"
            );
            assert!(!stale.block_event(game, [0, 64, 0], note_key, 0, 0));
            assert!(
                !nether.block_event(game, [0, 64, 0], note_key, 0, 0),
                "a level not in place"
            );
            assert_eq!(
                game.block_ticks.block_events.len(),
                2,
                "refusals queue nothing"
            );

            game.level_events.clear();
            game.with_level(|b, level, ticks| b.run_block_events(level, ticks));
            assert!(game.block_ticks.block_events.is_empty());
            let handled: Vec<_> = game
                .level_events
                .iter()
                .filter(|e| matches!(e, LevelEvent::BlockEntityEvent { .. }))
                .collect();
            assert_eq!(
                handled,
                [&LevelEvent::BlockEntityEvent {
                    pos: BlockPos::new(0, 64, 0),
                    state: note,
                    a: 0,
                    b: 0,
                }],
                "the note block's triggerEvent handled it once, for its players"
            );
        });
    }

    #[test]
    #[ignore = "needs prepared resources and an isolated process for native mods"]
    fn a_level_event_in_a_write_hook_joins_the_events_in_order() {
        let mods: mod_api::Mods =
            vec![mod_api::ModEntry::new("level-events", Box::new(EventsMod))].into();
        crate::game::api_values::install_parts(&mods).unwrap();
        mod_platform::testing::with_mods(mods, || {
            super::super::native_guest_turn_tests::with_guests(|game| {
                game.options.dedicated = Some(Vec::new());
                game.world.chunks.insert(
                    (0, 0),
                    std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
                );
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(0, 64, 0, stone).unwrap();
                let before = LevelEvent::LevelEvent {
                    id: 1000,
                    pos: horizon_core::position::BlockPos::new(0, 64, 0),
                    data: 0,
                    source: None,
                };
                game.level_events.push(before.clone());
                let state = game.world.block(0, 64, 0);
                let mut handled = false;
                game.as_guest(12, |game| {
                    game.audio.capture = Some(Vec::new());
                    game.with_level(|_, level, ticks| {
                        level.native_random_tick(
                            ticks,
                            LevelDimension::Overworld,
                            horizon_core::position::BlockPos::new(0, 64, 0),
                            state,
                            &mut handled,
                        );
                    });
                    let events = game.level_events.clone();
                    game.lan_record_events(&events);
                    assert_eq!(
                        game.lan_events[1],
                        super::super::QueuedLevelEvent {
                            kind: 1500,
                            pos: [1, 64, 1],
                            data: 0,
                            source: None,
                        }
                    );
                    assert_eq!(
                        game.lan_events[2],
                        super::super::QueuedLevelEvent {
                            kind: 2005,
                            pos: [2, 64, 2],
                            data: 7,
                            source: None,
                        }
                    );
                })
                .unwrap();
                let at = |id, x, z, data| LevelEvent::LevelEvent {
                    id,
                    pos: horizon_core::position::BlockPos::new(x, 64, z),
                    data,
                    source: None,
                };
                assert_eq!(
                    game.level_events,
                    [before, at(1500, 1, 1, 0), at(2005, 2, 2, 7)]
                );
            });
        });
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn a_global_level_event_reaches_every_player_or_falls_back_to_a_plain_one() {
        use horizon_client::gamerules::Value;
        use mod_api::{LevelRef, WorldEpoch};
        super::super::native_entity_refs::tests::with_game(|game| {
            let level = game.level(game.dim.id()).unwrap();
            let stale = LevelRef::new(WorldEpoch::new(level.epoch().get() + 1), level.dimension());
            game.audio.capture = Some(Vec::new());
            game.lan_global_events.clear();
            game.lan_events.clear();

            // An End portal opening: one packet with the global flag for every player.
            assert!(level.global_level_event(game, 1038, [10, 64, -3], 0));
            assert_eq!(
                game.lan_global_events,
                [("minecraft:overworld", 1038, [10, 64, -3], 0)]
            );
            assert!(
                game.lan_events.is_empty(),
                "no plain event while the rule is on"
            );
            assert!(
                !level.global_level_event(game, 2001, [0, 64, 0], -1),
                "2001 carries a block state"
            );
            assert!(!stale.global_level_event(game, 1038, [0, 64, 0], 0));
            assert_eq!(game.lan_global_events.len(), 1, "refusals send nothing");

            // ServerLevel.globalLevelEvent without global_sound_events: levelEvent(null, ...).
            assert!(game.ui.rules.set("global_sound_events", Value::Bool(false)));
            game.lan_global_events.clear();
            assert!(level.global_level_event(game, 1038, [10, 64, -3], 0));
            assert!(game.lan_global_events.is_empty());
            assert_eq!(
                game.lan_events,
                [super::super::QueuedLevelEvent {
                    kind: 1038,
                    pos: [10, 64, -3],
                    data: 0,
                    source: None,
                }]
            );
        });
    }

    /// `ServerLevel.globalLevelEvent` with `global_sound_events` off is
    /// `levelEvent(null, ...)`: a player within 64 blocks gets the plain packet
    /// at the event's position, and one further away gets nothing.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn with_the_rule_off_a_global_level_event_reaches_a_player_in_range_as_a_plain_one() {
        use super::super::lan::LanSettings;
        use horizon_client::gamerules::Value;
        use horizon_client::lan::{
            Joined, NetConfig, Out, PlayerSender, guest::Guest, hub::HubConfig,
        };
        use horizon_protocol::packets::{
            configuration::ClientInformation,
            play::{Clientbound, LevelEvent},
        };
        use std::sync::{
            Arc,
            atomic::{AtomicI32, AtomicUsize},
        };
        super::super::native_entity_refs::tests::with_game(|game| {
            let net = NetConfig {
                listen: false,
                announce: false,
                lod_distance: 0,
                ..NetConfig::lan(0, String::new(), 2)
            };
            let config = HubConfig {
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: game.dim.id().to_owned(),
                allow_flight: true,
                seed: 0,
                max_players: 2,
                integrated: true,
                levels: game.level_shapes,
            };
            let settings = LanSettings {
                port: 0,
                guest_commands: false,
                force_game_mode: false,
            };
            game.lan_host(settings, net, config).unwrap();
            let mut guest_at = |conn: u32, at: [f64; 3]| {
                let (tx, rx) = std::sync::mpsc::channel();
                let mut guest = Guest::new(
                    Joined {
                        conn,
                        uuid: u128::from(conn),
                        name: format!("guest{conn}"),
                        properties: Vec::new(),
                        info: ClientInformation::new(8),
                        brand: String::new(),
                        sender: PlayerSender::new(tx, Arc::new(AtomicUsize::new(0))),
                        latency: Arc::new(AtomicI32::new(0)),
                        address: "127.0.0.1:1".parse().unwrap(),
                        expected: false,
                    },
                    conn as i32,
                    at,
                    10,
                );
                guest.placed = true;
                guest.dimension = game.dim.id().to_owned();
                game.lan.as_mut().unwrap().hub.guests.push(guest);
                rx
            };
            let near = guest_at(12, [30.5, 64.0, -3.5]);
            let far = guest_at(13, [110.5, 64.0, -3.5]);
            let events = |rx: &std::sync::mpsc::Receiver<Out>| {
                rx.try_iter()
                    .flat_map(|out| match out {
                        Out::Packet(body) => vec![body],
                        Out::Packets(bodies) => bodies,
                        Out::Disconnect(_) => panic!("unexpected disconnect"),
                    })
                    .filter_map(|body| {
                        let mut reader = horizon_protocol::buf::Reader::new(&body);
                        let id = reader.varint().unwrap();
                        match Clientbound::decode(id, reader.rest()) {
                            Ok(Clientbound::LevelEvent(p)) => Some(p),
                            _ => None,
                        }
                    })
                    .collect::<Vec<_>>()
            };
            game.lan_flush_effects();
            events(&near);
            events(&far);
            game.audio.capture = Some(Vec::new());
            assert!(game.ui.rules.set("global_sound_events", Value::Bool(false)));
            let level = game.level(game.dim.id()).unwrap();
            assert!(level.global_level_event(game, 1038, [10, 64, -3], 0));
            game.lan_flush_effects();
            assert_eq!(
                events(&near),
                [LevelEvent {
                    kind: 1038,
                    pos: [10, 64, -3],
                    data: 0,
                    global: false,
                }]
            );
            assert!(events(&far).is_empty(), "100 blocks away is out of range");
        });
    }
}
