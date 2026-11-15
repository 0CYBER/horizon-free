//! Construction before the immutable controller table exists.
/// Cold authoring access to the opaque token representation.
pub fn ticket_controller(slot: u32) -> crate::TicketController {
    crate::TicketController { slot }
}
