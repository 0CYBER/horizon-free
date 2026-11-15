# Slowing or boosting entities on a block

Attach a `BlockPart` to change how fast, or how high, entities move on your blocks, as a block that overrides NeoForge's `Block.getSpeedFactor()` and `Block.getJumpFactor()` does. Both Part methods get the loop `context` of the side that ticks the entity (no `Host`) and an event with the block `state` and the `factor` to edit, which starts at the block's own value (honey is 0.4 speed and 0.5 jump, soul sand 0.4 speed, every other block 1.0).

```rust,ignore
struct Mud;

impl mod_api::BlockPart for Mud {
    fn block_speed_factor(&self, _: &dyn mod_api::LoopContext, event: &mut mod_api::BlockSpeedFactorEvent) {
        event.factor = 0.25;
    }
    fn block_jump_factor(&self, _: &dyn mod_api::LoopContext, event: &mut mod_api::BlockJumpFactorEvent) {
        event.factor = 0.5;
    }
}

// In `register`: let mud = r.block_key("mymod:mud"); r.block_part(mud, Mud);
```

The entity reads the block at its feet and, when that gives 1.0 (and, for speed, the block is not water or a bubble column), the block below it. A part's factor is the block's factor in that rule, so 1.0 from your part lets the block below decide, exactly as for a vanilla block.

The player, every mob's ground and jump path, boats and minecarts read the factors where vanilla does. Both hooks fire on both sides and on the client's replayed ticks: register the mod on both and keep each method a pure function of its inputs, or prediction and the server disagree. To affect entities on every block, use `living_speed` and `living_jump_power` instead.

See [the hook notes](https://github.com/horizon-rust/horizon/blob/main/crates/horizon-mod-vocab/src/hooks/block_factors.rs) for the cost and the details.
