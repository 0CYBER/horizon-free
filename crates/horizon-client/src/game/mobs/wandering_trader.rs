//! `WanderingTraderSpawner`: every 1200 ticks the spawn delay counts down;
//! when it runs out a trader may appear near the player (or the nearest
//! bell), bringing two llamas on leads, and wanders off after two days.
use super::{Dim, Game, creatures::Creatures};
use horizon_client::save::{read_dat, write_dat};

pub(super) use horizon_gameplay::wandering_trader::Spawner;

impl Creatures {
    /// A fresh spawner for a newly opened world.
    pub(super) fn reset_trader(&mut self) {
        self.trader = Spawner::default();
    }
}
impl Game<'_> {
    fn trader_path(&self) -> std::path::PathBuf {
        self.world_root
            .join("data")
            .join("minecraft")
            .join("wandering_trader.dat")
    }
    fn load_trader_data(&mut self) {
        if self.creatures.trader.is_loaded() {
            return;
        }
        let tag = read_dat(&self.trader_path());
        self.creatures
            .trader
            .load(tag.as_ref().and_then(|t| t.get("data")));
    }
    pub(super) fn save_wandering_trader(&mut self) {
        let Some(tag) = self.creatures.trader.to_save() else {
            return;
        };
        if write_dat(&self.trader_path(), tag).is_ok() {
            self.creatures.trader.saved();
        }
    }
    /// `WanderingTraderSpawner.tick`: the data is loaded here, the rest runs in
    /// `horizon_gameplay::wandering_trader`.
    pub(super) fn tick_wandering_trader(&mut self) {
        if self.dim != Dim::Overworld
            || !self
                .ui
                .rules
                .bool_at(horizon_client::gamerules::vanilla::SPAWN_WANDERING_TRADERS)
        {
            return;
        }
        self.load_trader_data();
        let mut trader = std::mem::take(&mut self.creatures.trader);
        trader.tick(self);
        self.creatures.trader = trader;
    }
}
