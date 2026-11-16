//! The `music_layers` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0122

pub trait Hooks {
    /// The tracks to play over (or instead of) the music on this client,
    /// asked once a client tick: push a `MusicLayer` (a sound event, looped,
    /// and its volume, 0 to 1) for each. A layer eases to a new volume over up
    /// to two seconds, and one left out fades out and stops; layers that
    /// first appear in the same tick start together, so list every stem of a
    /// piece from the start, the silent ones at volume 0, and raise them (a
    /// fuller arrangement as a village grows). Their sound events should be
    /// `stream: true` in `sounds.json`.
    #[hook(
        side = "client",
        contract = "collect",
        out = "layers",
        neoforge = "none (SelectMusicEvent swaps the one track; NeoForge has no layered music)"
    )]
    fn music_layers();
}
