use super::*;

/// The hook points. Each does nothing in Horizon.
impl Game<'_> {
    /// Two animals bred and a baby is about to be made (NeoForge's
    /// `BabyEntitySpawnEvent`, vanilla's `Animal.spawnChildFromBreeding`):
    /// `parent` and `partner` are entity ids, `player` and `player_entity_id`
    /// the UUID and entity id of the player who fed them into love
    /// (`getCausedByPlayer`, None when none), `kind` the baby's type (a horse
    /// and a donkey have a mule). NeoForge's `setChild` (another baby)
    /// needs a handle to a mob not yet in the world, which Horizon doesn't
    /// have yet. Set `cancel` for no baby: the
    /// parents still rest and lose their love (their ages were already set
    /// to 6000), but there is no baby, no experience, no `animals_bred`
    /// statistic, no advancement and no heart particles. Not fired for
    /// turtles, frogs and sniffers (an egg, no baby) or armadillos' own paths.
    /// Villagers do not post this animal-only event, matching NeoForge's
    /// documented `Animal.spawnChildFromBreeding` posting site; a rejected
    /// villager join instead leaves the child's bed free. On the server. API 395.
    pub(in crate::game) fn native_baby_entity_spawn(
        &mut self,
        parent: u32,
        partner: u32,
        player: Option<u128>,
        player_entity_id: Option<i32>,
        kind: &str,
        cancel: &mut bool,
    ) {
        self.fire_baby_entity_spawn(parent, partner, player, player_entity_id, kind, cancel);
    }
}
