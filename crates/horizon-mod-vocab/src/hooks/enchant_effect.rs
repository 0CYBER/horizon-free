//! The `enchant_effect` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0090

pub trait Hooks {
    /// A `minecraft:post_attack` effect of a mod's own kind, on a mob a
    /// player's weapon hit: the entry says `enchanted` attacker and `affected`
    /// victim, its `requirements` passed, and its `effect` names a `type` in a
    /// namespace outside `minecraft:`. Told the enchantment id, the kind, the
    /// `effect` payload as a borrowed `Tag`, the level and the mob's id (it may have just died). The
    /// acting player is the one whose weapon carries the enchantment. Each
    /// such entry is told once a hit, and once for each mob a sweep lands on.
    /// A hit on a player, and an entry inside `minecraft:all_of`, aren't told
    /// yet. An unclaimed kind does nothing.
    #[hook(side = "server", contract = "listen", neoforge = "RegisterEvent")]
    fn enchant_effect();
}
