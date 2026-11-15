//! The `beacon_color_multiplier` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The colour the block `state` at `pos` gives the beam of the beacon at `beacon_pos`
    /// (`IBlockExtension.getBeaconColorMultiplier`), asked as the beacon builds its beam sections
    /// one block at a time. `color` starts at the block's own answer: the beacon's white and a
    /// stained glass block's or pane's dye colour, empty for the rest. Set `Some(rgb)` to tint
    /// the beam as stained glass does (a colour unlike the section below starts a new section
    /// mixed half and half with it), or `None` to let the beam pass untinted, or stop at an
    /// opaque block other than bedrock, as vanilla does. The beam takes the red, green and blue
    /// channels; alpha is ignored. Asked on both sides, since a client builds the beam it draws
    /// from the blocks it has, with the level read-only: register the mod on both and answer
    /// the same on each. With no part attached to the state, the game checks the hook
    /// subscription and the block key before any event conversion, so the cost is one table
    /// lookup per block above a beacon. Diverges from upstream: new hook.
    #[hook(
        side = "both",
        contract = "collect",
        out = "color",
        scope = "block",
        key = "state",
        host = "read",
        neoforge = "IBlockExtension.getBeaconColorMultiplier, IBlockStateExtension.getBeaconColorMultiplier"
    )]
    fn beacon_color_multiplier();
}
