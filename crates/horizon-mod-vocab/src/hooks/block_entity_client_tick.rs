//! The `block_entity_client_tick` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A client tick of this client's copy of the mod block entity at `pos`
    /// (`ClientLevel.tickBlockEntities`), when its `id` is yours and its
    /// block's state ticks on the client (`block_entity_client_ticks`):
    /// once per client tick while the level runs, in vanilla's ticker order
    /// among the others, whether or not the copy is drawn. `block_entity` is
    /// its handle, `state` its block's state and `entity` its data, read only,
    /// as the frame's handling of the server's packets merged it before the
    /// frame's ticks (`block_entity_update_tag`). Start, stop and fade
    /// its sounds and add its particles here. Never called on a server, and
    /// never again for a copy removed or whose chunk unloaded.
    #[notes = r##"### Client ticks: hums, smoke and moving parts

`block_entity_client_tick` is vanilla's client `BlockEntityTicker` (`EntityBlock.getTicker` asked with the
client level): what a machine does on the client alone, a looping hum, smoke from its chimney, a part eased
between two server updates. A block entity type opts in with `block_entity_client_ticks` (its client side of
`block_entity_ticks`), asked once for each mod block; with no part answering, nothing is asked and nothing runs.

It runs on each client (the window's own in single player included) once per client tick while its level runs
(not paused or frozen), for each loaded copy of the type, in the order vanilla's client ticker list gives
(`Level.blockEntityTickers`): the order the copies arrived, a chunk's own in its map's order. A loaded copy
ticks whether or not it is drawn (`block_entity_submit` runs only for those in view). The event has the copy's
position, handle, block state and data, and a `ClientHost`: play a sound with `play_local_sound` and keep its
`SoundRef` to stop or fade it, add particles with `add_particle`. The data is read only and is what the frame's
handling of the server's packets merged into the copy (`block_entity_data_received` told first): a frame
handles what has arrived and then runs its client ticks, so all the ticks of one frame see the same data and an
update arriving during a frame is seen from the next frame's first tick. The copy's snapshot is made once for
each change to it, not once a tick. A copy that is removed, replaced by another type, changed to a state without
a client ticker or whose chunk unloads is not called again from that tick on, and its handle no longer reads
once it is removed, replaced or unloaded: stop what you keep for it then. It never runs on a server."##]
    #[hook(
        side = "client",
        contract = "listen",
        scope = "block_entity",
        key = "entity",
        neoforge = "none (vanilla BlockEntityTicker from EntityBlock.getTicker with the client level; NeoForge adds no event)"
    )]
    fn block_entity_client_tick();
}
