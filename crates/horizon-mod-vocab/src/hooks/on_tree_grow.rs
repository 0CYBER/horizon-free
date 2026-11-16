//! The `on_tree_grow` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A trunk placer is about to place the block under its trunk at `pos`, where the soil
    /// `state` this part is attached to stands (`IBlockExtension.onTreeGrow`, asked from
    /// `TrunkPlacer.placeBelowTrunkBlock`). Answer `Some(Some(state))` to place `state` there
    /// through the tree's trunk setter, `Some(None)` to place nothing, or `None` to leave it to the
    /// tree's `below_trunk_provider` (vanilla's trees place dirt unless the soil is in
    /// `#minecraft:cannot_replace_below_tree_trunk`). A part that answers skips the provider and
    /// its draws. Asked once per trunk, four times for the 2×2 trunks (giant, mega jungle, dark
    /// oak). Draw only from `event.random`, the tree's own: your draws fall between the tree's, in
    /// call order. Server-side, on the worldgen worker threads and on the server thread when a
    /// sapling grows, with the server's worldgen `context`: no `Host`, no writes. NeoForge's
    /// `feature` argument is left out: an inline tree feature has no key. Diverges from upstream:
    /// new hook.
    #[notes = r#"## Block Parts in worldgen

- **Which Part methods.** A `BlockPart` method declared with the worldgen context runs while a feature generates:
  today `on_tree_grow` and `rotate`. The game asks them on the worldgen worker threads, and on the server thread when a sapling
  grows into a tree, with the same tables in both places.
- **What it reads.** `context` is a `WorldgenHost`, the server's registries, settings and mod list as they were at
  its last resource load; there is no `Host` and nothing to write. The parts asked are those the server resolved
  against its own tags when its world opened or at its last `/reload`, attached by key or by tag alike; a client
  window's tables never reach the workers. A feature that started before a `/reload` finishes with the tables it
  started with.
- **What it may keep.** Nothing mutable: neighbouring chunks generate at the same time on other threads, so a part
  is a pure function of its inputs. Draw only from the feature's own random in the event: the part's draws fall
  between the feature's, in call order, and the same seed gives the same world.
- **Answers.** A state the game's registry does not have is refused: the game logs the mods whose parts answered and
  keeps the feature's own answer. A refusal never panics.
- **Wasm.** A wasm part answers the same way. Its calls run one at a time for its mod's group, so a wasm part
  serialises the workers that ask it.
- **Cost.** With no part subscribed, a call costs one subscription check. A lookup reads this thread's handle on
  the published tables and takes no lock."#]
    #[hook(
        side = "server",
        contract = "replace",
        out = "soil",
        context = "worldgen",
        scope = "block",
        key = "state",
        target = "mod_api::world::ground::native_on_tree_grow",
        neoforge = "IBlockExtension.onTreeGrow"
    )]
    fn on_tree_grow();
}
