//! Production-path storage lifecycle and lifted-pass test support.
use crate::game::Game;
use horizon_client::join_seq::JoinList;
use horizon_mod_runtime::attachments::{
    AttachmentRegistration, AttachmentType, Attachments, HasAttachmentLifetime, Holder,
    index::AttachmentIndex,
};
use mod_api::{EntityRef, WorldEpoch};
use std::{
    cell::Cell,
    collections::HashMap,
    sync::{Arc, Mutex},
};

type Spawn = fn(&mut Game<'_>);
thread_local! {
    static AFTER_TAKE: Cell<Option<Spawn>> = const { Cell::new(None) };
}

pub(in crate::game) fn with_spawn(spawn: Spawn, tick: impl FnOnce()) {
    assert!(AFTER_TAKE.replace(Some(spawn)).is_none());
    tick();
    assert!(AFTER_TAKE.take().is_none(), "tick did not lift its batch");
}

pub(in crate::game) fn after_take(game: &mut Game<'_>) {
    if let Some(spawn) = AFTER_TAKE.take() {
        spawn(game);
    }
}

/// Storage adapters for the same production lifecycle walk across entity kinds.
pub(in crate::game) struct LifecycleFixture {
    pub seed: fn(&mut Game<'_>, &[(u32, [f64; 3])]),
    pub ids: fn(&Game<'_>) -> Vec<u32>,
    pub target: fn(u32) -> crate::game::commands::Target,
    pub unloads_with_chunk: bool,
    pub remote: Option<RemoteLifecycle>,
}

pub(in crate::game) struct RemoteLifecycle {
    pub prepare: fn(&mut Game<'_>),
    pub apply: fn(&mut Game<'_>, u32, u32),
}

struct ScratchDirectory(std::path::PathBuf);

impl Drop for ScratchDirectory {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

static LIFECYCLE_VALUE: AttachmentType<i32> = AttachmentType {
    namespace: "test",
    name: "lifecycle_owner",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync: false,
};

/// Registration memory shared by slot-bearing adapters to the lifecycle walk.
#[derive(Default)]
pub(in crate::game) struct SlotIdentities {
    values: Attachments,
    index: Arc<Mutex<AttachmentIndex<EntityRef>>>,
    registrations: HashMap<u32, AttachmentRegistration>,
}

impl SlotIdentities {
    pub fn observe<T: HasAttachmentLifetime + horizon_client::join_seq::Joined>(
        &mut self,
        owners: &mut JoinList<T, false>,
        epoch: WorldEpoch,
        id: fn(&T) -> u32,
    ) {
        owners.for_each_mut(|owner| {
            let id = id(owner);
            self.observe_one(owner, epoch, id);
        });
    }

    pub fn observe_one<T: HasAttachmentLifetime>(
        &mut self,
        owner: &mut T,
        epoch: WorldEpoch,
        id: u32,
    ) {
        use std::collections::hash_map::Entry;
        match self.registrations.entry(id) {
            Entry::Occupied(original) => {
                let current = owner
                    .attachment_lifetime()
                    .registration()
                    .expect("known owner lost its registration");
                assert!(
                    original.get().same_registration(&current),
                    "owner {id} changed registration"
                );
            }
            Entry::Vacant(entry) => {
                LIFECYCLE_VALUE
                    .set(&mut self.values, Holder::Entity(id), id as i32)
                    .unwrap();
                owner.attachment_lifetime().bind(
                    &mut self.values,
                    &self.index,
                    EntityRef::new(epoch, id),
                );
                entry.insert(owner.attachment_lifetime().registration().unwrap());
            }
        }
    }
}

impl LifecycleFixture {
    /// Adapters seed owners and read ordered identities; all exchanges and
    /// removals run through production entry points, never a substitute model.
    pub fn run(self) {
        self.run_with(|_| {});
    }

    /// Observe each seeded or transformed storage before checking its order.
    /// Slot-bearing adapters bind new owners and compare remembered registrations here.
    pub fn run_with(self, observe: impl FnMut(&mut Game<'_>)) {
        self.walk(false, observe);
    }

    /// A singleton tests selected and unselected chunks in separate passes.
    /// Remote singleton behavior is checked by its storage-specific fixture.
    pub fn run_singleton(self) {
        assert!(self.remote.is_none());
        self.walk(true, |_| {});
    }

    fn walk(self, singleton: bool, mut observe: impl FnMut(&mut Game<'_>)) {
        crate::game::native_entity_refs::tests::with_game(|game| {
            let mut ids = |game: &mut Game<'_>| {
                observe(game);
                use horizon_client::join_seq::Joined;
                let mut unique = std::collections::HashSet::new();
                macro_rules! lookup {
                    ($list:expr) => {
                        for (index, entity) in $list.iter().enumerate() {
                            let join = entity.join_seq();
                            assert!(join.is_joined());
                            assert!(join < horizon_client::join_seq::JoinSeq::upcoming());
                            assert!(unique.insert(join), "duplicate owner join");
                            assert_eq!($list.locate(join), Some(index));
                        }
                    };
                }
                lookup!(game.others);
                lookup!(game.fangs.list);
                lookup!(game.ominous_spawners);
                lookup!(game.breath_clouds);
                if let Some(dragon) = game.dragon.as_ref() {
                    assert!(dragon.join.is_joined());
                    assert!(dragon.join < horizon_client::join_seq::JoinSeq::upcoming());
                    assert!(unique.insert(dragon.join));
                    assert_eq!(game.dragon.locate(dragon.join), Some(0));
                }
                (self.ids)(game)
            };
            let root = ScratchDirectory(std::env::temp_dir().join(format!(
                "horizon-lifecycle-{}-{:?}",
                std::process::id(),
                std::thread::current().id()
            )));
            game.world_root = root.0.clone();
            let active = if singleton || self.unloads_with_chunk {
                vec![103]
            } else {
                vec![101, 103, 104]
            };
            if singleton {
                (self.seed)(game, &[(101, [1.0, 80.0, 1.0])]);
                game.kill_target((self.target)(101));
                assert!(ids(game).is_empty(), "singleton command removal");
                (self.seed)(game, &[(103, [33.0, 80.0, 1.0])]);
                game.entity_chunks(&[(0, 0)].into_iter().collect(), true);
                assert_eq!(ids(game), [103], "unselected singleton chunk");
                (self.seed)(game, &[(101, [1.0, 80.0, 1.0])]);
                game.entity_chunks(&[(0, 0)].into_iter().collect(), true);
                assert_eq!(
                    ids(game),
                    if self.unloads_with_chunk {
                        vec![]
                    } else {
                        vec![101]
                    },
                    "selected singleton chunk"
                );
                (self.seed)(game, &[(103, [33.0, 80.0, 1.0])]);
            } else {
                (self.seed)(
                    game,
                    &[
                        (101, [1.0, 80.0, 1.0]),
                        (102, [17.0, 80.0, 1.0]),
                        (103, [33.0, 80.0, 1.0]),
                        (104, [2.0, 80.0, 1.0]),
                    ],
                );
                assert_eq!(ids(game), [101, 102, 103, 104]);
                game.kill_target((self.target)(102));
                assert_eq!(ids(game), [101, 103, 104], "command removal");
                game.entity_chunks(&[(0, 0)].into_iter().collect(), true);
                assert_eq!(ids(game), active, "selected versus unselected chunk unload");
            }
            game.enter_level(crate::game::Dim::Nether).unwrap();
            assert!(
                ids(game).is_empty(),
                "new dimension inherited the active owners"
            );
            let parked = &[(201, [1.0, 80.0, 1.0]), (202, [33.0, 80.0, 1.0])]
                [..if singleton { 1 } else { 2 }];
            let parked_ids = if singleton { vec![201] } else { vec![201, 202] };
            (self.seed)(game, parked);
            assert_eq!(ids(game), parked_ids);
            game.enter_level(crate::game::Dim::Overworld).unwrap();
            assert_eq!(ids(game), active, "active owner restore");
            game.enter_level(crate::game::Dim::Nether).unwrap();
            assert_eq!(ids(game), parked_ids, "parked owner restore");
            game.enter_level(crate::game::Dim::Overworld).unwrap();
            assert_eq!(ids(game), active, "second active restore");
            if let Some(remote) = self.remote {
                (remote.prepare)(game);
                (self.seed)(
                    game,
                    &[
                        (301, [1.0, 80.0, 1.0]),
                        (302, [17.0, 80.0, 1.0]),
                        (303, [33.0, 80.0, 1.0]),
                    ],
                );
                assert_eq!(ids(game), [301, 302, 303], "remote owner install");
                (remote.apply)(game, 302, 303);
                assert_eq!(ids(game), [301, 303], "remote update and removal");
            }
        });
    }
}

#[test]
fn lifecycle_scratch_is_removed_when_an_assertion_unwinds() {
    let path = std::env::temp_dir().join(format!(
        "horizon-lifecycle-cleanup-{}-{:?}",
        std::process::id(),
        std::thread::current().id()
    ));
    let result = std::panic::catch_unwind(|| {
        let _scratch = ScratchDirectory(path.clone());
        std::fs::create_dir_all(&path).unwrap();
        panic!("fixture assertion");
    });
    assert!(result.is_err());
    assert!(!path.exists());
}

#[test]
#[ignore = "daily"]
fn lifecycle_fixture_walks_fangs_owners() {
    LifecycleFixture {
        seed: |game, entries| {
            game.fangs.list.clear();
            for &(id, at) in entries {
                game.insert_fangs(id, at, 0.0, 0, 0);
            }
        },
        ids: |game| game.fangs.list.iter().map(|f| f.id).collect(),
        target: crate::game::commands::Target::Fangs,
        unloads_with_chunk: true,
        remote: None,
    }
    .run();
}

#[test]
#[ignore = "daily"]
fn lifecycle_fixture_walks_other_owners() {
    LifecycleFixture {
        seed: |game, entries| {
            game.others.clear();
            for &(id, at) in entries {
                game.others.push(crate::game::commands::OtherEntity {
                    attachment_lifetime: Default::default(),
                    knot_check: None,
                    eye: None,
                    join: Default::default(),
                    id,
                    kind: "minecraft:marker".into(),
                    position: horizon_core::geometry::Vec3::new(at[0], at[1], at[2]),
                    yaw: 0.0,
                    pitch: 0.0,
                    item_transformation: None,
                    interaction: None,
                });
            }
        },
        ids: |game| game.others.iter().map(|entity| entity.id).collect(),
        target: crate::game::commands::Target::Other,
        unloads_with_chunk: true,
        remote: None,
    }
    .run();
}

/// Run each fixture's complete setup and actual action in both dimensions.
pub(in crate::game) fn in_both_dimensions(mut test: impl FnMut(&mut Game<'_>, &PassBalance)) {
    for dimension in [crate::game::Dim::Overworld, crate::game::Dim::Nether] {
        // with_game isolates the initial SaveDir; newly opened dimensions use
        // world_root instead. Keep that root alive until the Game is dropped.
        let root = ScratchDirectory(std::env::temp_dir().join(format!(
            "horizon-caller-balance-{}-{:?}-{dimension:?}",
            std::process::id(),
            std::thread::current().id()
        )));
        crate::game::native_entity_refs::tests::with_game(|game| {
            game.world_root = root.0.clone();
            game.enter_level(dimension).unwrap();
            use horizon_client::join_seq::residency::{Kind, Owner};
            let balance = PassBalance {
                journal: game
                    .resident_entities
                    .table
                    .borrow()
                    .producer()
                    .observe_structural_events(),
                owner: Owner {
                    dimension: game.dim.key(),
                    kind: Kind::Mob,
                },
            };
            test(game, &balance);
            balance.assert_parked(game);
            assert_eq!(
                game.dim, dimension,
                "fixture changed the selected dimension"
            );
        });
    }
}

pub(in crate::game) struct PassBalance {
    journal: horizon_client::join_seq::residency::Journal,
    owner: horizon_client::join_seq::residency::Owner,
}
impl PassBalance {
    pub(in crate::game) fn drain_balanced(&self) -> usize {
        use horizon_client::join_seq::residency::Change;
        let mut events = Vec::new();
        self.journal.drain_into(&mut events);
        let mut depth = 0usize;
        let mut lifts = 0;
        for event in events {
            if event.owner != self.owner {
                continue;
            }
            match event.change {
                Change::Lifted => {
                    depth += 1;
                    lifts += 1;
                }
                Change::Restored => depth = depth.checked_sub(1).expect("restore without lift"),
                _ => {}
            }
        }
        assert_eq!(depth, 0, "actual caller abandoned a lifted pass");
        lifts
    }
    fn assert_parked(&self, game: &mut Game<'_>) {
        let home = game.dim;
        let away = match home {
            crate::game::Dim::Overworld => crate::game::Dim::Nether,
            _ => crate::game::Dim::Overworld,
        };
        game.enter_level(away).unwrap();
        self.drain_balanced();
        game.enter_level(home).unwrap();
    }
}

/// Inject a nested spawn into an actual lifted mob operation, then exercise the
/// next real entity pass with just that new owner so the caller's other state
/// remains available for its own assertions.
pub(in crate::game) fn assert_mob_spawn_survives(
    game: &mut Game<'_>,
    balance: &PassBalance,
    action: impl FnOnce(&mut Game<'_>),
) {
    const SPAWNED: u32 = 1_900_000_000;
    fn spawn(game: &mut Game<'_>) {
        let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
        let mut mob = horizon_client::mobs::Mob::new(
            SPAWNED,
            kind,
            horizon_core::geometry::Vec3::new(0.5, 80.0, 0.5),
            0.0,
        );
        mob.no_ai = true;
        game.mobs.push(mob);
    }
    balance.drain_balanced();
    let mut expected: Vec<_> = game.mobs.iter().map(|mob| (mob.id, mob.join)).collect();
    with_spawn(spawn, || action(game));
    assert!(
        balance.drain_balanced() > 0,
        "actual caller did not emit its lift"
    );
    let index = game
        .mobs
        .iter()
        .position(|m| m.id == SPAWNED)
        .expect("nested spawn was dropped");
    assert_eq!(game.mobs[index].age, 0, "spawn ticked in the current pass");
    let join = game.mobs[index].join;
    expected.push((SPAWNED, join));
    assert_eq!(
        game.mobs
            .iter()
            .map(|mob| (mob.id, mob.join))
            .collect::<Vec<_>>(),
        expected,
        "caller changed survivor identity or spawn admission order"
    );
    let mut original = game.mobs.lift();
    let spawned = original.extract(join).unwrap();
    game.mobs.push(spawned);
    if !game.world.chunks.contains_key(&(0, 0)) {
        game.world.chunks.insert(
            (0, 0),
            std::sync::Arc::new(horizon_client::chunk::Chunk::empty(0, 0)),
        );
    }
    game.tick_creature_entities().unwrap();
    assert_eq!(game.mobs[0].id, SPAWNED);
    assert_eq!(game.mobs[0].join, join);
    assert_eq!(game.mobs[0].age, 1, "spawn missed the next entity pass");
    game.mobs.clear();
    game.mobs.restore(
        original,
        horizon_client::join_seq::RestoreOrder::SurvivorsFirst,
    );
    balance.assert_parked(game);
}

/// Check a real player/guest projectile pickup pass and the next server flight pass.
pub(in crate::game) fn assert_projectile_spawn_survives(
    game: &mut Game<'_>,
    action: impl FnOnce(&mut Game<'_>),
) {
    const ID: u32 = 1_900_000_002;
    fn spawn(game: &mut Game<'_>) {
        let mut p = horizon_client::projectiles::Projectile::new(
            horizon_client::projectiles::ProjectileKind::Egg,
            horizon_core::geometry::Vec3::new(3.5, 80.0, 3.5),
            None,
        );
        p.id = ID;
        game.creatures.projectiles.push(p);
    }
    let mut expected: Vec<_> = game
        .creatures
        .projectiles
        .iter()
        .filter(|p| p.id != 72)
        .map(|p| (p.id, p.join))
        .collect();
    with_spawn(spawn, || action(game));
    let p = game
        .creatures
        .projectiles
        .iter()
        .find(|p| p.id == ID)
        .expect("pickup dropped a nested projectile");
    assert_eq!(p.age, 0);
    let join = p.join;
    expected.push((ID, join));
    assert_eq!(
        game.creatures
            .projectiles
            .iter()
            .map(|p| (p.id, p.join))
            .collect::<Vec<_>>(),
        expected
    );
    let mut original = game.creatures.projectiles.lift();
    game.creatures
        .projectiles
        .push(original.extract(join).unwrap());
    game.tick_projectiles().unwrap();
    assert_eq!(game.creatures.projectiles[0].age, 1);
    assert_eq!(game.creatures.projectiles[0].join, join);
    game.creatures.projectiles.clear();
    game.creatures.projectiles.restore(
        original,
        horizon_client::join_seq::RestoreOrder::SurvivorsFirst,
    );
}

#[test]
#[ignore = "daily"]
fn data_merge_preserves_two_distinct_other_joins_for_every_runtime_kind() {
    crate::game::native_entity_refs::tests::with_game(|game| {
        game.build_registries();
        game.set_allow_commands(true);
        for kind in [
            "marker",
            "block_display",
            "item_display",
            "text_display",
            "interaction",
        ] {
            game.others.clear();
            for (tag, x) in [("first", 1), ("second", 8)] {
                assert!(game.run_command(&format!("summon {kind} {x} 80 1 {{Tags:[\"{tag}\"]}}")));
            }
            let expected: Vec<_> = game.others.iter().map(|e| (e.id, e.join)).collect();
            assert_eq!(expected.len(), 2, "{kind}");
            assert_ne!(expected[0].1, expected[1].1);
            assert_ne!(expected[0].1, horizon_client::join_seq::JoinSeq::UNJOINED);
            for tag in ["first", "second"] {
                assert!(game.run_command(&format!(
                    "data merge entity @e[tag={tag},limit=1] {{Rotation:[45.0f,10.0f]}}"
                )));
            }
            assert_eq!(
                game.others
                    .iter()
                    .map(|e| (e.id, e.join))
                    .collect::<Vec<_>>(),
                expected,
                "{kind}"
            );
            for (id, join) in expected {
                assert_eq!(game.others[game.others.locate(join).unwrap()].id, id);
            }
        }
    });
}

#[test]
#[ignore = "daily"]
fn mob_tick_restores_spawn_buffer_before_multiple_survivors_without_renumbering() {
    fn seed(game: &mut Game<'_>, ids: std::ops::RangeInclusive<u32>, x: f64) {
        for (index, id) in ids.enumerate() {
            let kind = game.res.mob_types().get("minecraft:pig").unwrap().clone();
            let mut mob = horizon_client::mobs::Mob::new(
                id,
                kind,
                horizon_core::geometry::Vec3::new(x + index as f64 * 2.0, 80.0, 1.5),
                0.0,
            );
            mob.no_ai = true;
            game.mobs.push(mob);
        }
    }
    crate::game::native_entity_refs::tests::with_game(|game| {
        game.world
            .chunks
            .insert((0, 0), Arc::new(horizon_client::chunk::Chunk::empty(0, 0)));
        seed(game, 71..=73, 1.5);
        let original: Vec<_> = game.mobs.iter().map(|m| (m.id, m.join)).collect();
        // Synthetic additions at the actual lift, then the complete production
        // tick (not a reconstructed restore or a helper replacing that tick).
        with_spawn(
            |g| seed(g, 81..=82, 9.5),
            || game.tick_creature_entities().unwrap(),
        );
        assert_eq!(
            game.mobs.iter().map(|m| m.id).collect::<Vec<_>>(),
            [81, 82, 71, 72, 73]
        );
        assert_eq!(
            game.mobs
                .iter()
                .skip(2)
                .map(|m| (m.id, m.join))
                .collect::<Vec<_>>(),
            original
        );
        assert_eq!(
            game.mobs.iter().map(|m| m.age).collect::<Vec<_>>(),
            [0, 0, 1, 1, 1]
        );
        let all: Vec<_> = game.mobs.iter().map(|m| (m.id, m.join)).collect();
        game.tick_creature_entities().unwrap();
        assert_eq!(
            game.mobs.iter().map(|m| (m.id, m.join)).collect::<Vec<_>>(),
            all
        );
        assert_eq!(
            game.mobs.iter().map(|m| m.age).collect::<Vec<_>>(),
            [1, 1, 2, 2, 2]
        );
    });
}
