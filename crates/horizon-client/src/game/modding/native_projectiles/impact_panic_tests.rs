use super::*;
use horizon_client::{
    chunk::Chunk,
    join_seq::residency::{
        Change, Journal, Kind, Owner,
        table::{EntityLookup, EntityTable, SourceEntity},
    },
};
use mod_api::{Flow, HearsProjectileImpactsEvent, Hooks, ProjectileImpactEvent, ServerHost};
use std::sync::{
    Arc,
    atomic::{AtomicBool, Ordering},
};

struct PanicAfterAdmission {
    admitted: Arc<AtomicBool>,
    panicked: Arc<AtomicBool>,
}
#[mod_api::__subscriptions]
impl Hooks for PanicAfterAdmission {
    fn hears_projectile_impacts(
        &self,
        _: &dyn mod_api::Host,
        event: &mut HearsProjectileImpactsEvent,
    ) {
        event.hears = true;
    }
    fn projectile_impact(
        &self,
        host: &mut dyn ServerHost,
        event: &mut ProjectileImpactEvent<'_>,
    ) -> Flow {
        assert!(host.entity(event.projectile).is_some());
        self.admitted.store(true, Ordering::SeqCst);
        Flow::Continue
    }
    fn vanilla_game_event(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::VanillaGameEventEvent<'_>,
    ) -> Flow {
        if event.event == "minecraft:projectile_shoot" {
            assert!(self.admitted.load(Ordering::SeqCst));
            let _ = host.summon(
                event.dimension,
                "minecraft:snowball",
                [9.5, 70.0, 0.5],
                Default::default(),
            );
            self.panicked.store(true, Ordering::SeqCst);
            panic!("listener panicked after impact admission");
        }
        Flow::Continue
    }
}

#[test]
#[ignore = "daily"]
fn shoot_listener_panic_after_admission_preserves_owned_rows() {
    eprintln!(
        "PANIC_GUARD_ASSERTIONS_DISABLED:{}",
        !cfg!(debug_assertions)
    );
    for dimension in [crate::game::Dim::Overworld, crate::game::Dim::Nether] {
        let admitted = Arc::new(AtomicBool::new(false));
        let panicked = Arc::new(AtomicBool::new(false));
        let mods = vec![mod_api::ModEntry::new(
            "panic-after-admission",
            Box::new(PanicAfterAdmission {
                admitted: admitted.clone(),
                panicked: panicked.clone(),
            }),
        )]
        .into();
        mod_platform::testing::with_mods(mods, || {
            crate::game::native_entity_refs::tests::with_game(|game| {
                game.enter_level(dimension).unwrap();
                game.options.dedicated = Some(Vec::new());
                game.world
                    .chunks
                    .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
                let stone = game
                    .pack
                    .assets
                    .registry
                    .default_state("minecraft:stone")
                    .unwrap();
                game.world.set(2, 64, 0, stone).unwrap();
                // Hook handles resolve through the game's canonical residency table.
                let journal = game.resident_entities.table.borrow().producer();
                let observation = journal.observe_structural_events();
                let owner = Owner {
                    dimension: game.dim.key(),
                    kind: Kind::Projectile,
                };
                game.creatures
                    .projectiles
                    .bind_residency(&journal, owner, |p| (p.id, p.join));
                for i in 0..3 {
                    let x = if i == 2 { 5.5 } else { 0.5 };
                    let kind = if i == 2 {
                        // The missing snowball survives and queues its first-shot event.
                        horizon_client::projectiles::ProjectileKind::Snowball
                    } else {
                        // Arrows stay alive after impact, so no row is terminal here.
                        horizon_client::projectiles::ProjectileKind::Arrow {
                            base_damage: 2.0,
                            critical: false,
                            effect: None,
                        }
                    };
                    let mut p = Projectile::new(kind, Vec3::new(x, 64.5, 0.5), None);
                    p.id = 71 + i;
                    p.velocity = Vec3::new(3.0, 0.0, 0.0);
                    game.creatures.projectiles.push(p);
                }
                let before: Vec<_> = game
                    .creatures
                    .projectiles
                    .iter()
                    .map(|p| (p.id, p.join))
                    .collect();
                let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    game.tick_projectiles().unwrap();
                }));
                assert!(result.is_err());
                assert!(admitted.load(Ordering::SeqCst));
                assert!(
                    panicked.load(Ordering::SeqCst),
                    "did not reach the post-admission listener"
                );
                // Observe without stealing events already consumed by hook queries.
                let mut events = Vec::new();
                observation.drain_into(&mut events);
                let mut depth = 0usize;
                for event in events {
                    assert_eq!(event.owner, owner);
                    match event.change {
                        Change::Lifted => depth += 1,
                        Change::Restored => {
                            depth = depth.checked_sub(1).expect("restore without lift")
                        }
                        _ => {}
                    }
                }
                assert_eq!(depth, 0, "listener panic abandoned the lifted pass");
                game.resident_entities
                    .table
                    .borrow_mut()
                    .end_tick()
                    .expect("listener panic abandoned the lifted pass");
                let restored: Vec<_> = game
                    .creatures
                    .projectiles
                    .iter()
                    .map(|p| (p.id, p.join))
                    .collect();
                assert_eq!(restored.len(), before.len() + 1);
                assert_eq!(
                    game.creatures.projectiles[0].position,
                    Vec3::new(9.5, 70.0, 0.5)
                );
                assert_eq!(
                    restored[1..],
                    before,
                    "panic changed survivor identity or spawn admission order"
                );
                let mut table = game.resident_entities.table.borrow_mut();
                for expected in restored {
                    let found = table.entity_lookup(&owner.dimension, expected.0, |at| {
                        let list = &game.creatures.projectiles;
                        let entity = &list[list.locate(at.join)?];
                        Some(SourceEntity {
                            id: entity.id,
                            value: (entity.id, entity.join),
                        })
                    });
                    let EntityLookup::Found { location, value } = found else {
                        panic!("restored row did not resolve: {expected:?}");
                    };
                    assert_eq!(location.owner, owner);
                    assert_eq!(value, expected);
                }
                table.end_tick().expect("panic left an abandoned operation");
            });
        });
    }
}

#[test]
#[ignore = "daily"]
fn normal_dispatch_returns_the_owned_pass_for_explicit_restore() {
    for dimension in [crate::game::Dim::Overworld, crate::game::Dim::Nether] {
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.enter_level(dimension).unwrap();
            let journal = Journal::default();
            let owner = Owner {
                dimension: game.dim.key(),
                kind: Kind::Projectile,
            };
            game.creatures
                .projectiles
                .bind_residency(&journal, owner, |p| (p.id, p.join));
            let mut original = Projectile::new(
                horizon_client::projectiles::ProjectileKind::Egg,
                Vec3::new(3.5, 80.0, 0.5),
                None,
            );
            original.id = 7;
            game.creatures.projectiles.push(original);
            let identity = (7, game.creatures.projectiles[0].join);
            let mut pass = game.creatures.projectiles.lift();
            let mut child = pass[0].clone();
            child.id = 8;
            child.join = horizon_client::join_seq::JoinSeq::UNJOINED;
            game.creatures.projectiles.push(child);
            let child_identity = (8, game.creatures.projectiles[0].join);
            game.with_projectile_dispatch(&mut pass, |game| {
                assert_eq!(
                    game.creatures
                        .projectiles
                        .iter()
                        .map(|p| (p.id, p.join))
                        .collect::<Vec<_>>(),
                    [child_identity]
                );
            });
            assert_eq!(
                pass.iter().map(|p| (p.id, p.join)).collect::<Vec<_>>(),
                [identity]
            );
            assert_eq!(
                game.creatures.projectiles.len(),
                1,
                "normal dispatch must not auto-restore"
            );
            game.creatures
                .projectiles
                .restore(pass, horizon_client::join_seq::RestoreOrder::SpawnsFirst);
            assert_eq!(
                game.creatures
                    .projectiles
                    .iter()
                    .map(|p| (p.id, p.join))
                    .collect::<Vec<_>>(),
                [child_identity, identity]
            );
            EntityTable::new(journal)
                .end_tick()
                .expect("normal dispatch changed the lift depth");
        });
    }
}
