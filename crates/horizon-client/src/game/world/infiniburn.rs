//! Fire sources are resolved when a World receives its dimension facts or new tags.
use super::*;
use horizon_world::level::data::BlockSet;

impl Game<'_> {
    /// World data-pack reloads rebuild `Items.block_tags`, not the shared `Behaviours`
    /// tables. Both server and client therefore resolve from the items snapshot they play
    /// with, translating block names through the shared state table only at resolution.
    pub(super) fn resolve_level_infiniburn(
        &self,
        facts: &horizon_world::dimension_type::DimensionFacts,
    ) -> Arc<BlockSet> {
        let name = facts
            .infiniburn
            .strip_prefix('#')
            .unwrap_or(&facts.infiniburn);
        let mut set = BlockSet::default();
        if let Some(members) = self.items().block_tags.get(name) {
            for member in members {
                if let Some(block) = self.res.behaviours().table.block_by_name(member) {
                    set.insert(block);
                }
            }
        }
        Arc::new(set)
    }

    pub(super) fn refresh_infiniburn(&mut self) {
        self.world.infiniburn = Some(self.resolve_level_infiniburn(self.dimension_type()));
        let parked: Vec<_> = self
            .levels
            .parked
            .keys()
            .copied()
            .map(|dim| {
                (
                    dim,
                    self.resolve_level_infiniburn(self.dimension_type_of(dim)),
                )
            })
            .collect();
        for (dim, set) in parked {
            self.levels.parked.get_mut(&dim).unwrap().world.infiniburn = Some(set);
        }
    }

    /// An opened world's level becomes current: its `World` was installed after the tables
    /// were built, so its fire sources resolve now, against the tags just installed.
    pub(in crate::game) fn enter_opened_dim(&mut self, dim: Dim) {
        self.dim = dim;
        self.refresh_infiniburn();
    }

    /// The new remote World and its connection's dimension facts are now both installed.
    pub(super) fn finish_remote_level(&mut self) {
        self.refresh_infiniburn();
        self.apply_settings();
    }
}
