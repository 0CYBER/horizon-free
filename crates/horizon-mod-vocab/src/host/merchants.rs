//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The server host's methods of the `merchants` area, implemented by the game and reached through [`ServerHost`].
#[horizon_mod_api_macros::host_area]
pub trait MerchantsServerHost {
    /// Side: server.
    /// Narrows an entity to a villager or wandering trader, or None for
    /// another kind, a stale handle, or a client.
    #[access(WorldOnly)]
    fn merchant(&self, entity: EntityRef) -> Option<MerchantRef>;
}
/// The handles' reads of the `merchants` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait MerchantsBackend {
    /// Side: server.
    /// `AbstractVillager.getOffers`, as they are now (a copy). Offers not
    /// rolled yet read as empty: they are rolled when a player first trades,
    /// or by `add_offer`; reading never draws a random.
    #[access(WorldOnly)]
    fn merchant_offers(&self, merchant: MerchantRef) -> Option<Vec<MerchantOffer>>;
    /// Side: server.
    /// `Merchant.getTradingPlayer`: the player whose trading screen is open on
    /// it. None when nobody trades, or the screen is the host window's.
    #[access(WorldOnly)]
    fn merchant_trading_player(&self, merchant: MerchantRef) -> Option<PlayerRef>;
    /// Side: server.
    /// `VillagerData.profession`, a registry id (`minecraft:farmer`; none is
    /// `minecraft:none`). None for a wandering trader.
    #[access(WorldOnly)]
    fn merchant_profession(&self, merchant: MerchantRef) -> Option<String>;
    /// Side: server.
    /// `VillagerData.level`, at least 1.
    #[access(WorldOnly)]
    fn merchant_level(&self, merchant: MerchantRef) -> Option<i32>;
    /// Side: server.
    /// `VillagerData.type`, a registry id (`minecraft:plains`).
    #[access(WorldOnly)]
    fn merchant_villager_type(&self, merchant: MerchantRef) -> Option<String>;
    /// Side: server.
    /// `Villager.getVillagerXp`.
    #[access(WorldOnly)]
    fn merchant_xp(&self, merchant: MerchantRef) -> Option<i32>;
}
/// The handles' writes of the `merchants` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait MerchantsBackendMut {
    /// Side: server.
    /// Replaces offer `index` (a player trading with it is sent the new
    /// offers). False for an index past the end, a stale handle, or an offer
    /// without a real item in `base_cost_a` or `result` (or `cost_b` when set).
    /// An invalid count, damage or component maximum refuses the whole write with a typed error.
    #[access(WorldOnly)]
    fn merchant_set_offer(
        &mut self,
        merchant: MerchantRef,
        index: u32,
        offer: MerchantOffer,
    ) -> Result<bool, StackWriteError>;
    /// Side: server.
    /// Appends an offer, as `getOffers().add(offer)` does. Offers not rolled
    /// yet are rolled first, as `getOffers` does for a player's first trade
    /// (this draws the trade set's world random sequence, as that does).
    /// False for a stale handle or an invalid offer (see `set_offer`).
    #[access(WorldOnly)]
    fn merchant_add_offer(
        &mut self,
        merchant: MerchantRef,
        offer: MerchantOffer,
    ) -> Result<bool, StackWriteError>;
    /// Side: server.
    /// Removes offer `index`, the ones after it moving up. False for an index
    /// past the end or a stale handle.
    #[access(WorldOnly)]
    fn merchant_remove_offer(&mut self, merchant: MerchantRef, index: u32) -> bool;
    /// Side: server.
    /// `Villager.setVillagerData` with a new profession: a different one
    /// forgets the offers, which roll again on the next trade. Clients see the
    /// new profession through entity data. False for an unknown id, a wandering
    /// trader, or a villager with a trading screen open.
    #[access(WorldOnly)]
    fn merchant_set_profession(&mut self, merchant: MerchantRef, profession: &str) -> bool;
    /// Side: server.
    /// `VillagerData.withLevel`: `Math.max(1, level)`, with no upper limit in
    /// vanilla. The offers stay as they are; the next level-up adds its trades
    /// (a level past 5 can't level up). A trading guest's session follows,
    /// and gets the sequences a level-up from the new level needs.
    #[access(WorldOnly)]
    fn merchant_set_level(&mut self, merchant: MerchantRef, level: i32) -> bool;
    /// Side: server.
    /// `VillagerData.withType`: also the type a trading guest's session
    /// rolls its next level-up offers with. False for an unknown id, a
    /// wandering trader, or a trade open where this can't reach.
    #[access(WorldOnly)]
    fn merchant_set_villager_type(&mut self, merchant: MerchantRef, kind: &str) -> bool;
    /// Side: server.
    /// `Villager.setVillagerXp`: assigned as given, as vanilla does.
    #[access(WorldOnly)]
    fn merchant_set_xp(&mut self, merchant: MerchantRef, xp: i32) -> bool;
}
