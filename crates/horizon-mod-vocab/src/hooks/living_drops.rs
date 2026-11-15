//! The `living_drops` hooks and helpers: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// Not `loot_drops`: that is the loot-modifier step, which edits
    /// the stacks a table rolled (blocks, shearing, a mob's table). This hook
    /// fires once after it, on the entity's whole collected drop list, and
    /// nothing is spawned until it returns.
    /// A dying mob, server player or armor stand `entity` of `kind` (`player`
    /// is a player's UUID) is about to drop `drops`, the items
    /// `LivingEntity.dropAllDeathLoot` collected (NeoForge's
    /// `LivingDropsEvent`): the loot tables' stacks (after `loot_drops`),
    /// the equipment that rolled to drop, a mob's own extras (an enderman's
    /// carried block, a fox's mouth item, a piglin's inventory, a copper golem's
    /// preserved equipment) and a player's inventory, each with the position,
    /// motion and pickup delay its item entity will have. Edit the list (take
    /// out, change or add drops) or set `cancel` for no drops at all; empty
    /// stacks don't drop. `source` is the killing blow (`causing` and `direct`
    /// entity ids, 0 when none or not known) and `recently_hit` is whether a
    /// player hurt it within 100 ticks, which is what lets a mob drop its
    /// equipment. An armor stand fires it from `brokenByAnything` with an
    /// empty list (its stand item and its equipment pop outside the list, as
    /// in vanilla). Experience orbs are not in the list
    /// (`living_experience_drop`); a spectator, who drops nothing,
    /// doesn't call this. Each `ItemDrop` also sets the item entity's
    /// `thrower`, `target` (the only player who may pick it up), `no_gravity`
    /// and starting `age` (`ItemDrop::UNLIMITED_LIFETIME` never despawns); a
    /// drop left as it was spawns as before. Server-side.
    #[hook(
        side = "server",
        contract = "cancel",
        out = "cancel",
        neoforge = "LivingDropsEvent",
        cancellable = true
    )]
    fn living_drops(
        entity: u32,
        kind: &str,
        player: Option<u128>,
        source: &HurtSource<'_>,
        recently_hit: bool,
        drops: &mut Vec<ItemDrop>,
        cancel: &mut bool,
    );
    /// A dying mob `entity` of `kind` is about to roll its drop chance for
    /// the item `stack` in `slot` (`Mob.dropCustomDeathLoot`, NeoForge's
    /// `GetEquipmentDropChanceEvent`). `original_chance` is
    /// `DropChances.byEquipment`; `chance` starts as the chance after the
    /// enchantments of the attacker and the victim (`getOriginalChance` and
    /// `getDropChance`); change `chance` (1.0 or more always drops, 0 or less
    /// never does). `preserved` is `DropChances.isPreserved`: the slot drops
    /// without a player kill, whatever the mod chooses. Each slot is asked
    /// at its own point of the loop, after the mob's loot table was rolled
    /// and just before the slot's roll, only when it holds an item and its
    /// recorded chance isn't 0, in `EquipmentSlot` order. The roll is drawn
    /// whatever the chance is, so the mob's random follows vanilla's order.
    /// `source` and `recently_hit` are as in `native_living_drops`.
    /// Server-side.
    #[hook(
        side = "server",
        contract = "listen",
        neoforge = "GetEquipmentDropChanceEvent",
        cancellable = false
    )]
    fn get_equipment_drop_chance(
        entity: u32,
        kind: &str,
        slot: EquipmentSlot,
        source: &HurtSource<'_>,
        recently_hit: bool,
        stack: &ItemStack,
        original_chance: f32,
        preserved: bool,
        chance: &mut f32,
    );
}
