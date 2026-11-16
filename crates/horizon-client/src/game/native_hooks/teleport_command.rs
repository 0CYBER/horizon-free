use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// A server entity is about to be teleported by `/tp` or `/teleport`
    /// (NeoForge's `EntityTeleportEvent.TeleportCommand`, in
    /// `TeleportCommand.performTeleport`, once per teleported entity):
    /// `dimension` is the level it arrives in, `entity` its network id,
    /// `from` its feet now and `to` where the command sends it. A mod may
    /// move `to` (a value that is not a number is ignored). Canceling leaves
    /// the entity where it is; the command still reports success.
    /// `native_entity_teleport` and `native_player_teleported` hear it after.
    pub(in crate::game) fn native_teleport_command(
        &mut self,
        dimension: &str,
        entity: u32,
        from: [f64; 3],
        to: &mut [f64; 3],
        cancel: &mut bool,
    ) {
        self.fire_teleport_command(dimension, entity, from, to, cancel);
    }
}
