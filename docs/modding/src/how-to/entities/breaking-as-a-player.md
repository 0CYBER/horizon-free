# Breaking blocks as a player

`PlayerRef::destroy_block(host, pos, into_inventory)` breaks a block as that player breaks it,
through vanilla's `ServerPlayerGameMode.destroyBlock`, as FTB Ultimine does for each block of a
vein. The player's own checks hold (game mode, the tool's `can_break`, game master blocks, spawn
protection, bedrock), and the break is the player's: `break_block` and `player_destroyed_block`
hear it, the loot is rolled for the held tool and its enchantments, `block_drops` sees the drops
with the player as `breaker`, and the tool wears, the mined statistic counts and the player gets
the exhaustion. The player needn't reach or look at the block. It is true when the block was
removed. With `into_inventory`, the drops `block_drops` leaves go to the player's inventory, and
what doesn't fit drops from them.

## A vein miner

A vein miner keeps the drops of its own breaks with `block_drops`: it takes the item list and the
experience during the breaks, without canceling, so the other effects of each break still happen.
Then it merges the stacks and sets them down at the block the player mined, with
`LevelRef::pop_resource` and `LevelRef::award_experience`. Nothing spawns at the other blocks.

```rust,ignore
struct VeinMiner {
    busy: AtomicBool,
    vein: Mutex<Vec<[i32; 3]>>,
    kept: Mutex<(Vec<ItemStack>, u32)>,
}

impl Hooks for VeinMiner {
    fn break_block(&self, host: &mut dyn ServerHost, event: &mut BreakBlockEvent<'_>) -> Flow {
        // Our own breaks come through here too.
        if self.busy.swap(true, Ordering::Relaxed) {
            return Flow::Continue;
        }
        let player = host.players().acting_player().unwrap();
        let origin = event.pos;
        let vein = find_vein(host, origin); // the origin first
        *self.vein.lock().unwrap() = vein.clone();
        for pos in vein {
            player.destroy_block(host, pos, false);
        }
        self.busy.store(false, Ordering::Relaxed);
        let (stacks, experience) = std::mem::take(&mut *self.kept.lock().unwrap());
        let level = player.level(host).unwrap();
        for stack in merge(host, stacks) {
            let _ = level.pop_resource(host, origin, stack);
        }
        let [x, y, z] = origin.map(|v| f64::from(v) + 0.5);
        level.award_experience(host, x, y, z, experience);
        // The origin is already broken, by the loop.
        Flow::Cancel
    }

    fn block_drops(&self, _: &dyn LevelHost, event: &mut BlockDropsEvent<'_>) -> Flow {
        let pos = [event.pos.x, event.pos.y, event.pos.z];
        if self.busy.load(Ordering::Relaxed) && self.vein.lock().unwrap().contains(&pos) {
            let mut kept = self.kept.lock().unwrap();
            kept.0.extend(event.drops.drain(..).map(|drop| drop.stack));
            kept.1 += std::mem::take(&mut event.experience);
        }
        Flow::Continue
    }
}
```

`merge` folds each stack into the earlier ones it `is_stackable_with`, up to
`host.max_stack_size(stack.item())`, in the order they came. Filter `block_drops` on the
breaker too when other players can break the same blocks while the vein runs. What else a break
sets loose (a torch losing its support, a container's contents) is not the block's own drop and
still drops where it was.
