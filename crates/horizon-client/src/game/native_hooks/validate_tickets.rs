use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// `dimension`'s level has loaded the saved chunk tickets of `controller`
    /// (NeoForge's `LoadingValidationCallback.validateTickets`), before its
    /// first tick and after `native_level_load`, once for each open and each
    /// controller registered with a validator that has tickets in the level.
    /// `validation` holds that controller's tickets, block owners and entity
    /// owners apart; the mod that owns the controller removes the ones that no
    /// longer hold (a machine that is gone), and they stop forcing their
    /// chunks. Server-side.
    pub(in crate::game) fn native_validate_tickets(
        &mut self,
        dimension: &str,
        controller: TicketController,
        validation: &mut dyn mod_api::TicketValidation,
    ) {
        self.fire_validate_tickets(dimension, controller, validation);
    }
}
