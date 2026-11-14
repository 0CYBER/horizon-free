//! Sounds to one player and sounds that follow an entity, for native server
//! mods, through a `PlayerRef` and an `EntityRef` (docs/modding/src/how-to/native-mods.md):
//! `StopSoundCommand`'s packet, a `ClientboundSoundPacket` for one player
//! (`PlaySoundCommand`) and `Level.playSound(null, entity, ...)`, which
//! sends a `ClientboundSoundEntityPacket` to the players in range. They
//! return false for a stale handle, a player who isn't online here, an
//! unknown `minecraft:` event, and a volume or pitch that isn't a finite
//! non-negative number.
use super::{
    Game,
    commands::Target,
    native_hooks::{mod_sound_ok, sound_id},
};
use horizon_core::random::RandomSource;
use mod_api::{EntityRef, PlayerRef, SoundSource};

impl Game<'_> {
    /// Where `player` is reached now: the window's own player or a guest.
    fn native_player_sound_target(&self, player: PlayerRef) -> Option<Target> {
        if self.player(player.uuid()) != Some(player) {
            return None;
        }
        match self.native_entity_target(player.entity())? {
            t @ (Target::Player | Target::Guest(_)) => Some(t),
            _ => None,
        }
    }
    /// Whether the server's registry has `sound`, when it is a vanilla
    /// event; a mod's goes by name.
    pub(super) fn native_sound_known(&self, sound: &str) -> bool {
        !sound.starts_with("minecraft:")
            || self.lan.as_ref().is_none_or(|host| {
                host.hub
                    .registry_id("minecraft:sound_event", sound)
                    .is_some()
            })
    }
    /// `StopSoundCommand` for one player: their client stops the playing
    /// sounds that match `sound` (an event id) and `source`; `None` matches
    /// any. True means sent.
    pub(super) fn player_stop_sound(
        &mut self,
        player: PlayerRef,
        sound: Option<&str>,
        source: Option<SoundSource>,
    ) -> bool {
        let Some(target) = self.native_player_sound_target(player) else {
            return false;
        };
        if sound.is_some_and(|s| !sound_id(s)) {
            return false;
        }
        self.stop_sound_for(&[target], sound, source.map(SoundSource::index));
        true
    }
    /// `PlaySoundCommand` for one player: plays sound event `sound` at the
    /// player, for them only, under `source`'s volume slider. The seed is the
    /// level random's `nextLong`, drawn once per call. True means sent, not
    /// heard.
    pub(super) fn player_play_sound(
        &mut self,
        player: PlayerRef,
        sound: &str,
        source: SoundSource,
        volume: f32,
        pitch: f32,
    ) -> bool {
        let Some(target) = self.native_player_sound_target(player) else {
            return false;
        };
        let Some(at) = self.target_position(target) else {
            return false;
        };
        if !mod_sound_ok(sound, at, source.index(), volume, pitch)
            || !self.native_sound_known(sound)
        {
            return false;
        }
        let seed = self.world.random.next_long();
        self.play_sound_to(target, sound, at, volume, pitch, source.index(), seed);
        true
    }
    /// `Level.playSound(null, entity, sound, source, volume, pitch)`: plays
    /// sound event `sound` bound to the entity, so that it follows it, for
    /// the players within range (16 blocks, times the volume above 1) of
    /// where it is now, the entity's own player too. True means queued, not
    /// heard; false for an entity outside the level in place.
    pub(super) fn entity_play_bound_sound(
        &mut self,
        entity: EntityRef,
        sound: &str,
        source: SoundSource,
        volume: f32,
        pitch: f32,
    ) -> bool {
        let Some(target) = self.native_entity_target(entity) else {
            return false;
        };
        let (Some(level), Some(at), Ok(id)) = (
            self.entity_level(entity),
            self.target_position(target),
            i32::try_from(entity.id()),
        ) else {
            return false;
        };
        self.native_queue_sound(level, sound, at, source, volume, pitch, None)
            .map(|queued| queued.entity = Some(id))
            .is_some()
    }
}

#[cfg(test)]
pub(super) mod tests {
    use super::super::lan::LanSettings;
    use super::super::native_entity_refs::tests::with_game;
    use horizon_client::lan::{Joined, NetConfig, Out, PlayerSender, guest::Guest, hub::HubConfig};
    use horizon_client::mobs::Mob;
    use horizon_core::geometry::Vec3;
    use horizon_protocol::packets::{configuration::ClientInformation, play::Clientbound};
    use mod_api::__private::game::*;
    use mod_api::{SoundSource, WorldEpoch};
    use std::sync::{
        Arc,
        atomic::{AtomicI32, AtomicUsize},
        mpsc::Receiver,
    };

    pub(in crate::game) fn guest(conn: u32, uuid: u128, at: [f64; 3]) -> (Guest, Receiver<Out>) {
        let (tx, rx) = std::sync::mpsc::channel();
        let mut g = Guest::new(
            Joined {
                conn,
                uuid,
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
        g.placed = true;
        (g, rx)
    }

    pub(in crate::game) fn packets(rx: &Receiver<Out>) -> Vec<Clientbound> {
        let mut out = Vec::new();
        while let Ok(Out::Packet(body)) = rx.try_recv() {
            let mut reader = horizon_protocol::buf::Reader::new(&body);
            let id = reader.varint().unwrap();
            if let Ok(p) = Clientbound::decode(id, reader.rest()) {
                out.push(p);
            }
        }
        out
    }

    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn sounds_reach_the_right_connections() {
        with_game(|game| {
            let net = NetConfig {
                listen: false,
                announce: false,
                lod_distance: 0,
                ..NetConfig::lan(0, String::new(), 3)
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
                max_players: 3,
                integrated: true,
                levels: game.level_shapes,
            };
            let settings = LanSettings {
                port: 0,
                guest_commands: false,
                force_game_mode: false,
            };
            game.lan_host(settings, net, config).unwrap();
            let (a, rx_a) = guest(21, 21, [1.0, 64.0, 2.0]);
            let (b, rx_b) = guest(22, 22, [3.0, 64.0, 2.0]);
            let (far, rx_far) = guest(23, 23, [900.0, 64.0, 2.0]);
            game.lan.as_mut().unwrap().hub.guests.extend([a, b, far]);
            let pa = game.player(21).unwrap();
            let pb = game.player(22).unwrap();
            let sound = "minecraft:entity.player.burp";

            // stop_sound: only the named player's connection.
            assert!(game.player_stop_sound(pa, Some(sound), Some(SoundSource::Players)));
            assert!(game.player_stop_sound(pa, None, None));
            assert!(!game.player_stop_sound(pa, Some("bad id"), None));
            let got = packets(&rx_a);
            assert_eq!(got.len(), 2);
            let Clientbound::StopSound(first) = &got[0] else {
                panic!("expected stop sound")
            };
            assert_eq!(first.sound.as_deref(), Some(sound));
            assert_eq!(first.source, Some(SoundSource::Players.index() as i32));
            let Clientbound::StopSound(second) = &got[1] else {
                panic!("expected stop sound")
            };
            assert_eq!((second.sound.clone(), second.source), (None, None));
            assert!(packets(&rx_b).is_empty());

            // play_sound: one packet for one player, at their position.
            assert!(game.player_play_sound(pb, sound, SoundSource::Neutral, 0.5, 1.5));
            assert!(!game.player_play_sound(pb, sound, SoundSource::Neutral, 1.0, f32::NAN));
            assert!(!game.player_play_sound(
                pb,
                "minecraft:no.such.sound",
                SoundSource::Neutral,
                1.0,
                1.0
            ));
            assert!(packets(&rx_a).is_empty());
            let got = packets(&rx_b);
            assert_eq!(got.len(), 1);
            let Clientbound::Sound(p) = &got[0] else {
                panic!("expected sound")
            };
            assert_eq!((p.x, p.y, p.z), (24, 512, 16));
            assert_eq!((p.volume, p.pitch), (0.5, 1.5));
            assert_eq!(p.source, SoundSource::Neutral.index() as i32);

            // A stale handle and an offline player fail.
            let stale = mod_api::__private::game::player_ref(
                mod_api::EntityRef::new(
                    WorldEpoch::new(pa.entity().epoch().get() + 1),
                    pa.entity().id(),
                ),
                pa.fields().uuid,
            );
            assert!(!game.player_stop_sound(stale, None, None));
            assert!(!game.player_play_sound(stale, sound, SoundSource::Players, 1.0, 1.0));
            assert!(game.player(99).is_none());

            // play_bound_sound: players in range get a SoundEntity packet.
            let kind = game.res.mob_types().get("minecraft:cow").unwrap().clone();
            game.mobs
                .push(Mob::new(40, kind, Vec3::new(2.0, 64.0, 2.0), 0.0));
            let cow = game.entity(40).unwrap();
            assert!(game.entity_play_bound_sound(cow, sound, SoundSource::Neutral, 1.0, 1.0));
            assert!(!game.entity_play_bound_sound(cow, sound, SoundSource::Neutral, -1.0, 1.0));
            game.flush_native_sounds();
            for rx in [&rx_a, &rx_b] {
                let got = packets(rx);
                assert_eq!(got.len(), 1);
                let Clientbound::SoundEntity(p) = &got[0] else {
                    panic!("expected sound entity")
                };
                assert_eq!(p.id, 40);
                assert_eq!(p.source, SoundSource::Neutral.index() as i32);
            }
            assert!(packets(&rx_far).is_empty());
        });
    }

    /// `Level.playSound` draws its seed at the call, from the level's own
    /// sound-seed generator (`Level.soundSeedGenerator`), not the gameplay
    /// random; and a player exactly at the range edge is outside
    /// `PlayerList.broadcast`.
    #[test]
    #[ignore = "needs prepared resources in HORIZON_TEST_RESOURCES"]
    fn queued_sounds_draw_sound_seeds_at_play_and_cut_at_the_range_edge() {
        use horizon_core::random::RandomSource;

        with_game(|game| {
            let net = NetConfig {
                listen: false,
                announce: false,
                lod_distance: 0,
                ..NetConfig::lan(0, String::new(), 3)
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
                max_players: 3,
                integrated: true,
                levels: game.level_shapes,
            };
            let settings = LanSettings {
                port: 0,
                guest_commands: false,
                force_game_mode: false,
            };
            game.lan_host(settings, net, config).unwrap();
            let (near, rx_near) = guest(31, 31, [0.5, 64.0, 1.0]);
            let (edge, rx_edge) = guest(32, 32, [0.5, 64.0, 16.0]);
            game.lan.as_mut().unwrap().hub.guests.extend([near, edge]);

            let level = game.level("minecraft:overworld").unwrap();
            let sound = "minecraft:entity.player.burp";
            // The sound source's draws bookend the queue's: each queued sound
            // takes one of its own, distinct from every neighbour's.
            let before = game.lan.as_ref().unwrap().hub.next_sound_seed();
            let mut level_random = game.world.random.clone();
            for _ in 0..2 {
                assert!(
                    game.native_queue_sound(
                        level,
                        sound,
                        [0.5, 64.0, 0.0],
                        SoundSource::Neutral,
                        1.0,
                        1.0,
                        None,
                    )
                    .is_some()
                );
            }
            // The gameplay random is exactly where it was: a mod's sound
            // moves no level draw.
            assert_eq!(
                game.world.random.next_long(),
                level_random.next_long(),
                "a queued sound draws no level-random long"
            );
            game.flush_native_sounds();

            let got = packets(&rx_near);
            assert_eq!(got.len(), 2, "both queued sounds reach the near player");
            let seeds: Vec<i64> = got
                .iter()
                .map(|packet| {
                    let Clientbound::Sound(p) = packet else {
                        panic!("expected sound")
                    };
                    p.seed
                })
                .collect();
            let after = game.lan.as_ref().unwrap().hub.next_sound_seed();
            assert_ne!(seeds[0], seeds[1], "each play draws its own seed");
            assert!(
                seeds.iter().all(|seed| *seed != before && *seed != after),
                "the queued sounds take the sound source's next draws"
            );
            assert!(
                packets(&rx_edge).is_empty(),
                "a player exactly at the range edge hears nothing"
            );
        });
    }
}
