//! The `can_survive` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0078

pub trait Hooks {
    /// `can_survive` again, for the world generation of the server: while a feature places
    /// `state` at `pos`, its parts answer `survives` here, as they answer `survives` in the
    /// live game. Answer only for your own blocks: `Some(false)` refuses the spot, `Some(true)`
    /// allows it, and the block's own rule doesn't run. A rule that picks the ground the block
    /// stands on answers `may_place_on_v2` instead. Server-side, on the worldgen worker threads
    /// and on the server thread, with the server's worldgen `context`: no `Host`, no writes. A
    /// panicking part is contained and the block's own rule kept. Diverges from upstream:
    /// `can_survive` in world generation.
    #[notes = r#"## The worldgen survival questions

- **Which Part methods.** A `BlockPart` answers `can_survive_v2`, `may_place_on_v2` and `can_sustain_plant_v2` while a feature places a block in world generation, with the server's worldgen `context`. `can_survive` and `may_place_on` stay the live game's, with the level to read: a mod that wants one answer in both places answers both pairs.
- **Where it is asked.** The table a feature's placement reads asks the soil's `can_sustain_plant_v2` first and the plant's `may_place_on_v2` second, then its own rule, exactly as the live game asks that pair before `VegetationBlock.mayPlaceOn`. `can_survive_v2` comes before either, where the live game asks `canSurvive` first.
- **What it reads.** `context` is a `WorldgenHost`, the server's registries, settings and mod list as they were at its last resource load; there is no `Host` and nothing to write. A part is a pure function of its inputs: neighbouring chunks generate on worker threads at the same time.
- **Answers.** `Some(true)` and `Some(false)` replace the game's own rule; left `None` the game's rule answers. A refusal never panics."#]
    #[hook(
        side = "server",
        contract = "replace",
        out = "survives",
        context = "worldgen",
        scope = "block",
        key = "state",
        target = "mod_api::world::survival::native_can_survive_v2",
        neoforge = "none (vanilla BlockBehaviour.canSurvive; NeoForge adds no event or extension for it)"
    )]
    fn can_survive_v2();
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
