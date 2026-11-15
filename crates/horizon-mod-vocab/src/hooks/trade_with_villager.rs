//! The `trade_with_villager` hook: declarations the mod API generator reads (see `hooks/mod.rs`).
// catalog-order: 0338

pub trait Hooks {
    /// A trade of a villager or a wandering trader was made
    /// (`MerchantResultSlot.onTake` once the offer is taken): `merchant` is
    /// the trader's entity id and `kind` its type (`minecraft:villager`,
    /// `minecraft:wandering_trader`), `offer` the live one in the trading
    /// session (not a copy) as the trade left it: `uses` counted, `result`
    /// what the player got. Once per trade, so a shift-click that trades
    /// several times is heard for each, after the trade's statistic and
    /// `villager_trade` trigger. A mod may change the offer (Bukkit's
    /// `PlayerTradeEvent#getTrade` is mutable): `uses`, `max_uses`, `demand`,
    /// `special_price_diff`, `price_multiplier`, `xp`. The change stands: the
    /// next click and the offers sent to the player see it. Not asked for a
    /// trade window without a trader. The player's state is swapped in
    /// (`host.players().acting_player()`). Server-side. Version 2: was a listen hook
    /// returning the offer read-only; now collects the live offer as `out`
    /// (the moment upstream's separate `merchant_offer_traded` covered).
    #[hook(
        side = "server",
        contract = "collect",
        out = "offer",
        neoforge = "TradeWithVillagerEvent",
        cancellable = false,
        version = 2
    )]
    fn trade_with_villager();
}
