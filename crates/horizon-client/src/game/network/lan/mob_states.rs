//! Mob metadata and live attributes, built on the owning server thread.
//! With multiple tick lanes, metadata is deferred until after the snapshot pass; it is still
//! built serially because `Mob` contains non-`Sync` interior-mutable state.
use super::*;
use horizon_client::mobs::villager::ResolvedProfessions;

/// A fixture large enough to exercise the former parallel-state threshold.
#[cfg(test)]
const MIN_MOBS: usize = 1024;

/// One mob's entity data (`meta`, and the `defaults` its entries can go back to); whether its
/// position is sent precisely.
#[allow(clippy::too_many_arguments)]
fn mob_state(
    m: &Mob,
    sync: &SyncData,
    professions: &ResolvedProfessions,
    effects: Vec<(u8, i32, Value)>,
    player_entity: Option<i32>,
    game_time: i64,
    meta: &mut Vec<(u8, i32, Value)>,
    defaults: &mut Vec<(u8, i32, Value)>,
) -> bool {
    super::super::villager_client::server_meta_sync(
        m,
        sync,
        effects,
        (player_entity, game_time),
        meta,
        defaults,
        professions,
    );
    mob_freeze_meta(m, meta, defaults);
    // HappyGhast.aiStep: requiresPrecisePosition while on a still timeout.
    m.species() == horizon_client::mobs::Species::HappyGhast
        && horizon_client::mobs::happy_ghast::on_still_timeout(m)
}

#[cfg(test)]
thread_local! {
    /// A test's choice of deferred or immediate metadata construction.
    static DEFER: std::cell::Cell<Option<bool>> = const { std::cell::Cell::new(None) };
}

/// Whether metadata waits until after the snapshot pass.
fn deferring() -> bool {
    #[cfg(test)]
    if let Some(defer) = DEFER.with(std::cell::Cell::get) {
        return defer;
    }
    horizon_client::parallel::lanes() > 1
}

/// What a waiting mob needs besides the mob: (its slot in the states, its index in the mobs, the
/// player entity its lead may name, its effect entries, which the hub makes).
type Waiting = (usize, usize, Option<i32>, Vec<(u8, i32, Value)>);

/// The mobs' states of one pass, with metadata optionally deferred until after the pass.
pub(super) struct Later {
    waiting: Option<Vec<Waiting>>,
    game_time: i64,
    speeds: super::super::lan_attributes::SpeedByType,
}

impl Later {
    pub(super) fn new(game_time: i64) -> Self {
        Self {
            waiting: deferring().then(Vec::new),
            game_time,
            speeds: Default::default(),
        }
    }

    /// Mob `index`'s state `e`, in `slot` of the states: built now, or left for [`Self::fill`].
    /// The attributes are always built now.
    // `mob_attributes` can initialize `m.extra.live` through a shared read.
    // Its `std::cell::OnceCell` is not `Sync`; neither attributes nor metadata
    // may share `&Mob` with helper threads, even if the live map was built first.
    #[allow(clippy::too_many_arguments)]
    pub(super) fn state(
        &mut self,
        game: &Game<'_>,
        (slot, index): (usize, usize),
        m: &Mob,
        player_entity: Option<i32>,
        hub: &Hub,
        e: &mut EntityState,
    ) {
        let effects = mob_effect_meta(m, hub);
        if let Some(waiting) = &mut self.waiting {
            waiting.push((slot, index, player_entity, effects));
        } else {
            e.precise = mob_state(
                m,
                hub.sync_data(),
                &game.stations().professions,
                effects,
                player_entity,
                self.game_time,
                &mut e.meta,
                &mut e.defaults,
            );
        }
        game.mob_attributes(m, &mut self.speeds, &mut e.attributes);
    }

    /// The waiting metadata, built serially on the owning server thread and put in place.
    pub(super) fn fill(self, game: &Game<'_>, mobs: &[Mob], sync: &SyncData, out: &mut States) {
        let Some(waiting) = self.waiting else {
            return;
        };
        let (game_time, professions) = (self.game_time, &game.stations().professions);
        let states = out.as_mut_slice();
        for (slot, index, player, effects) in waiting {
            let e = &mut states[slot];
            e.precise = mob_state(
                &mobs[index],
                sync,
                professions,
                effects,
                player,
                game_time,
                &mut e.meta,
                &mut e.defaults,
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The states put in place after the pass are those built in place during it, for mobs whose
    /// entity data and attributes vary: effects, armor, a villager's profession, variants, a baby,
    /// a happy ghast, and what `/attribute` changed.
    #[test]
    #[ignore = "daily"]
    fn deferred_states_match_those_built_in_place() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            // The fixture stops before world-open command registry initialization.
            game.build_registries();
            game.set_allow_commands(true);
            for command in [
                "summon piglin 8.5 80 8.5 {NoAI:1b,PersistenceRequired:1b}",
                "effect give @e[type=piglin] speed 30 1",
                "item replace entity @e[type=piglin,limit=1] armor.head with iron_helmet",
                "summon horse 10.5 80 8.5 {NoAI:1b}",
                "summon sheep 12.5 80 8.5 {NoAI:1b,Color:3b,Age:-24000}",
                "summon axolotl 14.5 80 8.5 {NoAI:1b,Variant:2}",
                "summon villager 16.5 80 8.5 {NoAI:1b,VillagerData:{profession:\"minecraft:farmer\",level:2,type:\"minecraft:plains\"}}",
                "summon happy_ghast 18.5 84 8.5 {NoAI:1b}",
                "summon pig 20.5 80 8.5 {NoAI:1b}",
                "attribute @e[type=pig,limit=1] minecraft:scale base set 1.5",
            ] {
                assert!(game.run_command(command), "{command}");
            }
            let examples: Vec<Mob> = game.mobs.iter().cloned().collect();
            assert!(!examples.is_empty());
            game.mobs.clear();
            for i in 0..MIN_MOBS * 2 + 37 {
                let mut mob = examples[i % examples.len()].clone();
                mob.id = 10_000 + i as u32;
                mob.removed = i % 11 == 0;
                game.mobs.push(mob);
            }
            let settings = LanSettings {
                port: 0,
                guest_commands: false,
                force_game_mode: false,
            };
            let net = NetConfig {
                listen: false,
                announce: false,
                lod_distance: 0,
                ..NetConfig::lan(0, String::new(), 2)
            };
            let config = HubConfig {
                default_game_mode: 0,
                guest_commands: false,
                force_game_mode: false,
                hardcore: false,
                view_distance: 2,
                simulation_distance: 2,
                dimension: game.dim.id().to_owned(),
                allow_flight: true,
                seed: 0,
                max_players: 2,
                integrated: true,
                levels: game.level_shapes,
                level_seeds: game.levels.runtime.seeds(),
            };
            game.lan_host(settings, net, config).unwrap();
            struct Reset;
            impl Drop for Reset {
                fn drop(&mut self) {
                    DEFER.with(|d| d.set(None));
                }
            }
            let _reset = Reset;
            for after in [0, 10_012] {
                let mut runs = Vec::new();
                for defer in [false, true, false] {
                    DEFER.with(|d| d.set(Some(defer)));
                    let mut states = States::default();
                    game.lan_entities(&mut states, false, after, None);
                    runs.push(states.as_slice().to_vec());
                }
                let expected_ids: Vec<_> = game
                    .mobs
                    .iter()
                    .filter(|m| !m.removed && m.id > after)
                    .map(|m| m.id as i32)
                    .collect();
                assert_eq!(
                    runs[0].iter().map(|e| e.id).collect::<Vec<_>>(),
                    expected_ids
                );
                assert!(runs[0].iter().filter(|e| !e.meta.is_empty()).count() >= MIN_MOBS);
                assert!(runs[0].iter().any(|e| !e.attributes.is_empty()));
                assert_eq!(runs[0], runs[1], "deferred, after {after}");
                assert_eq!(runs[0], runs[2], "switch off, after {after}");
            }
        });
    }
}

#[cfg(test)]
#[path = "mob_swing_animation_tests.rs"]
mod mob_swing_animation_tests;
