//! The `player_start_spectating_entity` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A server player started spectating `entity_id`, after their camera moved to it
    /// (`ServerPlayer.setCamera` with a new target: a spectator's click or `/spectate`) and their
    /// client was told. Heard once per change of target, not again while the camera follows it.
    /// The mirror of `player_stop_spectating_entity`.
    #[notes = r#"## Player start spectating entity

- **`player_start_spectating_entity(host, event)`**: a spectator's camera moved to `event.entity_id`, from a click (`handleSpectatorAction`) or `/spectate`. It is heard after vanilla's `ServerPlayer.setCamera` moved the view to the target and sent `ClientboundSetCameraPacket`, once per new target: switching straight from one entity to another is one start for the new one, and `player_stop_spectating_entity` is heard only when the camera returns to the player. Setting the camera to the target it already has, to the player's own entity, or to an entity that is gone (no camera is set then) is not heard."#]
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn player_start_spectating_entity();
}
