//! Client mining hit sound.
pub trait Hooks {
    /// Asked before a mining hit sound, including the initial hit, every fourth breaking
    /// tick, and the level-event route. Return Flow::Cancel when the part handled the
    /// sound through ClientHost and vanilla should stay silent.
    /// Otherwise vanilla plays the state's hit sound at (volume + 1) / 8 and pitch * 0.5.
    /// The position and face identify the current hit; sculk growth is outside this hook.
    #[hook(
        side = "client",
        contract = "cancel",
        out = "cancel",
        scope = "block",
        key = "state",
        neoforge = "IClientBlockExtensions.playHitSound"
    )]
    fn play_hit_sound(state: BlockState, pos: BlockPos, face: Direction, cancel: &mut bool);
}
