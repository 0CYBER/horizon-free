//! The `viewed_level_sound` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A sound of a level the server opened for viewing (arismoko/horizon#3716) reached this client, and the
    /// client asks where in the player's own level to play it. `level` is the viewed level's
    /// handle (what `host.level` gives for it) and `position` the sound's position there; `sound`,
    /// `source` and `seed` are the server's. `play_at` starts as None, and a sound left None is not
    /// played. Set it to a position in the player's level and the sound plays there, as the
    /// player's own level plays a sound packet (no distance delay), with the `volume` and `pitch`
    /// the listeners leave and the same seed. A `play_at` that is not finite, or a volume or pitch
    /// that is not finite and at least 0, plays nothing. Only a server of `horizon:levels`
    /// revision 3 sends these. A sound bound to an entity of the viewed level has that entity's
    /// position there, and plays at `play_at` without following it. Sounds of the player's own
    /// level ask `play_level_sound_at_position` and
    /// `play_level_sound_at_entity`, never this hook. With no listener the sound is dropped after
    /// one check, with nothing decoded or allocated. Client-side. Diverges from upstream: new
    /// hook, a Horizon addition with no NeoForge counterpart.
    #[hook(
        side = "client",
        contract = "listen",
        neoforge = "none (NeoForge has no viewed levels)"
    )]
    fn viewed_level_sound(
        level: LevelRef,
        sound: &Identifier,
        source: SoundSource,
        position: [f64; 3],
        volume: &mut f32,
        pitch: &mut f32,
        seed: i64,
        play_at: &mut Option<[f64; 3]>,
    );
}
