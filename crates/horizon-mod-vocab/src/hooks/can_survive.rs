//! The `can_survive` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0078

pub trait Hooks {
    /// On an authoritative server, `BlockState.canSurvive` for `state` at
    /// `pos` (`BlockBehaviour.canSurvive`), before the block answers it: as
    /// it is placed (`BlockItem.canPlace`), from a shape update of a block
    /// whose template checks it (a plant, a torch, a sign: false breaks the
    /// block with its drops), and wherever else vanilla asks. `state` may not
    /// be in the world yet, so decide from `state`. Answer only for your own
    /// blocks: `Some(false)` refuses the spot, `Some(true)` allows it, and
    /// the block's own rule doesn't run. The level is read-only. Asked on
    /// every shape update of such a block: keep it short.
    #[hook(
        side = "server",
        contract = "replace",
        out = "survives",
        scope = "block",
        key = "state",
        target = "horizon_client::game::native_hooks::base::GameLevel<'_>::native_can_survive",
        neoforge = "none (vanilla BlockBehaviour.canSurvive; NeoForge adds no event or extension for it)"
    )]
    fn can_survive();
}
