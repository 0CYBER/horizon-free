//! The `is_piglin_currency` hook: declarations the mod API generator reads (see `hooks/mod.rs`).

pub trait Hooks {
    /// NeoForge's `IItemExtension.isPiglinCurrency`: is `stack` barter currency to a piglin
    /// (`PiglinAi.isBarterCurrency`)? `currency` arrives with vanilla's answer, whether the
    /// stack is a `minecraft:gold_ingot`; set it `true` to make the stack currency, `false` to
    /// refuse it (a gold ingot included), or leave it to keep vanilla's answer. Asked where
    /// vanilla asks: whether a piglin wants a stack on the ground (currency is wanted whenever
    /// its off hand holds no `#minecraft:piglin_loved` item, with no inventory-space check);
    /// whether an adult piglin that isn't admiring takes the stack from a player's hand, on the
    /// server and in the client's prediction of that click; and when a piglin stops holding the
    /// item it admired, where an adult barters currency (`minecraft:gameplay/piglin_bartering`,
    /// thrown toward the nearest player) after admiring it, loses it when hit, and equips or
    /// keeps anything else. A currency stack picked up from the ground is admired, and so
    /// bartered, only when it is also in `#minecraft:piglin_loved`, as vanilla admires loved
    /// items only; otherwise the piglin equips or keeps it. Only the `ItemPart`s attached to
    /// `stack`'s item are asked, on both sides, so register the part on both for the client's
    /// prediction to agree with the server: answer from the stack alone, and fast. An item with
    /// no such part costs one table lookup. The contract beyond the method's name and
    /// parameters is Horizon's reading, not checked against NeoForge's.
    #[hook(
        side = "both",
        contract = "listen",
        scope = "item",
        key = "stack",
        host = "read",
        neoforge = "IItemExtension.isPiglinCurrency"
    )]
    fn is_piglin_currency();
}
