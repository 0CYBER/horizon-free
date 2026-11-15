//! A level's custom spawners (`ServerLevel.customSpawners`) and their tick
//! (`ServerLevel.tickCustomSpawners`): vanilla's, minus what a mod's
//! `modify_custom_spawners` took out, plus the mod spawners it added.
use super::Game;
use mod_api::{CustomSpawners, SpawnerEntry, SpawnerKind};

impl Game<'_> {
    /// The level's list, built (and `modify_custom_spawners` fired) on first use.
    fn level_spawners(&mut self) -> &CustomSpawners {
        let dim = self.dim;
        if !self.creatures.custom_spawners.contains_key(&dim) {
            let mut list = CustomSpawners::vanilla();
            if mod_api::__private::listened(mod_api::HookId::ModifyCustomSpawners) {
                self.native_modify_custom_spawners(dim.id(), &mut list);
            }
            self.creatures.custom_spawners.insert(dim, list);
        }
        &self.creatures.custom_spawners[&dim]
    }

    /// Whether vanilla's `kind` is still in this level's list.
    pub(super) fn has_custom_spawner(&mut self, kind: SpawnerKind) -> bool {
        self.level_spawners().contains(&SpawnerEntry::Builtin(kind))
    }

    /// `ServerLevel.tickCustomSpawners`: every spawner in the list, in order. A vanilla
    /// spawner keeps the place the game ticks it today (the phantom spawner runs with
    /// the stats, after the mobs: `spawn_phantoms`); a mod's spawner is the
    /// `custom_spawner_tick` hook for its key.
    pub(super) fn tick_custom_spawners(&mut self) {
        self.level_spawners();
        let dim = self.dim;
        let spawn_enemies = self
            .ui
            .rules
            .bool_at(horizon_client::gamerules::vanilla::SPAWN_MONSTERS)
            && self.ui.difficulty != 0;
        let mut at = 0;
        while let Some(entry) = self.creatures.custom_spawners[&dim]
            .entries()
            .get(at)
            .cloned()
        {
            at += 1;
            match entry {
                SpawnerEntry::Builtin(SpawnerKind::WanderingTrader) => self.tick_wandering_trader(),
                SpawnerEntry::Builtin(SpawnerKind::Patrol) => self.tick_patrol_spawner(),
                SpawnerEntry::Builtin(SpawnerKind::Cat) => self.tick_cat_spawner(),
                SpawnerEntry::Builtin(SpawnerKind::VillageSiege) => self.tick_village_siege(),
                SpawnerEntry::Mod(key) => {
                    self.native_custom_spawner_tick(dim.id(), key, spawn_enemies)
                }
                // `Phantom` ticks in `spawn_phantoms`.
                _ => {}
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_game;
    use super::*;
    use mod_api::SpawnerKey;
    use std::sync::{Arc, Mutex};

    type Log = Arc<Mutex<Vec<String>>>;

    /// A mod that takes the cat spawner out and adds spawners `<name>:swarm`,
    /// and logs every `custom_spawner_tick` and `modify_custom_spawners` it hears.
    struct Spawns {
        name: &'static str,
        log: Log,
    }
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Spawns {
        fn modify_custom_spawners(
            &self,
            _: &dyn mod_api::Host,
            event: &mut mod_api::ModifyCustomSpawnersEvent<'_>,
        ) {
            self.log
                .lock()
                .unwrap()
                .push(format!("{} modify", self.name));
            event
                .spawners
                .remove(&SpawnerEntry::Builtin(SpawnerKind::Cat));
            event
                .spawners
                .add(SpawnerKey::new(&format!("{}:swarm", self.name)).unwrap());
        }
        fn custom_spawner_tick(
            &self,
            _: &mut dyn mod_api::ServerHost,
            event: &mut mod_api::CustomSpawnerTickEvent<'_>,
        ) {
            self.log
                .lock()
                .unwrap()
                .push(format!("{} sees {}", self.name, event.key.as_str()));
        }
    }

    /// With no listener the list is vanilla's and every spawner ticks: the cat
    /// spawner counts its clock down. With two mods, `modify_custom_spawners`
    /// fires once per level, the cat spawner never ticks, and the mods' spawners
    /// tick in list order (the order they were added), each asked of every mod
    /// in dependency order.
    #[test]
    #[ignore = "daily"]
    fn custom_spawners_tick_in_list_order_and_a_removed_one_never_ticks() {
        with_game(|game| {
            game.creatures.cat_tick = 5;
            game.tick_custom_spawners();
            assert_eq!(
                game.creatures.cat_tick, 4,
                "no listener: vanilla's cat spawner ticks"
            );
            assert_eq!(
                game.creatures.custom_spawners[&game.dim],
                CustomSpawners::vanilla()
            );
        });
        let log = Log::default();
        let mods = ["amod", "bmod"]
            .map(|name| {
                mod_api::ModEntry::new(
                    name,
                    Box::new(Spawns {
                        name,
                        log: log.clone(),
                    }),
                )
            })
            .into();
        mod_platform::testing::with_mods(mods, || {
            with_game(|game| {
                game.creatures.cat_tick = 5;
                game.tick_custom_spawners();
                game.tick_custom_spawners();
                assert_eq!(
                    game.creatures.cat_tick, 5,
                    "the removed cat spawner never ticks"
                );
            });
        });
        let log = log.lock().unwrap();
        assert_eq!(
            log[..2],
            ["amod modify", "bmod modify"],
            "one modify event per level, in dependency order"
        );
        assert_eq!(log.iter().filter(|l| l.ends_with("modify")).count(), 2);
        let ticks: Vec<&str> = log[2..].iter().map(String::as_str).collect();
        let once = [
            "amod sees amod:swarm",
            "bmod sees amod:swarm",
            "amod sees bmod:swarm",
            "bmod sees bmod:swarm",
        ];
        assert_eq!(ticks, [once, once].concat());
    }
}
