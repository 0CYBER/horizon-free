//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The server host's methods of the `minecarts` area, implemented by the game and reached through [`ServerHost`].
#[horizon_mod_api_macros::host_area]
pub trait MinecartsServerHost {
    /// Side: server.
    /// Narrows an entity to a minecart of any kind, or None for another kind of
    /// entity, a stale handle, a client, or an entity held out of storage.
    #[access(WorldOnly)]
    fn minecart(&self, entity: EntityRef) -> Option<MinecartRef>;
}
/// The handles' reads of the `minecarts` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait MinecartsBackend {
    /// Side: server.
    /// A furnace minecart's push (`MinecartFurnace.push`): the way its fuel drives it on rails,
    /// x and z; its y part stays between ticks and sets the push's length. None for another
    /// kind of cart.
    #[access(WorldOnly)]
    fn minecart_push(&self, cart: MinecartRef) -> Option<[f64; 3]>;
    /// Side: server.
    /// Which `AbstractMinecart` subclass the cart is.
    #[access(WorldOnly)]
    fn minecart_kind(&self, cart: MinecartRef) -> Option<MinecartKind>;
    /// Side: server.
    /// `AbstractMinecart.getMaxSpeed`, in blocks per tick: the old behaviour's
    /// 0.4 (0.2 in water), or with the `minecart_improvements` experiment the
    /// `max_minecart_speed` rule over 20 (halved in water). A furnace minecart
    /// takes half of that, three quarters in water (`MinecartFurnace`).
    #[access(WorldOnly)]
    fn minecart_max_speed(&self, cart: MinecartRef) -> Option<f64>;
    /// Side: server.
    /// `AbstractMinecart.isOnRails`.
    #[access(WorldOnly)]
    fn minecart_is_on_rails(&self, cart: MinecartRef) -> Option<bool>;
    /// Side: server.
    /// `AbstractMinecart.isFlipped`.
    #[access(WorldOnly)]
    fn minecart_flipped(&self, cart: MinecartRef) -> Option<bool>;
    /// Side: server.
    /// `VehicleEntity.getDamage`, the wobble damage (`DATA_ID_DAMAGE`).
    #[access(WorldOnly)]
    fn minecart_damage(&self, cart: MinecartRef) -> Option<f32>;
    /// Side: server.
    /// `VehicleEntity.getHurtTime`, the wobble ticks (`DATA_ID_HURT`).
    #[access(WorldOnly)]
    fn minecart_hurt_time(&self, cart: MinecartRef) -> Option<i32>;
    /// Side: server.
    /// `AbstractMinecart.customDisplayBlockState`: the block the cart shows in
    /// place of its kind's default, None (inner) when it has none.
    #[access(WorldOnly)]
    fn minecart_custom_display_block(&self, cart: MinecartRef) -> Option<Option<BlockState>>;
    /// Side: server.
    /// `AbstractMinecart.getDisplayOffset`, in sixteenths of a block: the set
    /// offset, else the kind's default.
    #[access(WorldOnly)]
    fn minecart_display_offset(&self, cart: MinecartRef) -> Option<i32>;
    /// Side: server.
    /// `MinecartFurnace`'s fuel ticks; None for another kind of cart.
    #[access(WorldOnly)]
    fn minecart_fuel(&self, cart: MinecartRef) -> Option<i32>;
    /// Side: server.
    /// `MinecartTNT`'s fuse ticks, -1 when it is not primed; None for another
    /// kind of cart.
    #[access(WorldOnly)]
    fn minecart_fuse(&self, cart: MinecartRef) -> Option<i32>;
    /// Side: server.
    /// `MinecartHopper.isEnabled`; None for another kind of cart.
    #[access(WorldOnly)]
    fn minecart_hopper_enabled(&self, cart: MinecartRef) -> Option<bool>;
    /// Side: server.
    /// A command block minecart's command (`MinecartCommandBlock.getCommandBlock().getCommand()`).
    /// None for another kind of cart.
    #[access(WorldOnly)]
    fn minecart_command(&self, cart: MinecartRef) -> Option<String>;
}
/// The handles' writes of the `minecarts` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait MinecartsBackendMut {
    /// Side: server.
    /// Sets a furnace minecart's push, as a player's click with fuel sets it toward them
    /// (`MinecartFurnace.interact`): a cart a mod drives runs that way while it has fuel. False
    /// for another kind of cart or a push that isn't finite.
    #[access(WorldOnly)]
    fn minecart_set_push(&mut self, cart: MinecartRef, push: [f64; 3]) -> bool;
    /// Side: server.
    /// `AbstractMinecart.setFlipped`; saved as `FlippedRotation`, not synced.
    #[access(WorldOnly)]
    fn minecart_set_flipped(&mut self, cart: MinecartRef, flipped: bool) -> bool;
    /// Side: server.
    /// `VehicleEntity.setDamage`: synced to clients. The tick counts it down
    /// only while positive.
    #[access(WorldOnly)]
    fn minecart_set_damage(&mut self, cart: MinecartRef, damage: f32) -> bool;
    /// Side: server.
    /// `VehicleEntity.setHurtTime`: synced to clients.
    #[access(WorldOnly)]
    fn minecart_set_hurt_time(&mut self, cart: MinecartRef, ticks: i32) -> bool;
    /// Side: server.
    /// Sets the block the cart shows, or with None its kind's default again:
    /// saved as `DisplayState` and synced to clients. False for a state the
    /// registry doesn't know, as the loader drops one that does not decode.
    #[access(WorldOnly)]
    fn minecart_set_custom_display_block(
        &mut self,
        cart: MinecartRef,
        state: Option<BlockState>,
    ) -> bool;
    /// Side: server.
    /// `AbstractMinecart.setDisplayOffset`, in sixteenths of a block: saved as
    /// `DisplayOffset` (only off the kind's default) and synced to clients.
    #[access(WorldOnly)]
    fn minecart_set_display_offset(&mut self, cart: MinecartRef, offset: i32) -> bool;
    /// Side: server.
    /// Sets a furnace minecart's fuel ticks, any int as vanilla's field holds
    /// it (saved as a short, the low 16 bits). False for another kind of cart.
    /// The lit flag (`DATA_ID_FUEL`) follows with fuel above zero.
    #[access(WorldOnly)]
    fn minecart_set_fuel(&mut self, cart: MinecartRef, fuel: i32) -> bool;
    /// Side: server.
    /// `MinecartTNT.primeFuse(null)`: an 80 tick fuse, the primed sound and
    /// entity event 70 for clients. False for another kind of cart, or with
    /// the `tnt_explodes` rule off.
    #[access(WorldOnly)]
    fn minecart_prime_fuse(&mut self, cart: MinecartRef) -> bool;
    /// Side: server.
    /// `MinecartHopper.setEnabled`: saved as `Enabled`, not synced.
    #[access(WorldOnly)]
    fn minecart_set_hopper_enabled(&mut self, cart: MinecartRef, enabled: bool) -> bool;
    /// Side: server.
    /// Sets a command block minecart's command (`BaseCommandBlock.setCommand`, which resets the
    /// success count), then `MinecartCommandBase.onUpdated`, as the edit screen's save does: the
    /// synced command and last output follow, so clients' editors show the new command. The
    /// output and its tracking are kept, and no chat note is sent. An activator rail then runs
    /// the new command. False for another kind of cart.
    #[access(WorldOnly)]
    fn minecart_set_command(&mut self, cart: MinecartRef, command: &str) -> bool;
}
