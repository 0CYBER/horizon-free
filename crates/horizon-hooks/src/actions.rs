//! Key mapping registration (docs/layers.md, "Platform registries"): a crate above the
//! engine registers its extra key mappings before the game runs, so the engine's input layer
//! names no Horizon action.
use crate::{TooLate, registry::Registry};

/// One crate's registered key mappings: `(name, default_key, category)`.
pub type Mappings = [(&'static str, &'static str, &'static str)];

static MAPPINGS: Registry<Mappings> = Registry::new();

/// Registers a crate's key mappings, before the game starts (as [`crate::register`]).
/// A later call is refused.
pub fn register(mappings: &'static Mappings) -> Result<(), TooLate> {
    MAPPINGS.register(mappings)
}

pub(crate) fn all() -> &'static [&'static Mappings] {
    MAPPINGS.all()
}

/// The engine's side. Only the engine calls it.
pub mod engine {
    /// Every crate's mappings, in registration order, sealing the list on the first call.
    pub fn all() -> &'static [&'static super::Mappings] {
        super::all()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mappings_registered_before_the_game_starts_are_kept_in_order() {
        static A: &Mappings = &[("key.custom.a", "key.keyboard.k", "misc")];
        static B: &Mappings = &[
            ("key.custom.b", "key.keyboard.j", "gameplay"),
            ("key.custom.c", "key.keyboard.h", "multiplayer"),
        ];
        assert_eq!(register(A), Ok(()));
        assert_eq!(register(B), Ok(()));
        let all = engine::all();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0][0].0, "key.custom.a");
        assert_eq!(all[1][1].0, "key.custom.c");
        assert_eq!(register(A), Err(TooLate));
    }
}
