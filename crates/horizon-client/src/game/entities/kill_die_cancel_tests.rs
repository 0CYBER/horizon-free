//! `/kill` runs death before returning. A typed server hook can veto it;
//! a later allowed kill dies exactly once.
use super::kill_die_vibration_tests::{PIG, deaths, run};
use super::*;
use horizon_client::chunk::Chunk;
use horizon_core::position::ChunkPos;
use mod_api::{Hooks, LivingDeathEvent, ModEntry, ServerHost};
use mod_platform::testing::with_mods;
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

#[derive(Default)]
struct Probe {
    calls: Mutex<Vec<(u32, String)>>,
    veto: AtomicBool,
}
struct ProbeMod(Arc<Probe>);
#[mod_api::__subscriptions]
impl Hooks for ProbeMod {
    fn living_death(
        &self,
        host: &mut dyn ServerHost,
        e: &mut LivingDeathEvent<'_>,
    ) -> mod_api::Flow {
        self.0
            .calls
            .lock()
            .unwrap()
            .push((e.entity, e.source.damage_type.to_owned()));
        if self.0.veto.load(Ordering::Relaxed) {
            let living = host.living(host.entity(e.entity).unwrap()).unwrap();
            assert!(living.set_health(host, 5.0));
            mod_api::Flow::Cancel
        } else {
            mod_api::Flow::Continue
        }
    }
}

fn porkchops(game: &Game<'_>) -> usize {
    game.dropped
        .iter()
        .filter(|i| &*i.stack.item == "minecraft:porkchop")
        .count()
}

#[test]
#[ignore = "daily"]
fn a_vetoed_kill_keeps_the_mob_and_a_later_kill_dies_once() {
    crate::game::in_own_process(
        "game::kill_die_cancel_tests::a_vetoed_kill_keeps_the_mob_and_a_later_kill_dies_once",
        a_vetoed_kill_keeps_the_mob_and_a_later_kill_dies_once_isolated,
    );
}

fn a_vetoed_kill_keeps_the_mob_and_a_later_kill_dies_once_isolated() {
    let probe = Arc::new(Probe::default());
    let mods: mod_api::Mods = vec![ModEntry::new(
        "kill-death-probe",
        Box::new(ProbeMod(probe.clone())),
    )]
    .into();
    with_mods(mods, || {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.build_registries();
            game.load_server_loot();
            game.set_allow_commands(true);
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            assert!(game.run_command("fill 0 99 0 15 99 15 stone"));
            game.sculk_prepare();
            assert!(game.run_command("setblock 8 100 8 sculk_sensor"));
            game.sculk_sync_index();
            assert!(game.run_command("summon pig 5.5 100 8.5 {NoAI:1b,NoGravity:1b,PersistenceRequired:1b,UUID:[I;593,0,0,1]}"));
            let pig = game
                .mobs
                .iter()
                .find(|m| m.species() == horizon_client::mobs::Species::Pig)
                .unwrap()
                .id;
            let calls = |p: &Probe| std::mem::take(&mut *p.calls.lock().unwrap());
            // Vetoed: the death is asked during the command and refused.
            probe.veto.store(true, Ordering::Relaxed);
            let posted = deaths(game, |g| assert!(run(g, &format!("kill {PIG}")).0));
            assert_eq!(calls(&probe).len(), 1, "the veto is asked during /kill");
            assert_eq!(posted, [], "no event for a vetoed death");
            let m = game.mobs.iter().find(|m| m.id == pig).unwrap();
            assert_eq!(
                (m.health, m.death_time),
                (5.0, 0),
                "listener's healing survives"
            );
            assert_eq!(porkchops(game), 0, "no loot");
            let later = deaths(game, |g| g.tick_creature_entities().unwrap());
            assert_eq!((calls(&probe).len(), later), (0, vec![]), "nothing more");
            assert!(game.mobs.iter().any(|m| m.id == pig && m.alive()));
            // Allowed: one call, one event, one loot drop, and no more next tick.
            probe.veto.store(false, Ordering::Relaxed);
            let posted = deaths(game, |g| assert!(run(g, &format!("kill {PIG}")).0));
            let allowed = calls(&probe);
            assert_eq!(allowed.iter().map(|c| c.0).collect::<Vec<_>>(), [pig]);
            assert_eq!(posted.len(), 1, "one death event");
            assert_eq!(porkchops(game), 1, "its loot, during the command");
            let again = deaths(game, |g| assert!(run(g, &format!("kill {PIG}")).0));
            let later = deaths(game, |g| g.tick_creature_entities().unwrap());
            assert_eq!(
                (calls(&probe).len(), again, later, porkchops(game)),
                (0, vec![], vec![], 1),
                "a second kill and the next tick do not die again"
            );
            let m = game.mobs.iter().find(|m| m.id == pig).unwrap();
            assert!(!m.removed && m.health == 0.0, "the body stays");
        });
    });
}
