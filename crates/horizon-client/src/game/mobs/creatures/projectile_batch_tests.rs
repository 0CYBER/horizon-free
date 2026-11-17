use super::*;
use horizon_client::{chunk::Chunk, join_seq::JoinSeq};
use horizon_mod_runtime::attachments::{
    AttachmentType, Attachments, HasAttachmentLifetime, Holder, index::AttachmentIndex,
};
use mod_api::{Flow, HearsProjectileImpactsEvent, Hooks, ProjectileImpactEvent, ServerHost};
use std::sync::{Arc, Mutex};
static VALUE: AttachmentType<i32> = AttachmentType {
    namespace: "test",
    name: "projectile_lifetime",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync: false,
};

struct BoundProjectiles {
    _values: [Attachments; 2],
    index: Arc<Mutex<AttachmentIndex<mod_api::EntityRef>>>,
    registrations: [horizon_mod_runtime::attachments::AttachmentRegistration; 2],
}

impl BoundProjectiles {
    fn bind_two(game: &mut Game<'_>) -> Self {
        let index = Arc::new(Mutex::new(AttachmentIndex::default()));
        let mut values = [Attachments::default(), Attachments::default()];
        let registrations = std::array::from_fn(|i| {
            let id = game.creatures.projectiles[i].id;
            let holder = mod_api::EntityRef::new(game.native_lifecycle.epoch().unwrap(), id);
            VALUE
                .set(&mut values[i], Holder::Entity(id), 10 + i as i32)
                .unwrap();
            game.creatures.projectiles.update(i, |p| {
                p.attachment_lifetime().bind(&mut values[i], &index, holder);
                p.attachment_lifetime().registration().unwrap()
            })
        });
        Self {
            _values: values,
            index,
            registrations,
        }
    }
}

#[derive(Clone, Default)]
struct Observed {
    impacts: Vec<u32>,
    spawned: bool,
    buffered: bool,
    remove: Option<u32>,
}

struct CancelImpacts(Arc<Mutex<Observed>>);

#[mod_api::__subscriptions]
impl Hooks for CancelImpacts {
    fn vanilla_game_event(
        &self,
        host: &mut dyn ServerHost,
        event: &mut mod_api::VanillaGameEventEvent<'_>,
    ) -> Flow {
        if event.event == "minecraft:projectile_shoot" {
            let mut observed = self.0.lock().unwrap();
            if !observed.buffered {
                // Shoot events fire after settlement while the tick batch is taken.
                let _ = host.summon(
                    event.dimension,
                    "minecraft:snowball",
                    [9.5, 70.0, 0.5],
                    Default::default(),
                );
                observed.buffered = true;
            }
        }
        Flow::Continue
    }

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
        let mut observed = self.0.lock().unwrap();
        observed.impacts.push(event.projectile);
        if !observed.spawned {
            // The projectile is installed even though summon has no projectile return target.
            let _ = host.summon(
                event.dimension,
                "minecraft:snowball",
                [8.5, 70.0, 0.5],
                Default::default(),
            );
            observed.spawned = true;
        }
        if observed.remove == Some(event.projectile) {
            let entity = host.entity(event.projectile).unwrap();
            assert!(entity.discard(host));
        }
        Flow::Cancel
    }
}

fn with_batch(
    remove: bool,
    test: impl FnOnce(&mut Game<'_>, [u32; 3], [JoinSeq; 3], u32, u32),
) {
    let observed = Arc::new(Mutex::new(Observed::default()));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "cancel-projectile-batch",
        Box::new(CancelImpacts(observed.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        super::super::native_entity_refs::tests::with_game(|game| {
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
            let ids = std::array::from_fn(|i| game.next_entity + i as u32 + 1);
            game.next_entity += 3;
            // Snowballs leave on impact; arrows stay in their slot, stuck in the wall.
            // The third projectile misses, so its survivor order must be preserved.
            for (i, kind) in [
                ProjectileKind::Snowball,
                ProjectileKind::Arrow {
                    base_damage: 2.0,
                    critical: false,
                    effect: None,
                },
                ProjectileKind::Snowball,
            ]
            .into_iter()
            .enumerate()
            {
                let x = if i == 2 { 5.5 } else { 0.5 };
                let mut p = Projectile::new(kind, Vec3::new(x, 64.5, 0.5), None);
                p.id = ids[i];
                p.velocity = Vec3::new(3.0, 0.0, 0.0);
                game.creatures.projectiles.push(p);
            }
            let joins = std::array::from_fn(|i| game.creatures.projectiles[i].join);
            let bound = BoundProjectiles::bind_two(game);
            let index = &bound.index;
            let key = ("test".into(), "projectile_lifetime".into());
            let before = index.lock().unwrap().snapshot(&key);
            assert_eq!(before.len(), 2);
            observed.lock().unwrap().remove = remove.then_some(ids[1]);
            game.tick_projectiles().unwrap();
            assert_eq!(index.lock().unwrap().snapshot(&key).len(), 2);
            for (id, original) in ids[..2].iter().zip(&bound.registrations) {
                let i = game
                    .creatures
                    .projectiles
                    .iter()
                    .position(|p| p.id == *id)
                    .unwrap();
                let current = game
                    .creatures
                    .projectiles
                    .update(i, |p| p.attachment_lifetime().registration().unwrap());
                assert!(original.same_registration(&current));
            }
            let observed = observed.lock().unwrap().clone();
            assert_eq!(observed.impacts, [ids[0], ids[1]]);
            assert!(observed.spawned && observed.buffered);
            let spawned = game
                .creatures
                .projectiles
                .iter()
                .find(|p| p.position.x == 8.5)
                .unwrap()
                .id;
            let buffered = game
                .creatures
                .projectiles
                .iter()
                .find(|p| p.position.x == 9.5)
                .unwrap();
            assert_eq!(buffered.age, 0);
            let buffered = buffered.id;
            test(game, ids, joins, spawned, buffered);
            assert_eq!(
                index.lock().unwrap().snapshot(&key).len(),
                if remove { 1 } else { 2 }
            );
            game.creatures.projectiles.clear();
            assert!(index.lock().unwrap().snapshot(&key).is_empty());
            assert!(
                before
                    .iter()
                    .all(|row| matches!(row.entry.read(&VALUE), Ok(10 | 11)))
            );
        });
    });
}

#[test]
#[ignore = "daily"]
fn no_impact_expiration_retires_only_the_expired_owner() {
    super::super::native_entity_refs::tests::with_game(|game| {
        game.options.dedicated = Some(Vec::new());
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        for (id, age, x) in [(71, 5999, 0.5), (72, 0, 5.5)] {
            let mut p =
                Projectile::new(ProjectileKind::Snowball, Vec3::new(x, 70.0, 0.5), None);
            p.id = id;
            p.age = age;
            p.velocity = Vec3::new(0.25, 0.0, 0.0);
            game.creatures.projectiles.push(p);
        }
        let bound = BoundProjectiles::bind_two(game);
        let key = ("test".into(), "projectile_lifetime".into());
        let before = bound.index.lock().unwrap().snapshot(&key);
        assert_eq!(before.len(), 2);
        game.tick_projectiles().unwrap();
        assert_eq!(
            game.creatures
                .projectiles
                .iter()
                .map(|p| p.id)
                .collect::<Vec<_>>(),
            [72]
        );
        assert_eq!(game.creatures.projectiles[0].age, 1);
        let current = game
            .creatures
            .projectiles
            .update(0, |p| p.attachment_lifetime().registration().unwrap());
        assert!(bound.registrations[1].same_registration(&current));
        let after = bound.index.lock().unwrap().snapshot(&key);
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].entry.read(&VALUE), Ok(&11));
        game.creatures.projectiles.clear();
        assert!(bound.index.lock().unwrap().snapshot(&key).is_empty());
        assert!(
            before
                .iter()
                .all(|row| matches!(row.entry.read(&VALUE), Ok(10 | 11)))
        );
    });
}

#[test]
#[ignore = "daily"]
fn canceled_impacts_keep_projectile_batch_joins_and_survivor_order() {
    with_batch(false, |game, ids, joins, spawned, buffered| {
        let list = &mut game.creatures.projectiles;
        // The replayed snowball is appended; the arrow keeps its original slot.
        assert_eq!(
            list.iter().map(|p| p.id).collect::<Vec<_>>(),
            [buffered, ids[1], ids[2], spawned, ids[0]]
        );
        for (id, join) in ids.into_iter().zip(joins) {
            let p = list.iter().find(|p| p.id == id).unwrap();
            assert_eq!(p.join, join);
            assert_eq!(p.age, 1);
            assert!(p.position.x > 2.0);
            assert!(!p.stuck);
        }
        let snowball = list.iter().position(|p| p.id == ids[0]).unwrap();
        list.update(snowball, |p| {
            assert!(p.has_been_shot);
            assert!(p.take_shoot_event().is_none());
        });
        let fresh = list.iter().find(|p| p.id == spawned).unwrap();
        assert!(joins.into_iter().all(|join| join < fresh.join));
        assert_eq!(fresh.age, 0);
    });
}

#[test]
#[ignore = "daily"]
fn a_projectile_removed_in_its_impact_hook_stays_removed_after_replay() {
    with_batch(true, |game, ids, joins, spawned, buffered| {
        // Pending removal refuses the handle already, even before the physical sweep.
        assert!(game.entity(ids[1]).is_none());
        // Refusal must happen before replay, not only at the later removal sweep.
        let arrow = game
            .creatures
            .projectiles
            .iter()
            .find(|p| p.id == ids[1])
            .unwrap();
        assert!(arrow.stuck);
        assert!(arrow.position.x < 2.0);
        game.native_removals_flush();
        let list = &game.creatures.projectiles;
        assert_eq!(
            list.iter().map(|p| p.id).collect::<Vec<_>>(),
            [buffered, ids[2], spawned, ids[0]]
        );
        assert_eq!(list[1].join, joins[2]);
        assert_eq!(list[3].join, joins[0]);
        game.tick_projectiles().unwrap();
        assert!(!game.creatures.projectiles.iter().any(|p| p.id == ids[1]));
    });
}
