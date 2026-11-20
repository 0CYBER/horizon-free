//! Refusals when a mod hides an entity from one player's client, or asks whether it is hidden.

/// Why `entity.set_visible_to` or `entity.visible_to` answered with nothing done: nothing was
/// hidden, shown or read.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum EntityVisibilityRefusal {
    /// The viewer handle no longer identifies a player in the server's list: it is stale, or the
    /// player has left.
    Stale,
    /// The call was made on a client; these are the server's.
    Client,
    /// The viewer has no connection to send to (a fake player).
    NoConnection,
    /// The entity handle no longer names an entity in the world, or names the viewer's own
    /// player, which is never tracked to itself.
    StaleEntity,
}

impl crate::NoBackend for EntityVisibilityRefusal {
    fn no_backend() -> Self {
        Self::Stale
    }
}
