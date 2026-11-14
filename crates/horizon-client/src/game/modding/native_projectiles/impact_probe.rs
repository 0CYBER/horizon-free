//! Shared listener and resource fixture for independent projectile impact tests.
use crate::game::{
    Game,
    native_entity_refs::tests::{bind_local_player, with_game},
};
use horizon_client::{chunk::Chunk, items::ItemStack, motion::PlayerMotion};
use horizon_core::{geometry::Vec3, position::ChunkPos};
use mod_api::{Flow, Hooks, ServerHost};
use std::sync::{
    Arc, Mutex,
    atomic::{AtomicUsize, Ordering},
};

/// The listener action exercised by a tick.
#[derive(Clone, Copy)]
pub(in crate::game) enum Action {
    Allow,
    Cancel,
    Discard,
    Remove,
}

/// The independently observable event payload.
#[derive(Debug)]
pub(in crate::game) struct Seen {
    pub projectile: u32,
    pub kind: String,
    pub owner: u32,
    pub dimension: String,
    pub entity: Option<u32>,
    pub entity_kind: String,
    pub block: [i32; 3],
    pub face: [i32; 3],
    pub at: [f64; 3],
}

/// Listener observations, including any incorrectly triggered catch.
#[derive(Default)]
pub(in crate::game) struct Observed {
    pub hits: Mutex<Vec<Seen>>,
    pub catches: AtomicUsize,
}

struct Probe {
    action: Action,
    hears: bool,
    observed: Arc<Observed>,
}

#[mod_api::__subscriptions]
impl Hooks for Probe {
    fn hears_projectile_impacts(
        &self,
        _: &dyn mod_api::Host,
        event: &mut mod_api::HearsProjectileImpactsEvent,
    ) {
        event.hears = self.hears;
    }

    fn projectile_impact(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::ProjectileImpactEvent<'_>,
    ) -> Flow {
        let projectile = host
            .entity(event.projectile)
            .expect("impact projectile resolves");
        self.observed.hits.lock().unwrap().push(Seen {
            projectile: event.projectile,
            kind: event.kind.to_owned(),
            owner: event.owner,
            dimension: event.dimension.to_owned(),
            entity: event.entity,
            entity_kind: event.entity_kind.to_owned(),
            block: event.block,
            face: event.face,
            at: event.at,
        });
        match self.action {
            Action::Allow => Flow::Continue,
            Action::Cancel => Flow::Cancel,
            Action::Discard | Action::Remove => {
                let removed = match self.action {
                    Action::Discard => projectile.discard(host),
                    _ => projectile.remove(host, mod_api::RemovalReason::Killed),
                };
                assert!(removed);
                assert!(host.entity(event.projectile).is_none());
                Flow::Continue
            }
        }
    }

    fn item_fished(&self, _: &mut dyn ServerHost, _: &mut mod_api::ItemFishedEvent<'_>) -> Flow {
        self.observed.catches.fetch_add(1, Ordering::Relaxed);
        Flow::Continue
    }
}

/// Run a real server tick fixture with a live player and listener installed.
pub(in crate::game) fn with_probe(
    action: Action,
    hears: bool,
    test: impl FnOnce(&mut Game<'_>, &Observed),
) {
    let observed = Arc::new(Observed::default());
    let mods = vec![mod_api::ModEntry::new(
        "impact-probe",
        Box::new(Probe {
            action,
            hears,
            observed: observed.clone(),
        }),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            game.options.dedicated = Some(Vec::new());
            game.world
                .chunks
                .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
            game.block_ticks
                .add_empty_chunk(ChunkPos::new(0, 0))
                .unwrap();
            game.build_registries();
            game.set_allow_commands(true);
            assert!(game.run_command("forceload add 0 0"));
            game.player = Some(PlayerMotion::new(game.motion, Vec3::new(0.5, 64.0, 8.5)).unwrap());
            bind_local_player(game, 7, 7);
            game.selected = 0;
            game.inventory[0] = Some(ItemStack::new("minecraft:fishing_rod", 1));
            game.next_entity = 100;
            test(game, &observed);
        })
    });
}

/// Compare a geometric point against an independently specified expectation.
pub(in crate::game) fn point(actual: [f64; 3], expected: [f64; 3]) {
    for axis in 0..3 {
        assert!(
            (actual[axis] - expected[axis]).abs() < 1.0e-6,
            "{actual:?} != {expected:?}"
        );
    }
}
