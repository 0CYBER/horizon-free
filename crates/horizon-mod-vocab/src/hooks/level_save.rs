//! The `level_save` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// `dimension`'s level was saved (NeoForge's `LevelEvent.Save`, in
    /// `ServerLevel.save`): its level data, chunks and entities, by the
    /// autosave, `/save-all`, a pause, a closing level or the server
    /// stopping; each level of a world save in turn. Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "LevelEvent.Save",
        paper = "WorldSaveEvent"
    )]
    fn level_save();
}
