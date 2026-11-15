//! Named ticket controllers (NeoForge's `TicketController` and `RegisterTicketControllersEvent`): a
//! mod registers each controller once at startup, from [`Hooks::register`](crate::Hooks::register),
//! and holds the [`TicketController`] it gets back to hold chunks with.

/// What the game knows about a registered controller.
pub(crate) struct ControllerInfo {
    /// The namespaced id, `mymod:quarry`, which the tickets are saved under.
    pub(crate) id: Box<str>,
    /// The `[mod] id` that registered it, the only mod that may use it.
    pub(crate) mod_id: &'static str,
    /// Whether it validates its saved tickets (NeoForge's `LoadingValidationCallback`).
    pub(crate) validates: bool,
}

/// A ticket controller a mod registered with
/// [`Registrar::register_ticket_controller`](crate::TicketsRegistrar::register_ticket_controller): the
/// handle `chunk.set_mod_forced` and `level.mod_forced_chunks` take. Only the registrar makes
/// one, resolved once there, and it is `Copy`: keep it from `register`, for example in a
/// `OnceLock`. Another mod's controller does not work.
// no wire: decoding from_slot requires membership in the installed ticket-controller table.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
// exhaustive: its fields are private, so mods cannot build or destructure it
pub struct TicketController {
    /// Its place in the installed controller table.
    pub(crate) slot: u32,
}

impl TicketController {
    /// The controller's namespaced id (`mymod:quarry`), as its tickets are saved under it; empty
    /// for a registration that was refused.
    pub fn id(self) -> &'static str {
        self.info().map_or("", |info| &info.id)
    }
    /// The controller in slot `slot` of the installed table. For the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn from_slot(slot: u32) -> Option<Self> {
        controllers().get(slot as usize).map(|_| Self { slot })
    }
    /// The installed controller with this id. For the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn from_id(id: &str) -> Option<Self> {
        let slot = controllers().iter().position(|info| &*info.id == id)?;
        Some(Self { slot: slot as u32 })
    }
    /// Its slot in the installed table. For the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn slot(self) -> u32 {
        self.slot
    }
    /// The `[mod] id` that owns it. For the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn owner(self) -> Option<&'static str> {
        self.info().map(|info| info.mod_id)
    }
    /// Whether it validates its saved tickets. For the game.
    // hidden: a game-side helper, not part of the mod surface
    #[doc(hidden)]
    pub fn validates(self) -> bool {
        self.info().is_some_and(|info| info.validates)
    }
    fn info(self) -> Option<&'static ControllerInfo> {
        controllers().get(self.slot as usize)
    }
}

#[cfg(any(test, feature = "testing"))]
thread_local! {
    /// The controllers a test's mods registered (`testing::with_dispatch`), in place of the
    /// process-wide table.
    pub(crate) static TEST_CONTROLLERS: std::cell::Cell<Option<&'static [ControllerInfo]>> =
        const { std::cell::Cell::new(None) };
}

fn controllers() -> &'static [ControllerInfo] {
    #[cfg(any(test, feature = "testing"))]
    if let Some(table) = TEST_CONTROLLERS.get() {
        return table;
    }
    crate::parts::installed_controllers()
}
