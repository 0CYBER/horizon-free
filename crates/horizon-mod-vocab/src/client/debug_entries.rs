//! Named entries and world-space renderers a native mod adds to the debug screen (NeoForge's
//! `RegisterDebugEntriesEvent`, `RegisterDebugRenderersEvent`).

/// A debug entry a native mod registers: its id and the vanilla group its lines are drawn in.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: a mod builds it with a literal, an id and a group
#[derive(horizon_mod_api_macros::Wire)]
pub struct DebugEntry {
    /// `<namespace>:<path>` in a namespace other than vanilla's, like `mymod:tick_cost`.
    pub id: String,
    /// The group its lines join, with vanilla's lines of that group, or `DebugGroup::Renderer`.
    pub group: DebugGroup,
}

/// A world-space debug renderer a native mod pushes at each renderer list refresh.
#[derive(Clone, Debug, PartialEq, Eq)]
// exhaustive: a mod builds it with a literal, an id
#[derive(horizon_mod_api_macros::Wire)]
pub struct DebugRenderer {
    /// The id of the `DebugGroup::Renderer` entry that switches it, like `mymod:pipe_network`.
    pub id: String,
}

/// The vanilla groups a debug entry's lines can join (`DebugScreenDisplayer.addToGroup`).
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
// exhaustive: vanilla's debug screen groups and renderer section, matched without a wildcard
#[derive(horizon_mod_api_macros::Wire)]
pub enum DebugGroup {
    /// `position`: the player's position, section and speed.
    Position,
    /// `light`: the light levels at the feet.
    Light,
    /// `memory`: the process's memory use.
    Memory,
    /// `looking_at_block`: the targeted block's state and tags.
    LookingAtBlock,
    /// `looking_at_entity`: the targeted entity and its tags.
    LookingAtEntity,
    /// `looking_at_fluid`: the targeted fluid's state and tags.
    LookingAtFluid,
    /// `chunk_generation`: the density of the chunk at the feet.
    ChunkGeneration,
    /// No lines: a switch in the renderer section (`DebugEntryNoop`) for a `DebugRenderer`.
    Renderer,
}
