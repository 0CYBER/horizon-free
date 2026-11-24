//! An entity's sounds, game events, entity events and spectator state for
//! native server mods, through an `EntityRef` (docs/modding/src/how-to/native-mods.md, "Entity
//! and level handles"): `Entity.playSound`, `gameEvent`,
//! `Level.broadcastEntityEvent`, `getSoundSource` and `isSpectator`. They
//! return None or false for a stale handle, on a client, and for an entity
//! that is not in the level in place (a player in a parked level).
use super::{Game, commands::Target};
use horizon_client::mobs::Species;
use mod_api::{EntityRef, SoundSource};

/// `Monster`'s sound source is hostile, and so are a few mobs that are not
/// `Monster`s: `Hoglin`, `Ghast`, `Phantom`, `Shulker` and the slimes
/// (`AbstractCubeMob`).
pub(super) fn hostile_species(species: Species) -> bool {
    horizon_client::mobs::ai::hostile_species(species)
}

impl Game<'_> {
    /// The level in place holds the entity: events and game events are
    /// queued in it.
    pub(super) fn native_entity_here(&self, entity: EntityRef) -> bool {
        self.entity_level(entity)
            .is_some_and(|level| level.dimension() == self.native_level_key())
    }
    /// `Entity.getSoundSource`: the group an entity's sounds play under.
    /// Players are `Players`; `Monster`s, the dragon, hoglins, ghasts,
    /// phantoms, shulkers, slimes and a killer bunny are `Hostile`; dropped
    /// items and experience orbs `Ambient`; every other entity `Neutral`.
    pub(super) fn entity_sound_source(&self, entity: EntityRef) -> Option<SoundSource> {
        Some(match self.native_entity_target(entity)? {
            Target::Player | Target::Guest(_) => SoundSource::Players,
            Target::Mob(id) => {
                let m = self.mobs.iter().find(|m| m.id == id)?;
                let evil = m.species() == Species::Rabbit
                    && m.extra.variant == horizon_client::mobs::rabbit::EVIL;
                if evil || hostile_species(m.species()) {
                    SoundSource::Hostile
                } else {
                    SoundSource::Neutral
                }
            }
            Target::Item(_) | Target::Orb(_) => SoundSource::Ambient,
            Target::Dragon(_) => SoundSource::Hostile,
            t => match self.target_type(t).as_str() {
                "minecraft:shulker_bullet" => SoundSource::Hostile,
                "minecraft:lightning_bolt" => SoundSource::Weather,
                _ => SoundSource::Neutral,
            },
        })
    }
    /// `Entity.playSound(sound, volume, pitch)`: plays sound event `sound` at
    /// the entity under its sound source, for the players within range. A
    /// silent mob plays nothing, as vanilla's check is `isSilent`; a player
    /// (`Player.playSound`) doesn't hear it themself, their own client plays
    /// it. True means queued, not heard; false for an unknown `minecraft:`
    /// event or a volume or pitch that isn't a finite non-negative number.
    pub(super) fn entity_play_sound(
        &mut self,
        entity: EntityRef,
        sound: &str,
        volume: f32,
        pitch: f32,
    ) -> bool {
        let Some(target) = self.native_entity_target(entity) else {
            return false;
        };
        let (Some(level), Some(at), Some(source)) = (
            self.entity_level(entity),
            self.target_position(target),
            self.entity_sound_source(entity),
        ) else {
            return false;
        };
        let except = match target {
            Target::Player => self.lan_acting,
            Target::Guest(conn) => Some(conn),
            _ => {
                if self.entity_is_silent(entity) == Some(true) {
                    return false;
                }
                None
            }
        };
        self.native_queue_sound(level, sound, at, source, volume, pitch, except)
            .is_some()
    }
    /// `Entity.gameEvent(event)`: game event `event` (`minecraft:step`...)
    /// at the entity's position with the entity as its source, which sculk
    /// sensors, shriekers, wardens and allays in range hear. A player or mob
    /// is the event's source entity; other entities retain their own id.
    /// False for an event vanilla doesn't have, a dead guest and a guest while
    /// another one acts.
    pub(super) fn entity_game_event(&mut self, entity: EntityRef, event: &str) -> bool {
        let Some(target) = self.native_entity_target(entity) else {
            return false;
        };
        let Some(at) = self.target_position(target) else {
            return false;
        };
        let Some(level) = self.level(self.dim.id()) else {
            return false;
        };
        self.level_game_event(
            level,
            event,
            at,
            mod_api::GameEventContext {
                source_entity: Some(entity),
                affected_state: None,
            },
        )
    }
    /// `Level.broadcastEntityEvent(entity, id)`: sends entity event `id` (the
    /// byte `Entity.handleEntityEvent` reads: `2` is a hurt flash, `3` a
    /// death, `35` a totem...) to the players tracking the entity and to the
    /// entity itself. True means queued; false for an entity outside the
    /// level in place.
    pub(super) fn entity_broadcast_event(&mut self, entity: EntityRef, id: u8) -> bool {
        let Some(target) = self.native_entity_target(entity) else {
            return false;
        };
        let Some(at) = self.target_position(target) else {
            return false;
        };
        if !self.native_entity_here(entity) {
            return false;
        }
        let Ok(entity_id) = i32::try_from(entity.id()) else {
            return false;
        };
        self.lan_entity_event(entity_id, id as i8, at);
        true
    }
    /// `Entity.isSpectator`: true for a player in spectator mode, false for
    /// every other entity.
    pub(super) fn entity_spectator(&self, entity: EntityRef) -> Option<bool> {
        Some(match self.native_entity_target(entity)? {
            Target::Player => self.spectator(),
            Target::Guest(conn) => self.command_guest(conn)?.game_mode == 3,
            _ => false,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::dispense_fixture;
    use super::super::native_entity_refs::tests::with_game;
    use horizon_client::command::args::GameMode;
    use horizon_client::mobs::Mob;
    use horizon_client::motion::PlayerMotion;
    use horizon_core::geometry::Vec3;
    use mod_api::{EntityRef, SoundSource, WorldEpoch};

    fn spawn(game: &mut super::Game<'_>, id: u32, kind: &str) -> EntityRef {
        let kind = game.res.mob_types().get(kind).unwrap().clone();
        game.mobs
            .push(Mob::new(id, kind, Vec3::new(1.5, 64.0, 2.5), 0.0));
        game.entity(id).unwrap()
    }

    fn acting_player(game: &mut super::Game<'_>) {
        game.player = Some(PlayerMotion::new(game.motion, Vec3::ZERO).unwrap());
        crate::game::native_entity_refs::tests::bind_local_player(game, 7, 7);
    }

    #[test]
    #[ignore = "daily"]
    fn sound_sources_follow_the_entity_class() {
        with_game(|game| {
            let cow = spawn(game, 10, "minecraft:cow");
            let zombie = spawn(game, 11, "minecraft:zombie");
            let ghast = spawn(game, 12, "minecraft:ghast");
            let slime = spawn(game, 13, "minecraft:slime");
            let sulfur = spawn(game, 14, "minecraft:sulfur_cube");
            let bunny = spawn(game, 15, "minecraft:rabbit");
            let source = |game: &super::Game<'_>, e| game.entity_sound_source(e);
            assert_eq!(source(game, cow), Some(SoundSource::Neutral));
            assert_eq!(source(game, zombie), Some(SoundSource::Hostile));
            assert_eq!(source(game, ghast), Some(SoundSource::Hostile));
            assert_eq!(source(game, slime), Some(SoundSource::Hostile));
            assert_eq!(source(game, sulfur), Some(SoundSource::Neutral));
            assert_eq!(source(game, bunny), Some(SoundSource::Neutral));
            game.mobs
                .iter_mut()
                .find(|m| m.id == 15)
                .unwrap()
                .extra
                .variant = horizon_client::mobs::rabbit::EVIL;
            assert_eq!(source(game, bunny), Some(SoundSource::Hostile));
            acting_player(game);
            let player = game.entity(7).unwrap();
            assert_eq!(source(game, player), Some(SoundSource::Players));
            let stale = EntityRef::new(WorldEpoch::new(cow.epoch().get() + 1), 10);
            assert_eq!(source(game, stale), None);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn play_sound_queues_at_the_entity_in_its_source() {
        super::super::native_guest_turn_tests::with_guests(|game| {
            let cow = spawn(game, 10, "minecraft:cow");
            assert!(game.entity_play_sound(cow, "minecraft:entity.cow.ambient", 0.5, 1.5));
            let sound = &game.creatures.lan_sounds[0];
            assert_eq!(sound.sound, "minecraft:entity.cow.ambient");
            assert_eq!(sound.at, [1.5, 64.0, 2.5]);
            assert_eq!(sound.source as usize, SoundSource::Neutral.index());
            assert_eq!((sound.volume, sound.pitch, sound.except), (0.5, 1.5, None));
            // A silent mob plays nothing; a bad pitch is refused.
            game.mobs[0].silent = true;
            assert!(!game.entity_play_sound(cow, "minecraft:entity.cow.ambient", 1.0, 1.0));
            game.mobs[0].silent = false;
            assert!(!game.entity_play_sound(cow, "minecraft:entity.cow.ambient", 1.0, f32::NAN));
            assert_eq!(game.creatures.lan_sounds.len(), 1);
            // A player doesn't hear their own sound, silent or not.
            game.as_guest(12, |game| {
                let player = game.entity(12).unwrap();
                assert!(game.entity_play_sound(player, "minecraft:entity.player.burp", 1.0, 1.0));
                let sound = &game.creatures.lan_sounds[1];
                assert_eq!(sound.source as usize, SoundSource::Players.index());
                assert_eq!(sound.except, Some(12));
            })
            .expect("the sound's real guest turn runs");
            let stale = EntityRef::new(WorldEpoch::new(cow.epoch().get() + 1), 10);
            assert!(!game.entity_play_sound(stale, "minecraft:entity.cow.ambient", 1.0, 1.0));
            assert_eq!(game.creatures.lan_sounds.len(), 2);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn game_and_entity_events_come_from_the_entity() {
        with_game(|game| {
            let cow = spawn(game, 10, "minecraft:cow");
            dispense_fixture::start_recording();
            assert!(game.entity_game_event(cow, "minecraft:step"));
            assert!(!game.entity_game_event(cow, "minecraft:no_such_event"));
            assert_eq!(
                dispense_fixture::stop_recording(),
                ["G 1,64,2 minecraft:step"]
            );
            game.audio.capture = Some(Vec::new());
            assert!(game.entity_broadcast_event(cow, 3));
            assert_eq!(game.lan_entity_events, [(10, 3)]);
            let stale = EntityRef::new(WorldEpoch::new(cow.epoch().get() + 1), 10);
            assert!(!game.entity_broadcast_event(stale, 3));
            assert!(!game.entity_game_event(stale, "minecraft:step"));
            assert_eq!(game.lan_entity_events.len(), 1);
        });
    }

    #[test]
    #[ignore = "daily"]
    fn only_a_spectator_player_is_a_spectator() {
        with_game(|game| {
            let cow = spawn(game, 10, "minecraft:cow");
            assert_eq!(game.entity_spectator(cow), Some(false));
            acting_player(game);
            let player = game.entity(7).unwrap();
            assert_eq!(game.entity_spectator(player), Some(false));
            game.mode = GameMode::Spectator;
            assert_eq!(game.entity_spectator(player), Some(true));
            assert_eq!(game.entity_spectator(cow), Some(false));
        });
    }
}
