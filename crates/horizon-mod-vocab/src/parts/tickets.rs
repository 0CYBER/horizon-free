use super::*;
/// The ticket controllers registered at startup.
pub(crate) fn installed_controllers() -> &'static [ControllerInfo] {
    installed().map_or(&[], |installed| installed.controllers.as_slice())
}

/// Registrar operations for tickets.
pub trait TicketsRegistrar {
    /// Register ticket controller `name`, the way NeoForge's `RegisterTicketControllersEvent`
    /// does once at startup, and keep the handle for `chunk.set_mod_forced`. `name` is
    /// `quarry` or `mymod:quarry`; the controller's id is in a namespace owned by the calling mod's manifest
    /// (`mymod:quarry`), and an unowned namespace refuses the mods at startup, as does an id
    /// registered twice. `validates` is whether it validates its saved tickets: the
    /// `validate_tickets` hook then runs for it when a level loads them (NeoForge's
    /// `LoadingValidationCallback`). A world's tickets of a controller no loaded mod registers are
    /// dropped when it loads.
    fn register_ticket_controller(&mut self, name: &str, validates: bool) -> TicketController;
}
