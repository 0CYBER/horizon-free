//! The `level_sounds` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A sound is about to play at `position` in `level` (NeoForge's
    /// `PlayLevelSoundEvent.AtPosition`, in `Level.playSeededSound`):
    /// every server sound not bound to an entity, mobs' own sounds included
    /// (`Entity.playSound` plays at the entity's position). A mod may change
    /// `sound`, `source`, `volume` and `pitch`, or set `cancel` and no player
    /// hears it; `original_volume` and `original_pitch` are as asked. A sound
    /// left unplayable (a volume or pitch that isn't finite and at least 0)
    /// is dropped. Sounds the server's level plays during
    /// the tick are asked as they go out, at the end of the tick, with the state
    /// then. `/playsound` sends its packet directly and isn't asked, as in
    /// vanilla on the server. On the client, incoming sound packets ask before
    /// playback, including `/playsound`; client-predicted and game-event level
    /// requests ask here too. Canceling a sound does not cancel its action or prediction.
    /// Server and client requests are separate:
    /// `HostSide` identifies which one is being edited. UI and sound-engine-only
    /// playback do not ask this hook. Both sides.
    #[hook(
        side = "both",
        contract = "cancel",
        out = "cancel",
        neoforge = "PlayLevelSoundEvent.AtPosition",
        cancellable = true
    )]
    fn play_level_sound_at_position(
        level: LevelRef,
        position: [f64; 3],
        sound: &mut Identifier,
        source: &mut SoundSource,
        volume: &mut f32,
        pitch: &mut f32,
        original_volume: f32,
        original_pitch: f32,
        cancel: &mut bool,
    );
    /// A sound bound to `entity` is about to play (NeoForge's
    /// `PlayLevelSoundEvent.AtEntity`, in `Level.playSeededSound`'s entity
    /// overload, `playSound(except, sourceEntity, ...)`): a goat horn, a frog's
    /// tongue, eating and spawn, an animal eating (frog, goat, sniffer), a long
    /// jump, a goat's ram, a breeze's charge and deflection, shearing (sheep,
    /// mooshroom, snow golem, bogged, copper golem), dye on a sheep, an axe on a
    /// copper golem, an allay's item and duplication, a ghastling hatching, a
    /// trident thrown (the trident) or riptide (the player), and a mod's sound
    /// bound to an entity. Asked after the tick's other sounds. The fields and
    /// their rules are `play_level_sound_at_position`'s. Client entity-sound
    /// packets ask before playback and keep following their emitter; an absent
    /// emitter plays nothing. `LocalPlayer.playSound` also asks this hook,
    /// including steps, falls, hurt, bundle clicks and consumption; these remain
    /// positional sounds, not following sounds. Both sides, distinguished by `HostSide`.
    #[hook(
        side = "both",
        contract = "cancel",
        out = "cancel",
        neoforge = "PlayLevelSoundEvent.AtEntity",
        cancellable = true
    )]
    fn play_level_sound_at_entity(
        entity: EntityRef,
        sound: &mut Identifier,
        source: &mut SoundSource,
        volume: &mut f32,
        pitch: &mut f32,
        original_volume: f32,
        original_pitch: f32,
        cancel: &mut bool,
    );
}
