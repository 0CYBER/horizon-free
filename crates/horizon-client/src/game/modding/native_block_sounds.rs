// game-module-visibility: pub(crate)
//! The `sound_type` Part method (`IBlockExtension.getSoundType`). The client library asks through
//! `native_client_hooks`; only a state whose block has a subscribing part is converted, and only
//! then is its own sound type kept to fall back on.
use super::api_values::{TryIntoApi, block_state};
use super::{Game, GameLevel};
use horizon_core::{identifier::Identifier, position::BlockPos};
use horizon_world::{
    level::data::Sounds, native_load_context::LoopContext, registry::BlockStateId,
};
use mod_api::{BlockSoundCause, BlockSoundType};
use std::sync::Arc;

/// The game's side of `horizon_client::block_sounds::native_sound_type`.
pub(crate) fn sound_type(
    context: &LoopContext,
    pos: BlockPos,
    state: BlockStateId,
    entity: Option<(u32, &str)>,
    cause: BlockSoundCause,
    sound: &mut BlockSoundType,
) {
    if !block_state::block_listens(mod_api::HookId::SoundType, state) {
        return;
    }
    let Ok(api_state) = state.try_into_api() else {
        return;
    };
    let own = sound.clone();
    mod_api::dispatch_sound_type(
        &horizon_client_render::api_context::LoopReader(context).context(),
        pos,
        api_state,
        entity,
        cause,
        sound,
    );
    if !playable(sound) {
        *sound = own;
    }
}

/// Whether a part's answer can be played: a finite volume and pitch of at least 0, and every
/// event a `namespace:path` id (a mod's own included; one the resources lack plays nothing).
fn playable(sound: &BlockSoundType) -> bool {
    let level = |value: f32| value.is_finite() && value >= 0.0;
    let events = [
        &sound.break_sound,
        &sound.step,
        &sound.place,
        &sound.hit,
        &sound.fall,
    ];
    level(sound.volume)
        && level(sound.pitch)
        && events
            .iter()
            .all(|event| Identifier::parse_namespaced(event).is_some())
}

impl Game<'_> {
    /// The sound type of `state` at `pos` for `cause` (`IBlockExtension.getSoundType`), as the
    /// block's parts answer on this side; `entity` is the id and type of the entity stepping,
    /// landing, placing or digging, and None for a break or a placement by no entity.
    pub(in crate::game) fn block_sound_type(
        &self,
        [x, y, z]: [i32; 3],
        state: BlockStateId,
        entity: Option<(u32, &str)>,
        cause: BlockSoundCause,
    ) -> BlockSoundType {
        horizon_client::block_sounds::sound_type(
            &self.loop_context,
            self.res.facts(),
            BlockPos::new(x, y, z),
            state,
            entity,
            cause,
        )
    }
    /// [`Game::block_sound_type`] for this window's own player, as its client asks for the
    /// sounds it plays itself: its steps and the blocks it places.
    pub(in crate::game) fn own_sound_type(
        &self,
        pos: [i32; 3],
        state: BlockStateId,
        cause: BlockSoundCause,
    ) -> BlockSoundType {
        let player = (self.own_entity_id() as u32, "minecraft:player");
        self.block_sound_type(pos, state, Some(player), cause)
    }
    /// `LevelEventHandler`'s break sound (level event 2001, which a client's predicted break
    /// also plays): the block's break sound at its centre, at `(volume + 1) / 2` and pitch × 0.8.
    /// None for air.
    pub(in crate::game) fn play_break_sound(&mut self, pos: [i32; 3], state: BlockStateId) {
        if self.res.facts().is_air(state) {
            return;
        }
        let sound = self.block_sound_type(pos, state, None, BlockSoundCause::Break);
        self.audio.play(
            &sound.break_sound,
            Some(pos.map(|v| f64::from(v) + 0.5)),
            (sound.volume + 1.0) / 2.0,
            sound.pitch * 0.8,
        );
    }
}

impl GameLevel<'_> {
    /// `LevelAccess::place_sound_type`: the state's own `sounds` unless a part on its block
    /// answers another, asked with the acting player, or none when no player places it (a
    /// dispenser).
    pub(super) fn part_place_sound_type(
        &self,
        pos: BlockPos,
        state: BlockStateId,
        sounds: Arc<Sounds>,
    ) -> Arc<Sounds> {
        if !block_state::block_listens(mod_api::HookId::SoundType, state) {
            return sounds;
        }
        let entity = self
            .native_player_entity
            .map(|id| (id as u32, "minecraft:player"));
        let own = horizon_client::block_sounds::own(&self.facts.get(state).sounds);
        let mut answer = own.clone();
        horizon_client::block_sounds::native_sound_type(
            self.loop_context,
            pos,
            state,
            entity,
            BlockSoundCause::Place,
            &mut answer,
        );
        if answer == own {
            return sounds;
        }
        Arc::new(Sounds {
            volume: answer.volume,
            pitch: answer.pitch,
            place: answer.place,
            break_: answer.break_sound,
            fall: answer.fall,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_game;
    use super::playable;
    use horizon_client::mobs::{Mob, MobEvent, equine};
    use horizon_core::{geometry::Vec3, position::BlockPos};
    use horizon_world::level::LevelAccess;
    use mod_api::BlockSoundCause::{Break, Dig, Fall, Place, Step};
    use std::sync::Arc;

    #[test]
    fn sound_answers_validate_levels_and_names_without_requiring_loaded_events() {
        let stone = horizon_client::blocks::Sounds {
            volume: 1.0,
            pitch: 1.0,
            break_sound: "minecraft:block.stone.break".into(),
            step: "minecraft:block.stone.step".into(),
            place: "minecraft:block.stone.place".into(),
            hit: "minecraft:block.stone.hit".into(),
            fall: "minecraft:block.stone.fall".into(),
        };
        for value in [-1.0, f32::INFINITY, f32::NEG_INFINITY, f32::NAN] {
            let mut sound = horizon_client::block_sounds::own(&stone);
            sound.volume = value;
            assert!(!playable(&sound));
            sound.volume = 1.0;
            sound.pitch = value;
            assert!(!playable(&sound));
        }
        for value in [0.0, 1.0, 2.0, f32::MAX] {
            let mut sound = horizon_client::block_sounds::own(&stone);
            sound.volume = value;
            sound.pitch = value;
            assert!(playable(&sound));
        }
        for event in ["", "not_namespaced", "mod:bad path"] {
            let mut sound = horizon_client::block_sounds::own(&stone);
            sound.step = event.into();
            assert!(!playable(&sound));
        }
        let mut sound = horizon_client::block_sounds::own(&stone);
        sound.step = "unknown:unloaded_event".into();
        assert!(playable(&sound));
    }

    #[test]
    #[ignore = "daily"]
    fn a_block_part_answers_the_sound_type_a_block_plays() {
        // The fixture's parts: glass answers `mymod:block.x.*` at volume 2.0 and pitch 1.5 to
        // every cause, white wool the same to a player only, red and blue wool a NaN volume and
        // an event with no namespace, and a snow layer as glass but only when asked at y 64.
        mod_platform::__private::part_tests::sound_type_game_thread();
        // The client library asks the mods through the executable's callbacks.
        crate::entry::install_client_hooks();
        with_game(|game| {
            let registry = game.pack.assets.registry;
            let player = Some((7, "minecraft:player"));
            let creeper = Some((8, "minecraft:creeper"));
            let sniffer = Some((9, "minecraft:sniffer"));
            let sound = |game: &crate::game::Game<'_>, block: &str, entity, cause| {
                let state = registry.default_state(block).unwrap();
                let sound = game.block_sound_type([5, 64, 5], state, entity, cause);
                let event = match cause {
                    Step => sound.step,
                    Fall => sound.fall,
                    Place => sound.place,
                    Break => sound.break_sound,
                    _ => sound.hit,
                };
                (event.to_string(), sound.volume, sound.pitch)
            };
            let stone = |event: &str| (format!("minecraft:block.stone.{event}"), 1.0, 1.0);
            let wool = |event: &str| (format!("minecraft:block.wool.{event}"), 1.0, 1.0);
            let answered = |event: &str| (format!("mymod:block.x.{event}"), 2.0, 1.5);
            // No part: the state's own.
            assert_eq!(sound(game, "minecraft:stone", creeper, Step), stone("step"));
            // A part answering every cause and entity, with the mod's own events.
            assert_eq!(
                sound(game, "minecraft:glass", creeper, Step),
                answered("step")
            );
            assert_eq!(
                sound(game, "minecraft:glass", creeper, Fall),
                answered("fall")
            );
            assert_eq!(
                sound(game, "minecraft:glass", player, Place),
                answered("place")
            );
            assert_eq!(
                sound(game, "minecraft:glass", None, Break),
                answered("break")
            );
            assert_eq!(
                sound(game, "minecraft:glass", sniffer, Dig),
                answered("hit")
            );
            // A part answering for a player only.
            assert_eq!(
                sound(game, "minecraft:white_wool", player, Step),
                answered("step")
            );
            assert_eq!(
                sound(game, "minecraft:white_wool", creeper, Step),
                wool("step")
            );
            assert_eq!(
                sound(game, "minecraft:white_wool", None, Break),
                wool("break")
            );
            // Answers that can't be played are dropped whole for the state's own.
            assert_eq!(
                sound(game, "minecraft:red_wool", creeper, Step),
                wool("step")
            );
            assert_eq!(
                sound(game, "minecraft:blue_wool", creeper, Step),
                wool("step")
            );
            // The server's placement (`BlockItem.place`) asks with the acting player, and with
            // none when nothing places it (a dispenser).
            let glass = registry.default_state("minecraft:glass").unwrap();
            let white_wool = registry.default_state("minecraft:white_wool").unwrap();
            let kind = game
                .res
                .mob_types()
                .get("minecraft:creeper")
                .unwrap()
                .clone();
            let creeper = Mob::new(8, kind, Vec3::ZERO, 0.0);
            let step = horizon_client::mobs::entity_sounds::step(
                &creeper,
                &game.mob_world_base(&[]),
                [5, 64, 5],
                glass,
            )
            .unwrap();
            assert!(matches!(&step.0, std::borrow::Cow::Owned(_)));
            assert_eq!(step.0, "mymod:block.x.step");
            assert_eq!(step.1, 0.3);
            assert_eq!(step.2, 1.5);

            let mut placed = Vec::new();
            for acting in [Some(7), None] {
                game.lan_acting_entity = acting;
                placed.extend(game.with_level(|behaviours, level, _| {
                    // None when the level's own sound type is kept.
                    let place = |state| {
                        let own = Arc::clone(&behaviours.data(state).sounds);
                        let pos = BlockPos::new(5, 64, 5);
                        let sounds = level.place_sound_type(pos, state, Arc::clone(&own));
                        (!Arc::ptr_eq(&sounds, &own))
                            .then(|| (sounds.place.to_string(), sounds.volume, sounds.pitch))
                    };
                    [place(glass), place(white_wool)]
                }));
            }
            let placement = Some(answered("place"));
            assert_eq!(
                placed,
                [placement.clone(), placement.clone(), placement, None]
            );
            // A horse stepping on stone under a snow layer plays the snow's answer, asked at the
            // stone's position (`AbstractHorse.playStepSound`): volume 2.0 × 0.15, pitch 1.5.
            let stone = registry.default_state("minecraft:stone").unwrap();
            let snow = registry.default_state("minecraft:snow").unwrap();
            game.world
                .chunks
                .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
            game.world.set(5, 64, 5, stone).unwrap();
            game.world.set(5, 65, 5, snow).unwrap();
            let kind = game.res.mob_types().get("minecraft:horse").unwrap().clone();
            let mut horse = Mob::new(12, kind, Vec3::ZERO, 0.0);
            let mut events = Vec::new();
            equine::play_step(
                &mut horse,
                &game.mob_world_base(&[]),
                [5, 64, 5],
                stone,
                &mut events,
            );
            assert!(
                matches!(
                    events.as_slice(),
                    [MobEvent::Sound("entity.horse.step", _, volume, pitch)]
                        if *volume == 2.0 * 0.15 && *pitch == 1.5
                ),
                "{events:?}"
            );
        });
    }
}
