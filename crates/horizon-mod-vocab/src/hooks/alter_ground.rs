//! The `alter_ground` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// A tree's `alter_ground` decorator (`AlterGroundDecorator.place`, the
    /// podzol under a big spruce) is about to turn the ground around
    /// `positions`: the lowest of its trunk or roots (`logs` if it has no roots,
    /// `logs` and `roots` when they start level, else `roots`); only those at
    /// the first one's Y are placed around. `logs`, `leaves` and `roots` are the
    /// decorator context's lists, each sorted by Y. Set `provider` to your own
    /// `GroundProvider::Mod` to choose the ground; to change it only where you
    /// like, wrap the provider you found and ask it for the rest, since several
    /// mods may each wrap the one before. Once per decorated tree, before any
    /// ground is placed; the hook itself draws nothing. Server-side, on the
    /// worldgen worker threads (and on the server thread when a sapling grows),
    /// with the server's worldgen `context`: no `Host`, no writes.
    #[notes = r#"## Tree ground

- **`alter_ground(context, event)`**: NeoForge's `AlterGroundEvent`, fired where `AlterGroundDecorator.place` has found
  the positions it places around and before it places any ground. `event.provider` starts as
  `GroundProvider::Decorator`, the decorator's own provider; a mod sets it to `GroundProvider::Mod` with its own
  `GroundStateProvider`. The decorator then asks that provider once per position it tries, top down in each column
  (`state_at(level, pos)`): a state is placed there and the column stops, None tries the next position down. A
  provider reads the region through `level.block_state(pos)`, draws from the decorator's own random through
  `level.random()`, and asks the provider it wrapped with `GroundProvider::state_at` (`level.decorator_state(pos)`
  for the decorator's own). With no listener, or with `provider` left as `Decorator`, the decorator runs exactly as
  vanilla's, blocks and draws alike; a provider that only wraps the decorator's own keeps them too.
- **Where it runs.** On the worldgen worker threads while chunks decorate, and on the server thread when a sapling
  grows into a tree with this decorator. There is no `Host`: `context` is a `WorldgenHost`, the server's registries,
  settings and mod list as they were at its last resource load (start or `/reload`). A provider is shared by the
  workers, so it keeps no mutable state; the same seed must give the same ground.
- **Answers.** A state the game's registry doesn't have stops the tree feature, as an invalid state in a data pack
  does. A wasm mod's hook and provider calls run one at a time for the mod's group: a wasm listener serialises the
  workers that decorate trees with this decorator.
- **Cost.** With no listener the decorator pays one subscription check per decorated tree and allocates nothing.
  With a listener, one event per tree; a mod's provider is asked per position instead of the decorator's."#]
    #[hook(
        side = "server",
        contract = "collect",
        out = "provider",
        context = "worldgen",
        target = "mod_api::world::ground::native_alter_ground",
        neoforge = "AlterGroundEvent"
    )]
    fn alter_ground();
}
