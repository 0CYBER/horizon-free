//! Refusals when a mod places a configured feature (a tree, a mushroom) in a server level.

/// Why `level.generate_tree` was not tried: the call was refused before the feature ran.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
#[derive(horizon_mod_api_macros::Wire)]
pub enum FeatureRefusal {
    /// The level handle belongs to an earlier open of the world, or the level is gone.
    Stale,
    /// The call was made on a client; features are placed by the server.
    Client,
    /// The level is not the one in place, the one being ticked; only that one is written.
    NotInPlace,
    /// No configured feature of that id is in the level's feature registry.
    UnknownFeature,
}

impl crate::NoBackend for FeatureRefusal {
    fn no_backend() -> Self {
        Self::Stale
    }
}
