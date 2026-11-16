//! Links supplied by a product for the Mods screen's built-in entries. The engine knows the
//! entries, but only the product chooses the outside services their buttons open.
use crate::{TooLate, registry::Registry};

/// The links for one built-in entry, identified by its mod id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Links {
    pub id: &'static str,
    pub license: Option<&'static str>,
    pub homepage: Option<&'static str>,
    pub issues: Option<&'static str>,
}

static LINKS: Registry<Links> = Registry::new();

/// Adds links before the game starts. If several registrations name an entry, the first wins.
pub fn register(links: &'static Links) -> Result<(), TooLate> {
    LINKS.register(links)
}

/// The engine's side. Only the engine calls it.
pub mod engine {
    /// The registered links, sealing the list on the first call.
    pub fn all() -> &'static [&'static super::Links] {
        super::LINKS.all()
    }
}
