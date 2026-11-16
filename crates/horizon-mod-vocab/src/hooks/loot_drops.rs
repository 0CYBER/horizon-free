//! The `loot_drops` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0088

pub trait Hooks {
    /// A block broke or a mob died, and `drops` are the items about to drop, in
    /// the order they drop: what the loot tables rolled (after
    /// `loot_table_load`'s changes), and for a mob what it wore, held or
    /// carried (a fox's mouth item, an allay's, piglin's or horse's inventory),
    /// but not an enderman's carried block or a copper golem's preserved
    /// equipment, which drop after. A charged creeper's head and a fox's mouth
    /// item come before the mob's own table. A block's stacks get its block
    /// entity's data (a shulker box's items) after this. Add, change or take out
    /// stacks; empty ones don't drop (NeoForge's global loot modifiers, Fabric's
    /// `LootTableEvents.MODIFY_DROPS`); `context` says what dropped them and
    /// how, and it is called even when `drops` is empty. A player's use drops
    /// through it too (`LootDrops::used`: shears, a brush, an item
    /// frame), with that player named. Lead drops from untying or cutting a
    /// leash, and from a mob a player killed while leashed, pass through here
    /// too, with that player named. It hears blocks that a
    /// player, an explosion or a mob breaks, not other breaks (leaves decaying,
    /// a torch losing its wall, a piston, `/setblock ... destroy`), nor a
    /// creative break or a tool that can't harvest the block, nor experience.
    /// For deaths, `context.damage_source` carries the death invocation's
    /// damage type and causing/direct IDs (not remembered player credit), and
    /// `context.tool` is None (the entity context has no TOOL). Resolve the causing
    /// ID through `host.entity` and `host.living` to read its main hand live with
    /// `living.hand_item(host, Hand::Main)`, including earlier listeners' changes.
    /// `context.weapon_item(host)` reads the recorded projectile weapon or the
    /// live melee weapon, including a player's active auto-spin stack.
    /// Block and use drops retain their tool meaning.
    /// Server-side, for every player.
    #[hook(
        side = "server",
        contract = "collect",
        out = "drops",
        neoforge = "LivingDropsEvent, BlockDropsEvent"
    )]
    fn loot_drops();
}
