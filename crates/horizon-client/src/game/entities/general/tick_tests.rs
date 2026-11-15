//! arismoko/horizon#792 stage 4: the general rows' tick pass through the game fixture. The
//! fixture part advances a per-row counter attachment (arismoko/horizon#796: the sole
//! per-entity store) and records the callback trace; the tests pin the
//! acceptance matrix's ordering, residency and containment rows against the
//! pass's own contract, with the vanilla orderings cited where they hold.
use super::*;
use crate::game::native_entity_refs::tests::with_game;
use horizon_core::nbt::{Compound, Tag};
use mod_api::{
    AttachmentCodec, AttachmentError, AttachmentType, EntityPart, EntityTickListenedEvent,
    EntityTickPreEvent, Flow, GeneralEntityCategory, GeneralEntityTickEvent, GeneralSavePolicy,
    Holder, Hooks, Registrar, ServerHost,
};
use std::sync::{Arc, Mutex};

/// The fixture counter: an attachment with a codec, so a saved row resumes
/// its count and a quarantined record keeps it in its bytes.
static COUNTER: AttachmentType<i32> = AttachmentType {
    namespace: "mymod",
    name: "counter",
    default: || 0,
    codec: Some(AttachmentCodec {
        encode: |n| Ok([("n".into(), Tag::Int(*n))].into()),
        decode: |tag: &Compound| match tag.get(&"n".into()) {
            Some(Tag::Int(n)) => Ok(*n),
            _ => Err(AttachmentError::Decode),
        },
    }),
    copy_on_death: false,
    sync: false,
};

/// What the part saw, in call order: each row's id.
type Trace = Arc<Mutex<Vec<u32>>>;

/// The whole pass's global pair and ticks, in call order: a row's id and
/// which moment it heard — the flat rider order and the cancel rules are
/// asserted against this.
type Events = Arc<Mutex<Vec<(u32, &'static str)>>>;

/// What a listener does from inside one row's callback: the liveness tests'
/// mid-pass removals and dismounts, armed by the test before the tick.
enum OnTick {
    Nothing,
    /// From this row's callback, queue the named row's removal
    /// (`entity_remove`: it stays in the list until the sweep).
    Remove(u32),
    /// From this row's callback, take the named row off its vehicle.
    Dismount(u32),
}

/// The fixture part: `test:counter` of the acceptance matrix. It carries no
/// per-instance state — two rows of one type share this one part, and their
/// counters live apart in their attachments.
struct Counter {
    trace: Trace,
    events: Events,
    /// Panics on this row's first tick only, after its write: the
    /// containment row's failing subscriber (0 arms nothing — a test sets
    /// its target once its rows have ids).
    panic_on: Arc<std::sync::atomic::AtomicU32>,
    /// What the row's callback does to another row before it records itself.
    action: Arc<Mutex<OnTick>>,
    fired: std::sync::atomic::AtomicBool,
}

#[mod_api::horizon_part]
impl EntityPart for Counter {
    fn general_entity_tick(
        &self,
        host: &mut dyn ServerHost,
        event: &mut GeneralEntityTickEvent<'_>,
    ) {
        let holder = Holder::Entity(event.entity.id());
        let value = COUNTER.get(host, holder).unwrap() + 1;
        COUNTER.set(host, holder, value).unwrap();
        if self.panicking_on() == Some(event.entity.id())
            && !self.fired.swap(true, std::sync::atomic::Ordering::Relaxed)
        {
            panic!("general entity tick boom");
        }
        let action = std::mem::replace(&mut *self.action.lock().unwrap(), OnTick::Nothing);
        let other = |id: u32| mod_api::EntityRef::new(event.entity.epoch(), id);
        match action {
            OnTick::Nothing => {}
            OnTick::Remove(id) => {
                assert!(other(id).discard(host));
            }
            OnTick::Dismount(id) => {
                assert!(other(id).stop_riding(host));
            }
        }
        self.events
            .lock()
            .unwrap()
            .push((event.entity.id(), "tick"));
        self.trace.lock().unwrap().push(event.entity.id());
    }
}

impl Counter {
    fn panicking_on(&self) -> Option<u32> {
        let target = self.panic_on.load(std::sync::atomic::Ordering::Relaxed);
        (target != 0).then_some(target)
    }
}

/// Declares the two fixture types and attaches the counter part to
/// `mymod:robot`; `ghost` stays part-free for the zero-callback rows. The
/// same mod listens to the entity tick gates, records the global pair into
/// the events log, cancels the row named by `cancel_on` when armed, and
/// queues the removal of `pre_victim` from `pre_on`'s own Pre.
struct Declare {
    trace: Trace,
    events: Events,
    panic_on: Arc<std::sync::atomic::AtomicU32>,
    action: Arc<Mutex<OnTick>>,
    cancel_on: Arc<std::sync::atomic::AtomicU32>,
    pre_on: Arc<std::sync::atomic::AtomicU32>,
    pre_victim: Arc<std::sync::atomic::AtomicU32>,
}

#[mod_api::__subscriptions]
impl Hooks for Declare {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        for (path, save) in [("robot", true), ("ghost", false)] {
            r.declare_general_entity(
                path,
                GeneralEntityCategory::Misc,
                0.8,
                1.9,
                if save {
                    GeneralSavePolicy::Save
                } else {
                    GeneralSavePolicy::NoSave
                },
                true,
                8,
                3,
                1,
                &[],
            )
            .unwrap();
        }
        r.entity_part(
            "mymod:robot",
            Counter {
                trace: self.trace.clone(),
                events: self.events.clone(),
                panic_on: self.panic_on.clone(),
                action: self.action.clone(),
                fired: std::sync::atomic::AtomicBool::new(false),
            },
        );
    }
    fn entity_tick_listened(&self, _: &mut dyn ServerHost, event: &mut EntityTickListenedEvent) {
        event.listened = true;
    }
    fn entity_tick_pre(
        &self,
        host: &mut dyn ServerHost,
        event: &mut EntityTickPreEvent<'_>,
    ) -> Flow {
        self.events.lock().unwrap().push((event.entity, "pre"));
        if self.pre_on.load(std::sync::atomic::Ordering::Relaxed) == event.entity {
            let victim = self.pre_victim.load(std::sync::atomic::Ordering::Relaxed);
            let victim = host
                .entity(victim)
                .expect("the victim is a live row at the remover's Pre");
            assert!(victim.discard(host));
        }
        if self.cancel_on.load(std::sync::atomic::Ordering::Relaxed) == event.entity {
            return Flow::Cancel;
        }
        Flow::Continue
    }
    fn entity_tick_post(
        &self,
        _: &mut dyn ServerHost,
        event: &mut mod_api::EntityTickPostEvent<'_>,
    ) {
        self.events.lock().unwrap().push((event.entity, "post"));
    }
}

/// A counter fixture game. The registration phase is scoped around the whole
/// fixture: the harness re-runs the mods' `register` while the game builds,
/// and a general type is only declared while that phase is current. The
/// runtime initializes before the scope: a scope entered before the platform
/// installs its instrumentation writes a fallback nobody reads, and the
/// declaration then refuses as frozen.
fn with_counter(test: impl FnOnce(&mut Game<'_>, Levers)) {
    let declare = Declare {
        trace: Arc::default(),
        events: Arc::default(),
        panic_on: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        action: Arc::new(Mutex::new(OnTick::Nothing)),
        cancel_on: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        pre_on: Arc::new(std::sync::atomic::AtomicU32::new(0)),
        pre_victim: Arc::new(std::sync::atomic::AtomicU32::new(0)),
    };
    let levers = Levers {
        trace: declare.trace.clone(),
        events: declare.events.clone(),
        panic_on: declare.panic_on.clone(),
        action: declare.action.clone(),
        cancel_on: declare.cancel_on.clone(),
        pre_on: declare.pre_on.clone(),
        pre_victim: declare.pre_victim.clone(),
    };
    mod_platform::__private::initialize().expect("test registration requires the mod API runtime");
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::Registration);
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new("mymod", Box::new(declare))].into();
    mod_platform::testing::with_mods(mods, || {
        with_game(|game| {
            // The fixture speaks as a server, as the vehicle boundary tests
            // make theirs: attachment writes and the entity tick gates answer
            // only an authoritative game.
            game.options.dedicated = Some(Vec::new());
            game.native_level_opened(crate::game::Dim::Overworld);
            game.build_registries();
            game.reset_entity_residency();
            // The registration phase ends before the test's own ticks: a
            // part's writes are storage writes, which only a world's phases
            // take, as the server's own tick runs in World.
            let _world = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
            test(game, levers)
        })
    });
}

/// The test's handles into the fixture's listeners.
struct Levers {
    trace: Trace,
    events: Events,
    panic_on: Arc<std::sync::atomic::AtomicU32>,
    action: Arc<Mutex<OnTick>>,
    cancel_on: Arc<std::sync::atomic::AtomicU32>,
    pre_on: Arc<std::sync::atomic::AtomicU32>,
    pre_victim: Arc<std::sync::atomic::AtomicU32>,
}

/// One record's save tag at `at`, the format `to_tag` writes.
fn record_at(type_key: &str, uuid: [i32; 4], at: [f64; 3], riders: Vec<Tag>) -> Tag {
    let mut tag = GeneralEntityRecord {
        position: at,
        velocity: [0.0; 3],
        rotation: [0.0, 90.0],
        uuid,
        envelope: GeneralTypeEnvelope {
            type_key: type_key.to_owned(),
            payload_version: 3,
            payload_bytes: Vec::new(),
        },
        passengers: Vec::new(),
    }
    .to_tag();
    if let (Tag::Compound(map), false) = (&mut tag, riders.is_empty()) {
        map.insert(format::PASSENGERS.into(), Tag::List(riders));
    }
    tag
}

/// Spawns one tree of `mymod:robot` records and answers its root's row id.
fn spawn_tree(game: &mut Game<'_>, root: [i32; 4], at: [f64; 3], riders: Vec<Tag>) -> u32 {
    game.load_listed(
        "mymod:robot",
        None,
        None,
        record_at("mymod:robot", root, at, riders),
    )
    .unwrap();
    game.resident_entities
        .general
        .rows
        .iter()
        .find(|row| row.uuid == root)
        .unwrap()
        .id
}

/// The rows' ids in storage order.
fn row_ids(game: &Game<'_>) -> Vec<u32> {
    game.resident_entities
        .general
        .rows
        .iter()
        .map(|row| row.id)
        .collect()
}

/// One entity phase over the current level: what `tick_world` does between
/// its ordered runs, driven by hand. The spawn chunks tick with nobody
/// watching, as vanilla's do (`tickets.update(None)`).
fn tick(game: &mut Game<'_>) {
    game.tickets.update(None);
    game.begin_ordered_ticks();
    game.tick_general().unwrap();
}

/// A row's counter, read through the same host surface the part writes.
fn counter(game: &mut Game<'_>, id: u32) -> i32 {
    COUNTER.get(game, Holder::Entity(id)).unwrap()
}

/// One robot row, its counter already at `value`.
fn seeded_row(game: &mut Game<'_>, uuid: [i32; 4], value: i32) -> u32 {
    let id = spawn_tree(game, uuid, [0.5, 64.0, 0.5], Vec::new());
    COUNTER.set(game, Holder::Entity(id), value).unwrap();
    id
}

/// One tick of `test:counter` (the matrix's single row): 40 becomes 41.
#[test]
#[ignore = "daily"]
fn counter_advances_one_per_tick() {
    with_counter(|game, l| {
        let id = seeded_row(game, [1, 2, 3, 4], 40);
        assert_eq!(counter(game, id), 40);
        tick(game);
        assert_eq!(counter(game, id), 41);
        assert_eq!(*l.trace.lock().unwrap(), [id]);
    });
}

/// Two instances of one type: the part is shared, their counters are not.
#[test]
#[ignore = "daily"]
fn two_rows_share_the_part_but_not_their_counters() {
    with_counter(|game, l| {
        let first = seeded_row(game, [1, 2, 3, 4], 40);
        let second = seeded_row(game, [5, 6, 7, 8], 7);
        tick(game);
        assert_eq!((counter(game, first), counter(game, second)), (41, 8));
        assert_eq!(*l.trace.lock().unwrap(), [first, second]);
    });
}

/// A branching tree three generations deep (root, two children, a
/// grandchild under the first): one phase gives each row its 1, in
/// `tickNonPassenger`'s order — the root, then its rider subtree depth-first
/// (`tickPassenger` recurses before the next rider), a rider never a root.
#[test]
#[ignore = "daily"]
fn parent_ticks_before_its_depth_first_rider_subtree() {
    with_counter(|game, l| {
        let grandchild = record_at("mymod:robot", [8, 8, 8, 8], [0.5, 64.0, 0.5], Vec::new());
        let rider = record_at(
            "mymod:robot",
            [9, 9, 9, 9],
            [0.5, 64.0, 0.5],
            vec![grandchild],
        );
        let sibling = record_at("mymod:robot", [7, 7, 7, 7], [0.5, 64.0, 0.5], Vec::new());
        let root = spawn_tree(game, [1, 2, 3, 4], [0.5, 64.0, 0.5], vec![rider, sibling]);
        let ids = row_ids(game);
        let (rider_id, grandchild_id, sibling_id) = (ids[1], ids[2], ids[3]);
        tick(game);
        assert_eq!(
            *l.trace.lock().unwrap(),
            [root, rider_id, grandchild_id, sibling_id]
        );
        for id in ids {
            assert_eq!(counter(game, id), 1);
        }
    });
}

/// The join-cutoff row: a row that joins during the tick's span (here at or
/// above `begin_ordered_ticks`' `born` cutoff — a mod's own spawn path,
/// which stage 4 does not ship, joins exactly there) first ticks the next
/// phase, the vanilla tick list's passive swap.
#[test]
#[ignore = "daily"]
fn a_row_joined_during_the_tick_span_ticks_the_next_phase() {
    with_counter(|game, l| {
        let first = seeded_row(game, [1, 2, 3, 4], 40);
        game.tickets.update(None);
        game.begin_ordered_ticks();
        let late = spawn_tree(game, [5, 6, 7, 8], [0.5, 64.0, 0.5], Vec::new());
        game.tick_general().unwrap();
        assert_eq!(*l.trace.lock().unwrap(), [first]);
        assert_eq!(counter(game, late), 0);
        l.trace.lock().unwrap().clear();
        tick(game);
        assert_eq!(*l.trace.lock().unwrap(), [first, late]);
        assert_eq!(counter(game, late), 1);
    });
}

/// A shrunken row list leaves no stale rider link: removing the last rider
/// of a two-rider tree (the sweep's effect, reproduced by taking the row
/// out of the list) leaves the first rider's sibling chain pointing past
/// the list's end unless the index's buffers are cleared before the
/// rebuild — the next pass would walk off it. Without the panic, the root
/// and the remaining rider each tick once.
#[test]
#[ignore = "daily"]
fn a_removed_last_rider_leaves_no_stale_index_link() {
    with_counter(|game, l| {
        let first = record_at("mymod:robot", [9, 9, 9, 9], [0.5, 64.0, 0.5], Vec::new());
        let second = record_at("mymod:robot", [7, 7, 7, 7], [0.5, 64.0, 0.5], Vec::new());
        let root = spawn_tree(game, [1, 2, 3, 4], [0.5, 64.0, 0.5], vec![first, second]);
        let ids = row_ids(game);
        tick(game);
        assert_eq!(*l.trace.lock().unwrap(), [root, ids[1], ids[2]]);
        l.trace.lock().unwrap().clear();
        // The sweep took the last-joined rider out of the list.
        let gone = game
            .resident_entities
            .general
            .rows
            .iter()
            .position(|row| row.id == ids[2])
            .unwrap();
        game.resident_entities.general.rows.remove(gone);
        tick(game);
        assert_eq!(*l.trace.lock().unwrap(), [root, ids[1]]);
    });
}

/// The same shrink with the tree's only rider: the root's own first-child
/// link pointed at the removed place, and a reused buffer keeps it.
#[test]
#[ignore = "daily"]
fn a_removed_only_rider_leaves_no_stale_index_link() {
    with_counter(|game, l| {
        let rider = record_at("mymod:robot", [9, 9, 9, 9], [0.5, 64.0, 0.5], Vec::new());
        let root = spawn_tree(game, [1, 2, 3, 4], [0.5, 64.0, 0.5], vec![rider]);
        let ids = row_ids(game);
        tick(game);
        assert_eq!(*l.trace.lock().unwrap(), [root, ids[1]]);
        l.trace.lock().unwrap().clear();
        let gone = game
            .resident_entities
            .general
            .rows
            .iter()
            .position(|row| row.id == ids[1])
            .unwrap();
        game.resident_entities.general.rows.remove(gone);
        tick(game);
        assert_eq!(*l.trace.lock().unwrap(), [root]);
    });
}

/// A row a listener discarded from an earlier row's callback gets no
/// callback of its own: `entity_remove` queues the removal and invalidates
/// the handle, and the row stays in the list until the sweep — the walk
/// re-checks liveness at every step, so it ticks nothing more.
#[test]
#[ignore = "daily"]
fn a_listener_discarded_row_gets_no_callback() {
    with_counter(|game, l| {
        let first = seeded_row(game, [1, 2, 3, 4], 0);
        let second = seeded_row(game, [5, 6, 7, 8], 0);
        *l.action.lock().unwrap() = OnTick::Remove(second);
        tick(game);
        assert_eq!(*l.trace.lock().unwrap(), [first]);
        // Still in the list — no sweep has run — but its removal is queued.
        assert_eq!(game.resident_entities.general.rows.len(), 2);
        assert!(
            game.persist
                .removals
                .contains(&crate::game::commands::Target::General(second))
        );
        assert_eq!(counter(game, second), 0, "no callback ran for it");
    });
}

/// A rider a listener discarded from its vehicle's callback does not tick,
/// and neither does the rider's own subtree: the edge is re-checked at the
/// parent's callback (`tickPassenger`'s `getVehicle` gate).
#[test]
#[ignore = "daily"]
fn a_listener_discarded_rider_stops_its_subtree() {
    with_counter(|game, l| {
        let grandchild = record_at("mymod:robot", [8, 8, 8, 8], [0.5, 64.0, 0.5], Vec::new());
        let rider = record_at(
            "mymod:robot",
            [9, 9, 9, 9],
            [0.5, 64.0, 0.5],
            vec![grandchild],
        );
        let sibling = record_at("mymod:robot", [7, 7, 7, 7], [0.5, 64.0, 0.5], Vec::new());
        let root = spawn_tree(game, [1, 2, 3, 4], [0.5, 64.0, 0.5], vec![rider, sibling]);
        let ids = row_ids(game);
        *l.action.lock().unwrap() = OnTick::Remove(ids[1]);
        tick(game);
        assert_eq!(*l.trace.lock().unwrap(), [root, ids[3]]);
    });
}

/// A rider a listener took off its vehicle mid-pass walks as a root from
/// this same pass: the vehicle's subtree no longer holds it (the edge's
/// mount check), and the root walk reaches its place after the vehicle.
#[test]
#[ignore = "daily"]
fn a_listener_dismounted_rider_walks_as_a_root_after() {
    with_counter(|game, l| {
        let first = record_at("mymod:robot", [9, 9, 9, 9], [0.5, 64.0, 0.5], Vec::new());
        let second = record_at("mymod:robot", [7, 7, 7, 7], [0.5, 64.0, 0.5], Vec::new());
        let root = spawn_tree(game, [1, 2, 3, 4], [0.5, 64.0, 0.5], vec![first, second]);
        let ids = row_ids(game);
        *l.action.lock().unwrap() = OnTick::Dismount(ids[1]);
        tick(game);
        assert_eq!(*l.trace.lock().unwrap(), [root, ids[2], ids[1]]);
        assert_eq!(
            game.resident_entities.general.rows[1].mount, None,
            "the rider got off"
        );
    });
}

/// A row discarded from another row's Pre never reaches its own: no
/// bookkeeping, no Pre of its own, no callback.
#[test]
#[ignore = "daily"]
fn a_row_discarded_from_an_earlier_pre_ticks_nothing() {
    with_counter(|game, l| {
        let first = seeded_row(game, [1, 2, 3, 4], 0);
        let second = seeded_row(game, [5, 6, 7, 8], 0);
        l.pre_on.store(first, std::sync::atomic::Ordering::Relaxed);
        l.pre_victim
            .store(second, std::sync::atomic::Ordering::Relaxed);
        tick(game);
        assert_eq!(*l.trace.lock().unwrap(), [first]);
        assert_eq!(counter(game, second), 0);
    });
}

/// The re-entry oracle: vanilla's `EntityTickList.add` appends to the active
/// map's end, and a level's entities re-enter ticking through the section
/// callbacks when their chunk loads again, so a row that left and returned
/// ticks after a row that joined later and never left. Horizon's join order
/// is that insertion order (Ari's arismoko/horizon#792 ruling); the row's fresh admission on
/// reload is its re-entry.
#[test]
#[ignore = "daily"]
fn a_reloaded_row_reenters_ticking_behind_later_joins() {
    with_counter(|game, l| {
        let early = spawn_tree(game, [1, 2, 3, 4], [0.5, 64.0, 0.5], Vec::new());
        let late = spawn_tree(game, [5, 6, 7, 8], [1024.5, 64.0, 0.5], Vec::new());
        tick(game);
        assert_eq!(*l.trace.lock().unwrap(), [early, late]);
        // The early row's chunk unloads (the later row's stays loaded).
        let mut chunks = game.entity_chunks(&HashSet::from([(0, 0)]), true);
        let (_, mut records) = chunks.remove(0);
        assert_eq!(records.len(), 1);
        l.trace.lock().unwrap().clear();
        // It loads back into a fresh join, behind the row that never left.
        game.load_listed("mymod:robot", None, None, records.remove(0))
            .unwrap();
        let rows = &game.resident_entities.general.rows;
        let reloaded = rows.iter().find(|row| row.uuid == [1, 2, 3, 4]).unwrap();
        let (early_id, early_join) = (reloaded.id, reloaded.join);
        let late_join = rows
            .iter()
            .find(|row| row.uuid == [5, 6, 7, 8])
            .unwrap()
            .join;
        assert!(
            late_join < early_join,
            "the reloaded row rejoined at the end"
        );
        tick(game);
        assert_eq!(*l.trace.lock().unwrap(), [late, early_id]);
    });
}

/// The save row: a row saved at 41 loads back with its counter and ticks 42
/// from the next phase; its old handle — the id is spent, never reused
/// (`next_entity` only rises) — no longer reaches the row.
#[test]
#[ignore = "daily"]
fn a_reloaded_row_resumes_the_counter_and_refuses_its_old_handle() {
    with_counter(|game, _| {
        let old = seeded_row(game, [1, 2, 3, 4], 41);
        let mut chunks = game.entity_chunks(&HashSet::from([(0, 0)]), true);
        let (_, mut records) = chunks.remove(0);
        game.load_listed("mymod:robot", None, None, records.remove(0))
            .unwrap();
        let rows = &game.resident_entities.general.rows;
        let fresh = rows[0].id;
        assert_ne!(fresh, old);
        assert_eq!(counter(game, fresh), 41);
        assert!(COUNTER.get(game, Holder::Entity(old)).is_err());
        tick(game);
        assert_eq!(counter(game, fresh), 42);
    });
}

/// The parking row: a parked level's rows persist untouched — the pass only
/// runs for the level in place — and the first tick after it returns gives
/// the next count.
#[test]
#[ignore = "daily"]
fn a_parked_level_holds_the_counter_until_it_returns() {
    with_counter(|game, l| {
        let id = seeded_row(game, [1, 2, 3, 4], 41);
        game.enter_level(crate::game::Dim::Nether).unwrap();
        for _ in 0..10 {
            tick(game);
        }
        assert!(l.trace.lock().unwrap().is_empty(), "nothing ticked");
        // The row's counter reads through its own level's routing, so read it
        // once its level is back in place: ten parked ticks left it, and the
        // first tick after the return takes it.
        game.enter_level(crate::game::Dim::Overworld).unwrap();
        assert_eq!(counter(game, id), 41, "the parked level's rows do not tick");
        tick(game);
        assert_eq!(counter(game, id), 42);
    });
}

/// The containment row: a native panic in a row's tick rolls the row's
/// writes back (the counter keeps its value), the later row ticks on, and
/// the panicking row's own next tick proceeds. A subscriber is not
/// auto-disabled.
#[test]
#[ignore = "daily"]
fn a_panicking_row_rolls_back_and_the_next_tick_proceeds() {
    with_counter(|game, l| {
        let first = seeded_row(game, [1, 2, 3, 4], 41);
        let second = seeded_row(game, [5, 6, 7, 8], 7);
        l.panic_on
            .store(first, std::sync::atomic::Ordering::Relaxed);
        tick(game);
        assert_eq!(
            *l.trace.lock().unwrap(),
            [second],
            "only the later row ticked"
        );
        assert_eq!(counter(game, first), 41, "the panic rolled the write back");
        assert_eq!(counter(game, second), 8);
        tick(game);
        assert_eq!(*l.trace.lock().unwrap(), [second, first, second]);
        assert_eq!(counter(game, first), 42);
    });
}

/// The ruling's flat order: the vehicle's Pre, tick and Post, then each
/// passenger's own triple, then that passenger's passengers, depth first —
/// a branching three-generation tree spells its whole pass out.
#[test]
#[ignore = "daily"]
fn every_row_gets_its_pre_tick_post_in_the_flat_rider_order() {
    with_counter(|game, l| {
        let grandchild = record_at("mymod:robot", [8, 8, 8, 8], [0.5, 64.0, 0.5], Vec::new());
        let rider = record_at(
            "mymod:robot",
            [9, 9, 9, 9],
            [0.5, 64.0, 0.5],
            vec![grandchild],
        );
        let sibling = record_at("mymod:robot", [7, 7, 7, 7], [0.5, 64.0, 0.5], Vec::new());
        let root = spawn_tree(game, [1, 2, 3, 4], [0.5, 64.0, 0.5], vec![rider, sibling]);
        let ids = row_ids(game);
        tick(game);
        assert_eq!(
            *l.events.lock().unwrap(),
            [
                (root, "pre"),
                (root, "tick"),
                (root, "post"),
                (ids[1], "pre"),
                (ids[1], "tick"),
                (ids[1], "post"),
                (ids[2], "pre"),
                (ids[2], "tick"),
                (ids[2], "post"),
                (ids[3], "pre"),
                (ids[3], "tick"),
                (ids[3], "post"),
            ]
        );
    });
}

/// The ruling's cancel rules, the vehicle's side: a canceled vehicle Pre
/// skips only the vehicle — no bookkeeping, no tick, no Post — and its
/// passengers still ride their own ticks.
#[test]
#[ignore = "daily"]
fn a_canceled_vehicle_pre_still_ticks_its_passengers() {
    with_counter(|game, l| {
        let rider = record_at("mymod:robot", [9, 9, 9, 9], [0.5, 64.0, 0.5], Vec::new());
        let root = spawn_tree(game, [1, 2, 3, 4], [0.5, 64.0, 0.5], vec![rider]);
        let rider_id = row_ids(game)[1];
        tick(game);
        l.events.lock().unwrap().clear();
        l.cancel_on
            .store(root, std::sync::atomic::Ordering::Relaxed);
        tick(game);
        assert_eq!(
            *l.events.lock().unwrap(),
            [
                (root, "pre"),
                (rider_id, "pre"),
                (rider_id, "tick"),
                (rider_id, "post"),
            ]
        );
        l.cancel_on.store(0, std::sync::atomic::Ordering::Relaxed);
    });
}

/// The ruling's cancel rules, the passenger's side: a canceled passenger
/// Pre skips only its entity tick and its Post — it still follows its
/// vehicle, the bookkeeping runs, and its own passengers still tick. A
/// stack of three covers the depth.
#[test]
#[ignore = "daily"]
fn a_canceled_passenger_still_follows_and_his_passengers_tick() {
    with_counter(|game, l| {
        let bottom = record_at("mymod:robot", [8, 8, 8, 8], [0.5, 64.0, 0.5], Vec::new());
        let middle = record_at("mymod:robot", [9, 9, 9, 9], [0.5, 64.0, 0.5], vec![bottom]);
        let root = spawn_tree(game, [1, 2, 3, 4], [0.5, 64.0, 0.5], vec![middle]);
        let ids = row_ids(game);
        tick(game);
        l.events.lock().unwrap().clear();
        l.cancel_on
            .store(ids[1], std::sync::atomic::Ordering::Relaxed);
        tick(game);
        assert_eq!(
            *l.events.lock().unwrap(),
            [
                (root, "pre"),
                (root, "tick"),
                (root, "post"),
                (ids[1], "pre"),
                (ids[2], "pre"),
                (ids[2], "tick"),
                (ids[2], "post"),
            ]
        );
        // The canceled passenger still follows its vehicle: its bookkeeping
        // ran, and only its own counter stood still.
        assert_eq!(counter(game, ids[1]), 1);
        assert_eq!(counter(game, root), 2);
        assert_eq!(counter(game, ids[2]), 2);
        l.cancel_on.store(0, std::sync::atomic::Ordering::Relaxed);
    });
}

/// A rider removed in its own Pre — the removal the liveness re-check
/// exists for: it neither ticks nor walks its descendants, however its Pre
/// ended. A stack of three covers the depth: the middle rider's own Pre
/// discards it, and the bottom row hears nothing at all.
#[test]
#[ignore = "daily"]
fn a_rider_removed_in_its_own_pre_walks_no_descendants() {
    with_counter(|game, l| {
        let bottom = record_at("mymod:robot", [8, 8, 8, 8], [0.5, 64.0, 0.5], Vec::new());
        let middle = record_at("mymod:robot", [9, 9, 9, 9], [0.5, 64.0, 0.5], vec![bottom]);
        let root = spawn_tree(game, [1, 2, 3, 4], [0.5, 64.0, 0.5], vec![middle]);
        let ids = row_ids(game);
        tick(game);
        l.events.lock().unwrap().clear();
        // The middle rider's own Pre discards it.
        l.pre_on.store(ids[1], std::sync::atomic::Ordering::Relaxed);
        l.pre_victim
            .store(ids[1], std::sync::atomic::Ordering::Relaxed);
        tick(game);
        assert_eq!(
            *l.events.lock().unwrap(),
            [
                (root, "pre"),
                (root, "tick"),
                (root, "post"),
                (ids[1], "pre"),
            ],
            "the removed rider neither ticks nor walks its descendants"
        );
        assert_eq!(counter(game, ids[1]), 1);
        assert_eq!(counter(game, ids[2]), 1, "the descendant did not tick");
        assert_eq!(counter(game, root), 2);
    });
}

/// The same removal with the Pre also canceled: the liveness re-check runs
/// after a canceled Pre too — the cancel skips the rider's tick and Post,
/// not the check — so a rider that removed itself in its canceled Pre
/// walks no descendants, and a live one keeps the ruling's follow.
#[test]
#[ignore = "daily"]
fn a_rider_removed_in_its_own_canceled_pre_walks_no_descendants() {
    with_counter(|game, l| {
        let bottom = record_at("mymod:robot", [8, 8, 8, 8], [0.5, 64.0, 0.5], Vec::new());
        let middle = record_at("mymod:robot", [9, 9, 9, 9], [0.5, 64.0, 0.5], vec![bottom]);
        let root = spawn_tree(game, [1, 2, 3, 4], [0.5, 64.0, 0.5], vec![middle]);
        let ids = row_ids(game);
        tick(game);
        l.events.lock().unwrap().clear();
        // The middle rider's own Pre discards it and cancels itself.
        l.cancel_on
            .store(ids[1], std::sync::atomic::Ordering::Relaxed);
        l.pre_on.store(ids[1], std::sync::atomic::Ordering::Relaxed);
        l.pre_victim
            .store(ids[1], std::sync::atomic::Ordering::Relaxed);
        tick(game);
        assert_eq!(
            *l.events.lock().unwrap(),
            [
                (root, "pre"),
                (root, "tick"),
                (root, "post"),
                (ids[1], "pre"),
            ]
        );
        assert_eq!(counter(game, ids[2]), 1, "the descendant did not tick");
        assert_eq!(counter(game, root), 2);
        l.cancel_on.store(0, std::sync::atomic::Ordering::Relaxed);
    });
}

/// The zero-callback row, with riders: five hundred two-row trees of a
/// type no part subscribes to cost the pass one walk and no callback — no
/// call, no event, no type lookup (the slot resolved without subscribers
/// before the walk began) — and each tree costs its one edge through the
/// rider index, not a scan of the rows. A stable pass allocates nothing:
/// the index's buffers hold their capacity across ticks.
#[test]
#[ignore = "daily"]
fn five_hundred_part_free_trees_walk_each_edge_once_and_allocate_nothing() {
    with_counter(|game, l| {
        for index in 0..500u32 {
            let rider = record_at(
                "mymod:ghost",
                [index as i32, 1, 0, 0],
                [0.5, 64.0, 0.5],
                Vec::new(),
            );
            game.load_listed(
                "mymod:ghost",
                None,
                None,
                record_at(
                    "mymod:ghost",
                    [index as i32, 0, 0, 0],
                    [0.5, 64.0, 0.5],
                    vec![rider],
                ),
            )
            .unwrap();
        }
        assert_eq!(game.resident_entities.general.rows.len(), 1000);
        tick(game);
        assert!(l.trace.lock().unwrap().is_empty());
        // The second pass over unchanged rows reuses the index buffers:
        // their capacity does not grow, which is the allocation the walk
        // itself would have to make.
        let capacity = game.resident_entities.general.riders.first_child.capacity();
        tick(game);
        assert_eq!(
            game.resident_entities.general.riders.first_child.capacity(),
            capacity,
            "a stable pass allocates nothing"
        );
        assert_eq!(game.resident_entities.general.rows.len(), 1000);
    });
}

/// arismoko/horizon#3458's first uncovered test, on the tick's own fixture: a row of a
/// missing mod quarantines byte for byte, survives a second save unchanged,
/// and reinstating the mod restores the row with its counter — 41 resumed,
/// 42 on the next tick.
#[test]
#[ignore = "daily"]
fn quarantine_then_reinstall_restores_the_counter() {
    // With the mod: spawn at 41 and save the record out.
    let mut saved = None;
    with_counter(|game, _| {
        let id = seeded_row(game, [1, 2, 3, 4], 41);
        tick(game);
        assert_eq!(counter(game, id), 42);
        let mut chunks = game.entity_chunks(&HashSet::from([(0, 0)]), true);
        let (_, mut records) = chunks.remove(0);
        saved = Some(records.remove(0));
    });
    let saved = saved.unwrap();
    // Without it: the record quarantines (its namespace belongs to no
    // loaded package), and a save of that chunk writes the very same bytes
    // back.
    with_game(|game| {
        game.others.clear();
        game.spawn_loaded_entity((0, 0), saved.clone(), None);
        assert_eq!(game.others.len(), 1);
        let mut chunks = game.entity_chunks(&HashSet::from([(0, 0)]), false);
        let (_, mut records) = chunks.remove(0);
        assert_eq!(records.remove(0), saved, "the second save keeps the bytes");
    });
    // With the mod again: the row returns as itself, counter and all.
    with_counter(|game, l| {
        game.load_listed("mymod:robot", None, None, saved).unwrap();
        let rows = &game.resident_entities.general.rows;
        let id = rows[0].id;
        assert_eq!(rows[0].uuid, [1, 2, 3, 4]);
        assert_eq!(counter(game, id), 42);
        tick(game);
        assert_eq!(*l.trace.lock().unwrap(), [id]);
        assert_eq!(counter(game, id), 43);
    });
}

/// arismoko/horizon#3458's second uncovered test: a summon's insertion of a general type —
/// the path `/summon` drives, `spawn_entity_from_nbt` — answers a target
/// whose network id and position read back (a general row's network id is
/// its own id, until stage 5's tracking gives it a connection-local one).
/// The command's own argument parser still names only vanilla entity types;
/// feeding it the declared general types is the negotiation's follow-up.
#[test]
#[ignore = "daily"]
fn summon_answers_a_target_with_a_network_id() {
    with_counter(|game, _| {
        let mut tag = record_at("mymod:robot", [1, 2, 3, 4], [0.5, 64.0, 0.5], Vec::new());
        if let Tag::Compound(map) = &mut tag {
            map.insert("id".into(), Tag::String("mymod:robot".into()));
        }
        let target = game.spawn_entity_from_nbt(tag).unwrap();
        let rows = &game.resident_entities.general.rows;
        assert_eq!(rows.len(), 1);
        let id = rows[0].id;
        assert_eq!(target, crate::game::commands::Target::General(id));
        assert_eq!(
            game.network_entity_id(crate::game::commands::Target::General(id)),
            Some(id as i32)
        );
        assert_eq!(
            game.target_position(crate::game::commands::Target::General(id)),
            Some([0.5, 64.0, 0.5])
        );
    });
}
