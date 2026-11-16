//! The `entity_teleport` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0208

pub trait Hooks {
    /// A server entity was moved by a same-level teleport command, after its
    /// position changed. `entity` is its network id; `from` and `to` are
    /// coordinates in `dimension`. This hook reports command teleports that
    /// use Horizon's same-level target path; it does not change vanilla's
    /// movement.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "none (NeoForge has no such event)"
    )]
    fn entity_teleport();
}
