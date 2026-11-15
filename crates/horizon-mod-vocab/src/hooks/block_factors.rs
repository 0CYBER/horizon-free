//! The `block_factors` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// The jump factor of a block state this part is attached to (`Block.getJumpFactor`, which `Entity.getBlockJumpFactor` reads for the player, the cube, the panda, the rabbit and every other living jumper) is being read; vanilla's values are 0.5 for honey and 1.0 for the rest. A part may change `factor` (`event.factor`). The entity reads the block at its feet and, when that gives 1.0, the block below it, so a changed factor takes part in that choice exactly as the block's own would. It runs for each read, on both sides and on the client's replayed ticks, so register the mod on both and keep the listener a pure function of its inputs. It is given the loop `context` of the side that ticks the entity, and takes no `Host`. NeoForge's counterpart `Block.getJumpFactor()` is a method of the block, with no bus event, so this is a part method only; a mod reacting to its own blocks uses this part. Diverges from upstream: new hook.
    #[notes = r#"## Block jump and speed factors

- **Block Part methods `block_jump_factor(context, event)` and `block_speed_factor(context, event)`**: edit `event.factor`, the factor of the block state `event.state` the part is attached to (NeoForge's `Block.getJumpFactor()` and `Block.getSpeedFactor()`; vanilla's values are 0.5 jump and 0.4 speed for honey, 0.4 speed for soul sand, 1.0 elsewhere) where an entity reads it. `factor` starts at the block's own value. The edit is made on each read of that block, before the entity combines the blocks as vanilla does: the jump factor of the block at the feet unless it is 1.0, then the block below; the speed factor of the block at the feet unless that is water, a bubble column or not 1.0, then the block below. So a part that gives its block 0.5 slows an entity on it just as honey does, and one that gives 1.0 to honey lets the block below decide. Read for the player, every mob's ground and jump path (the rabbit's hop included), boats and minecarts where vanilla reads the speed factor (`Entity.move`'s last step). They fire on both sides and on the client's replayed ticks, so a mod registers on both and keeps each listener a pure function of its inputs. `context` is the loop context of the side that ticks the entity, as for the motion value hooks above. The player's `movement_efficiency` still applies after the part's factor. There is no global method: NeoForge has no bus event for these, so a mod reacting to its own blocks attaches a part and one reacting to every block listens to `living_jump_power` and `living_speed`. With no part attached to the state, the owning game checks the hook subscription and prepared API block key before any event conversion; with none installed the movement result is bit-identical to vanilla's. Beyond NeoForge (no event); Diverges from upstream: new hooks."#]
    #[hook(
        side = "both",
        contract = "listen",
        context = "loop",
        scope = "block",
        key = "state",
        cancellable = false,
        target = "horizon_client_lib::motion::surface::native_block_jump_factor",
        neoforge = "none (vanilla Block.getJumpFactor; NeoForge adds no event or extension for it)"
    )]
    fn block_jump_factor();
    /// The speed factor of a block state this part is attached to (`Block.getSpeedFactor`, which `Entity.getBlockSpeedFactor` reads for the player, mobs, boats and minecarts where vanilla reads it) is being read; vanilla's values are 0.4 for honey and soul sand and 1.0 for the rest. A part may change `factor` (`event.factor`). The entity reads the block at its feet and, when that is not water or a bubble column and gives 1.0, the block below it, so a changed factor takes part in that choice exactly as the block's own would. It runs for each read, on both sides and on the client's replayed ticks, so register the mod on both and keep the listener a pure function of its inputs. It is given the loop `context` of the side that ticks the entity, and takes no `Host`. NeoForge's counterpart `Block.getSpeedFactor()` is a method of the block, with no bus event, so this is a part method only; a mod reacting to its own blocks uses this part. Diverges from upstream: new hook.
    #[hook(
        side = "both",
        contract = "listen",
        context = "loop",
        scope = "block",
        key = "state",
        cancellable = false,
        target = "horizon_client_lib::motion::surface::native_block_speed_factor",
        neoforge = "none (vanilla Block.getSpeedFactor; NeoForge adds no event or extension for it)"
    )]
    fn block_speed_factor();
}
