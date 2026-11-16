//! The `can_entity_destroy` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// On an authoritative server, whether `entity` may destroy the block `state` at `pos`
    /// (`IBlockExtension.canEntityDestroy`), asked where vanilla asks its per-block rule: the
    /// wither's block break (`WitherBoss.canDestroy`: not `#wither_immune`), a dangerous
    /// wither skull's blast (the entity is the skull, readable during the blast; only a block
    /// it may destroy has its resistance capped at 0.8), the dragon's `checkWalls` (a
    /// `#dragon_transparent` block is skipped before the part is asked, and a
    /// `#dragon_immune` block stays even on a yes), a zombie's door breaking (asked as the
    /// goal starts) and the turtle-egg `RemoveBlockGoal`'s target choice. `Some(true)` makes a
    /// block vanilla spares destroyable for that entity, `Some(false)` spares one it would
    /// break (the dragon is then in a wall), and `None` keeps vanilla's rule. The griefing
    /// gate (`entity_mob_griefing`) runs first, so with griefing off nothing breaks whatever
    /// the part says, and `living_destroy_block` can still keep a block the part allowed (the
    /// egg goal's target choice fires no such event). Ravager: griefing event only. A
    /// panicking part is contained and vanilla's rule is used. Asked every tick for each
    /// block in the dragon's head, neck and body: answer fast, from the inputs. The host is
    /// read-only. With no part attached to the state, the cost is one table lookup. Diverges
    /// from upstream: new hook.
    #[notes = r#"## Blocks an entity may destroy

- **`BlockPart::can_entity_destroy(host, event)`** (`CanEntityDestroyEvent { pos, state, entity }`): whether the entity `event.entity` may destroy the block (NeoForge's `IBlockExtension.canEntityDestroy`), asked where vanilla asks its per-block rule: the wither's block break (`WitherBoss.canDestroy`: any block but air and `#wither_immune`), a dangerous wither skull's blast, where the entity is the skull (kept in the level and readable through the blast) and only a block it may destroy has its explosion resistance capped at 0.8 (`WitherSkull.getBlockExplosionResistance`), the dragon's `checkWalls`, a zombie's door breaking (`BreakDoorGoal.canUse`) and the turtle-egg `RemoveBlockGoal`'s target choice (`isValidTarget`). `Some(true)` makes a block vanilla spares destroyable for that entity, such as `#wither_immune` bedrock for the wither; `Some(false)` spares one it would break, and a spared block in the dragon's way puts it in a wall. `None` leaves it to another part and then vanilla's rule. The order is: the griefing gate first (`entity_mob_griefing`, the `mob_griefing` rule), so with griefing off nothing breaks whatever the part says, then this answer, then `living_destroy_block`, which can still keep a block the part allowed (fired at the dragon's check and at the door's break; the egg goal's target choice fires none). For the dragon, `#dragon_immune` still holds after all of them: a yes does not bypass it, and a `#dragon_transparent` block is skipped before the part is asked. Ravager: griefing event only. A panicking part is contained with a warning and vanilla's rule is used. Authoritative servers only; the host is read-only. The dragon asks every tick for each block in its head, neck and body, so answer fast. With no part attached to the state, one table lookup per block. Diverges from upstream: new hook."#]
    #[hook(
        side = "server",
        contract = "replace",
        out = "may_destroy",
        scope = "block",
        key = "state",
        host = "read",
        target = "horizon_client::game::native_hooks::can_entity_destroy::Game<'_>::native_can_entity_destroy",
        neoforge = "IBlockExtension.canEntityDestroy"
    )]
    fn can_entity_destroy();
}
