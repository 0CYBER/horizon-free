//! Host method declarations; access defaults and game forwarders are generated.
#![allow(unused_variables)]
use crate::*;
/// The server host's methods of the `boss_bars` area, implemented by the game and reached through [`ServerHost`].
#[horizon_mod_api_macros::host_area]
pub trait BossBarsServerHost {
    /// Side: server.
    /// An existing custom boss bar by id (`minecraft:` when no namespace), or
    /// None. Saved bars are there from the world's start, as vanilla's
    /// `CustomBossEvents`. Boss bars of mobs and raids are not custom boss
    /// events.
    #[access(WorldOnly)]
    fn boss_bar(&self, id: &str) -> Option<BossBarRef>;
    /// Side: server.
    /// Every custom boss bar, in the order `/bossbar list` shows them.
    #[access(WorldOnly)]
    fn boss_bars(&self) -> Vec<BossBarRef>;
    /// Side: server.
    /// `/bossbar add`: a new bar, visible, white, `progress`, at 0 of 100, with
    /// nobody added. None if the id is taken or invalid, or outside a server.
    #[access(WorldOnly)]
    /// Refuses text deeper than 512 component nodes before applying any changes.
    fn create_boss_bar(
        &mut self,
        id: &str,
        name: Text,
    ) -> Result<Option<BossBarRef>, TextFormError>;
}
/// The handles' reads of the `boss_bars` area, implemented by the game and reached through [`Backend`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait BossBarsBackend {
    /// Side: server.
    /// The bar's id (`CustomBossEvent.customId`), or None for a stale handle.
    #[access(WorldOnly)]
    fn boss_bar_id(&self, bar: BossBarRef) -> Option<String>;
    /// Side: server.
    /// `BossEvent.getName`.
    #[access(WorldOnly)]
    /// Refuses text deeper than 512 component nodes before applying any changes.
    fn boss_bar_name(&self, bar: BossBarRef) -> Result<Option<Text>, TextFormError>;
    /// Side: server.
    /// `BossEvent.getColor`.
    #[access(WorldOnly)]
    fn boss_bar_color(&self, bar: BossBarRef) -> Option<BossBarColor>;
    /// Side: server.
    /// `BossEvent.getOverlay`.
    #[access(WorldOnly)]
    fn boss_bar_overlay(&self, bar: BossBarRef) -> Option<BossBarOverlay>;
    /// Side: server.
    /// `BossEvent.shouldDarkenScreen`.
    #[access(WorldOnly)]
    fn boss_bar_darken_screen(&self, bar: BossBarRef) -> Option<bool>;
    /// Side: server.
    /// `BossEvent.shouldPlayBossMusic`.
    #[access(WorldOnly)]
    fn boss_bar_play_boss_music(&self, bar: BossBarRef) -> Option<bool>;
    /// Side: server.
    /// `BossEvent.shouldCreateWorldFog`.
    #[access(WorldOnly)]
    fn boss_bar_create_world_fog(&self, bar: BossBarRef) -> Option<bool>;
    /// Side: server.
    /// `CustomBossEvent.value`.
    #[access(WorldOnly)]
    fn boss_bar_value(&self, bar: BossBarRef) -> Option<i32>;
    /// Side: server.
    /// `CustomBossEvent.max`.
    #[access(WorldOnly)]
    fn boss_bar_max(&self, bar: BossBarRef) -> Option<i32>;
    /// Side: server.
    /// `ServerBossEvent.isVisible`.
    #[access(WorldOnly)]
    fn boss_bar_visible(&self, bar: BossBarRef) -> Option<bool>;
    /// Side: server.
    /// `ServerBossEvent.getPlayers`: the bar's players who are online.
    #[access(WorldOnly)]
    fn boss_bar_players(&self, bar: BossBarRef) -> Option<Vec<PlayerRef>>;
}
/// The handles' writes of the `boss_bars` area, implemented by the game and reached through [`BackendMut`].
// hidden: a handle's backend, reached through `Backend`; mods call the handle, never this trait
#[doc(hidden)]
#[horizon_mod_api_macros::host_area]
pub trait BossBarsBackendMut {
    /// Side: server.
    /// `ServerBossEvent.setName`: viewers are sent the new name.
    #[access(WorldOnly)]
    /// Refuses text deeper than 512 component nodes before applying any changes.
    fn boss_bar_set_name(&mut self, bar: BossBarRef, name: Text) -> Result<bool, TextFormError>;
    /// Side: server.
    /// `ServerBossEvent.setColor`.
    #[access(WorldOnly)]
    fn boss_bar_set_color(&mut self, bar: BossBarRef, color: BossBarColor) -> bool;
    /// Side: server.
    /// `ServerBossEvent.setOverlay`.
    #[access(WorldOnly)]
    fn boss_bar_set_overlay(&mut self, bar: BossBarRef, overlay: BossBarOverlay) -> bool;
    /// Side: server.
    /// `ServerBossEvent.setDarkenScreen`.
    #[access(WorldOnly)]
    fn boss_bar_set_darken_screen(&mut self, bar: BossBarRef, darken: bool) -> bool;
    /// Side: server.
    /// `ServerBossEvent.setPlayBossMusic`.
    #[access(WorldOnly)]
    fn boss_bar_set_play_boss_music(&mut self, bar: BossBarRef, music: bool) -> bool;
    /// Side: server.
    /// `ServerBossEvent.setCreateWorldFog`.
    #[access(WorldOnly)]
    fn boss_bar_set_create_world_fog(&mut self, bar: BossBarRef, fog: bool) -> bool;
    /// Side: server.
    /// `CustomBossEvent.setValue`: stored as given, as vanilla does; only the
    /// progress shown is kept between empty and full.
    #[access(WorldOnly)]
    fn boss_bar_set_value(&mut self, bar: BossBarRef, value: i32) -> bool;
    /// Side: server.
    /// `CustomBossEvent.setMax`: stored as given, and the value is kept. Only
    /// `/bossbar set max` limits it to 1 or more.
    #[access(WorldOnly)]
    fn boss_bar_set_max(&mut self, bar: BossBarRef, max: i32) -> bool;
    /// Side: server.
    /// `ServerBossEvent.setVisible`: a hidden bar is taken off its players'
    /// screens and keeps them.
    #[access(WorldOnly)]
    fn boss_bar_set_visible(&mut self, bar: BossBarRef, visible: bool) -> bool;
    /// Side: server.
    /// `CustomBossEvent.addPlayer`. False for a stale bar or an offline or
    /// stale player. The bar is kept for the player across sessions.
    #[access(WorldOnly)]
    fn boss_bar_add_player(&mut self, bar: BossBarRef, player: PlayerRef) -> bool;
    /// Side: server.
    /// `CustomBossEvent.removePlayer`. False for a stale bar or an offline or
    /// stale player.
    #[access(WorldOnly)]
    fn boss_bar_remove_player(&mut self, bar: BossBarRef, player: PlayerRef) -> bool;
    /// Side: server.
    /// `/bossbar remove` (`CustomBossEvents.remove`): the bar's viewers lose it
    /// and its handle goes stale.
    #[access(WorldOnly)]
    fn boss_bar_remove(&mut self, bar: BossBarRef) -> bool;
}
