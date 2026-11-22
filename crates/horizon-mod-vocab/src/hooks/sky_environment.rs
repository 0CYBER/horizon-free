//! The `sky_environment` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The time of day and the rain and thunder levels this client draws the world with, `partial`
    /// ticks past the last tick: `day_time` comes in as the level's (`Level.getDayTime`, or PvP &
    /// HUD's view time), `rain` and `thunder` as its levels in 0..=1 (`getRainLevel`,
    /// `getThunderLevel`). Set them to show another time or weather to this player only: noon
    /// while building, clear skies for a recording, night on a daytime server. Only the picture
    /// follows: the sun, moon and stars, the sky's and fog's colour, the sky's light, clouds and
    /// rain or snow. The level's clock, its weather, gameplay (crops, mobs, beds) and the server
    /// keep theirs, and nothing is sent. Rain and thunder count only where the dimension has
    /// weather; values that aren't finite keep the level's, others are clamped to 0..=1. Asked
    /// whenever the frame reads the sky, so it may be asked more than once a frame: answer the
    /// same each time. Client-side, only in a client of a world (single player's included).
    /// Beyond NeoForge: there is no counterpart (Java mods change `ClientLevel` with a mixin).
    #[hook(
        side = "client",
        contract = "listen",
        cancellable = false,
        neoforge = "none (client mods use a mixin on ClientLevel)"
    )]
    fn sky_environment();
}
