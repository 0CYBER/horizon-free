//! Entity Part state (arismoko/horizon#796 stage 3) on real entities: one value each, kept with its entity
//! through edits, transactions, removal, unloading, reloading and changes of level.
use std::{collections::HashSet, panic::AssertUnwindSafe, sync::Arc};

use horizon_client::chunk::Chunk;
use horizon_core::{
    geometry::Vec3,
    nbt::{Compound, Tag},
};
use horizon_mod_runtime::attachments::{AttachmentCodec, AttachmentError, Attachments, Holder};
use mod_api::{
    AttachmentType, EntityPart, EntityPartState, EntityRef, Hooks, HostTransaction, PartStateError,
    Registrar, ServerHost, WorldEpoch, edit_part_state, read_state,
};

use super::{Dim, Game};

static CHARGE: AttachmentType<i32> = AttachmentType {
    namespace: "part_state",
    name: "charge",
    default: || 0,
    codec: Some(AttachmentCodec {
        encode: |n| Ok([("n".into(), Tag::Int(*n))].into()),
        decode: |saved: &Compound| match saved.get(&"n".into()) {
            Some(Tag::Int(n)) => Ok(*n),
            _ => Err(AttachmentError::Decode),
        },
    }),
    copy_on_death: false,
    sync: false,
};

/// Kept for the session only: no codec.
static SCRATCH: AttachmentType<i32> = AttachmentType {
    namespace: "part_state",
    name: "scratch",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync: false,
};

/// Keeps a charge on each entity of its types.
struct Charge;
#[mod_api::horizon_part]
impl EntityPart for Charge {
    fn register_goals(&self, goals: &mut mod_api::GoalRegistrar<'_>) {
        let _ = goals;
    }
}
impl EntityPartState for Charge {
    type Data = i32;
    const DATA: &'static AttachmentType<i32> = &CHARGE;
}

/// Declares the general type `part_state:robot` and attaches `Charge` with state to two mob
/// types and to the robot; the `refused` case also attaches it to the refused kind `refused`.
struct StateMod {
    refused: Option<&'static str>,
}
#[mod_api::__subscriptions]
impl Hooks for StateMod {
    fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
        r.declare_general_entity(
            "robot",
            mod_api::GeneralEntityCategory::Misc,
            0.8,
            1.9,
            mod_api::GeneralSavePolicy::Save,
            true,
            8,
            3,
            1,
            &[],
        )
        .unwrap();
        for kind in ["minecraft:cow", "minecraft:pig", "part_state:robot"]
            .into_iter()
            .chain(self.refused)
        {
            r.entity_part_with_state(kind, Charge).unwrap();
        }
    }
}

/// Runs the test `name` in a fresh process (installed mods are process-wide) with `StateMod`
/// installed, on a dedicated server's overworld with chunk 0 0 loaded.
fn with_state_mod(name: &str, test: impl FnOnce(&mut Game<'_>)) {
    if super::native_entity_refs::tests::run_fresh_cases(
        name,
        "HORIZON_ENTITY_PART_STATE_CHILD",
        &["valid"],
    ) {
        return;
    }
    with_mod(StateMod { refused: None }, |game| {
        // Registration is over once the game is built: the test runs in the world's phase.
        let _world = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::World);
        game.options.dedicated = Some(Vec::new());
        game.native_level_opened(Dim::Overworld);
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.native_joins_skip();
        test(game);
    });
}

/// `test` on a game with `state_mod` installed. The registration phase is scoped around the
/// whole fixture, since the game fixture registers the installed mods again and a general type
/// is only declared while that phase is current.
fn with_mod(state_mod: StateMod, test: impl FnOnce(&mut Game<'_>)) {
    mod_platform::__private::initialize().expect("test registration requires the mod API runtime");
    let _phase = mod_api::__private::PhaseOwner::scope(mod_api::__private::ModPhase::Registration);
    let mods: mod_api::Mods =
        vec![mod_api::ModEntry::new("part_state", Box::new(state_mod))].into();
    mod_platform::testing::with_mods(mods, || {
        super::native_entity_refs::tests::with_game(test);
    });
}

/// A saved mob of `kind` with `uuid` at 1 70 3, in chunk 0 0.
fn mob_tag(kind: &str, uuid: i32) -> Tag {
    Tag::compound([
        ("id", Tag::String(kind.into())),
        ("UUID", Tag::IntArray(vec![uuid, 0, 0, 1])),
        (
            "Pos",
            Tag::List(
                [1.0f64, 70.0, 3.0]
                    .map(|n| Tag::Double(n.to_bits()))
                    .to_vec(),
            ),
        ),
    ])
}

/// A fresh mob of `kind` in the level the game is in, and its handle.
fn mob(game: &mut Game<'_>, kind: &str, uuid: i32) -> EntityRef {
    let id = game.load_mob(mob_tag(kind, uuid), None).unwrap();
    game.entity(id).unwrap()
}

/// A saved `part_state:robot` record with `uuid`.
fn robot_tag(uuid: i32) -> Tag {
    super::general::GeneralEntityRecord {
        position: [0.5, 64.0, 0.5],
        velocity: [0.0; 3],
        rotation: [0.0, 90.0],
        uuid: [uuid, 0, 0, 2],
        envelope: super::general::GeneralTypeEnvelope {
            type_key: "part_state:robot".to_owned(),
            payload_version: 1,
            payload_bytes: Vec::new(),
        },
        passengers: Vec::new(),
    }
    .to_tag()
}

fn stored(game: &mut Game<'_>, entity: EntityRef) -> Option<i32> {
    Charge::state_existing(game, entity).unwrap()
}

/// What `level`'s attachment index holds for `CHARGE`, by entity id.
fn indexed(game: &Game<'_>, level: Dim) -> Vec<(u32, i32)> {
    let Some(index) = game.attachment_indexes.get(&level) else {
        return Vec::new();
    };
    let mut rows: Vec<_> = index
        .lock()
        .unwrap()
        .snapshot(&("part_state".into(), "charge".into()))
        .iter()
        .map(|row| {
            let mod_api::AttachmentHolder::Entity(entity) = row.holder else {
                panic!("not an entity row");
            };
            (entity.id(), *row.entry.read(&CHARGE).unwrap())
        })
        .collect();
    rows.sort();
    rows
}

#[test]
#[ignore = "daily"]
fn entity_part_state_is_per_entity_through_edits_transactions_and_panics() {
    const NAME: &str = "game::entity_part_state_tests::entity_part_state_is_per_entity_through_edits_transactions_and_panics";
    with_state_mod(NAME, |game| {
        let (a, b) = (
            mob(game, "minecraft:cow", 11),
            mob(game, "minecraft:cow", 12),
        );
        // Reads insert nothing: an entity no edit reached keeps no map at all.
        assert_eq!(Charge::state(game, a), Ok(0));
        assert_eq!(stored(game, a), None);
        assert!(!game.entity_attachments.contains_key(&a.id()));
        assert_eq!(Charge::edit_state(game, a, |_, n| *n = 7), Ok(()));
        assert_eq!((stored(game, a), stored(game, b)), (Some(7), None));
        assert_eq!(
            indexed(game, Dim::Overworld),
            [(a.id(), 7)],
            "bound at first store"
        );

        // A handle of another world open is refused, and creates nothing.
        let foreign = EntityRef::new(WorldEpoch::new(a.epoch().get() + 1), b.id());
        assert_eq!(Charge::state(game, foreign), Err(PartStateError::Stale));
        assert_eq!(
            Charge::edit_state(game, foreign, |_, n| *n = 1),
            Err(PartStateError::Stale)
        );
        assert!(!game.entity_attachments.contains_key(&b.id()));

        // An inner edit commits into the outer transaction, which then aborts: the value goes
        // back, and with it the save gate.
        let aborted: Result<(), ()> = (game as &mut dyn ServerHost).transaction(|host, _| {
            assert_eq!(Charge::edit_state(host, a, |_, n| *n = 11), Ok(()));
            assert_eq!(Charge::state(host, a), Ok(11), "later steps see it");
            assert!(
                !horizon_client::join_seq::visibility::passes_settled(),
                "no save while it is tentative"
            );
            Err(())
        });
        assert_eq!(aborted, Err(()));
        assert_eq!(stored(game, a), Some(7));
        assert!(horizon_client::join_seq::visibility::passes_settled());
        assert_eq!(indexed(game, Dim::Overworld), [(a.id(), 7)]);

        // A panic inside an edit leaves no default behind.
        let host: &mut dyn ServerHost = game;
        let panicked = std::panic::catch_unwind(AssertUnwindSafe(|| {
            Charge::edit_state(host, b, |_, n| {
                *n = 5;
                panic!("edit fails");
            })
        }));
        assert!(panicked.is_err());
        assert!(game.transactions.is_empty());
        assert_eq!(stored(game, b), None);

        // The same state can't be edited from inside its own edit; another key can.
        let inner = Charge::edit_state(game, a, |host, n| {
            *n = 10;
            (
                Charge::edit_state(host, a, |_, n| *n = 99),
                edit_part_state(host, a, &SCRATCH, |_, n| *n = 1),
            )
        });
        assert_eq!(inner, Ok((Err(PartStateError::Reentrant), Ok(()))));
        assert_eq!(stored(game, a), Some(10));
        assert_eq!(read_state(game, a, &SCRATCH), Ok(Some(1)));

        // An undecodable saved value refuses before the closure runs and keeps its bytes.
        let mut saved = Attachments::default();
        CHARGE.set(&mut saved, Holder::Level, 0).unwrap();
        let mut raw = saved.to_nbt();
        let Some(Tag::Compound(names)) = raw.get_mut(&"part_state".into()) else {
            unreachable!()
        };
        let Some(Tag::Compound(entry)) = names.get_mut(&"charge".into()) else {
            unreachable!()
        };
        entry.insert(
            "value".into(),
            Tag::Compound([("n".into(), Tag::String("ten".into()))].into()),
        );
        game.entity_attachments
            .insert(b.id(), Attachments::from_nbt(raw.clone()));
        game.bind_resident_entity_attachments(b.id()).unwrap();
        let decode = PartStateError::Attachment {
            namespace: "part_state",
            name: "charge",
            error: AttachmentError::Decode,
        };
        assert_eq!(Charge::state(game, b), Err(decode));
        let mut ran = false;
        assert_eq!(Charge::edit_state(game, b, |_, _| ran = true), Err(decode));
        assert!(!ran);
        assert_eq!(game.entity_attachments[&b.id()].to_nbt(), raw);

        // A mob out of its list for its own tick can't be reached until it is back.
        let taken = game.mobs.lift();
        assert_eq!(Charge::state(game, a), Err(PartStateError::HolderBusy));
        assert_eq!(
            Charge::edit_state(game, a, |_, n| *n = 1),
            Err(PartStateError::HolderBusy)
        );
        game.mobs.restore(
            taken,
            horizon_client::join_seq::RestoreOrder::SurvivorsFirst,
        );
        assert_eq!(stored(game, a), Some(10));

        // A handle answers only in its own level.
        game.enter_level(Dim::Nether).unwrap();
        assert_eq!(Charge::state(game, a), Err(PartStateError::OtherLevel));
        assert_eq!(
            Charge::edit_state(game, a, |_, n| *n = 1),
            Err(PartStateError::OtherLevel)
        );
        game.enter_level(Dim::Overworld).unwrap();
        assert_eq!(stored(game, a), Some(10));
    });
}

#[test]
#[ignore = "daily"]
fn a_removed_or_unloaded_entity_is_stale_and_a_reloaded_one_reads_its_saved_value() {
    const NAME: &str = "game::entity_part_state_tests::a_removed_or_unloaded_entity_is_stale_and_a_reloaded_one_reads_its_saved_value";
    with_state_mod(NAME, |game| {
        // No declaration means no allocation: a mob type no Part keeps state on gets no
        // attachment map and no index row from its spawn, a tick and a save.
        let sheep = mob(game, "minecraft:sheep", 20);
        game.tickets.update(None);
        game.tick_creature_entities().unwrap();
        let saved = game.entity_chunks(&HashSet::from([(0, 0)]), false);
        assert!(
            saved
                .iter()
                .flat_map(|(_, tags)| tags)
                .any(|tag| horizon_client::entity_nbt::uuid(tag) == Some([20, 0, 0, 1])),
            "saved"
        );
        assert!(game.entity(sheep.id()).is_some(), "still in its level");
        assert!(!game.entity_attachments.contains_key(&sheep.id()));
        assert!(indexed(game, Dim::Overworld).is_empty(), "no index row");

        // Removed, then a new cow: the old handle is stale, the new one reads the default.
        let gone = mob(game, "minecraft:cow", 21);
        assert_eq!(Charge::edit_state(game, gone, |_, n| *n = 3), Ok(()));
        assert!(game.entity_discard(gone));
        game.native_removals_flush();
        assert_eq!(Charge::state(game, gone), Err(PartStateError::Stale));
        let again = mob(game, "minecraft:cow", 22);
        assert_ne!(again.id(), gone.id());
        assert_eq!(Charge::state(game, again), Ok(0));

        // Unloaded by the residency owner's walk, which writes each map into the entity's NBT
        // before the rows go, then loaded back from that NBT under a fresh id.
        assert_eq!(Charge::edit_state(game, again, |_, n| *n = 7), Ok(()));
        assert_eq!(
            edit_part_state(game, again, &SCRATCH, |_, n| *n = 4),
            Ok(())
        );
        let saved = game.entity_chunks(&HashSet::from([(0, 0)]), true);
        assert_eq!(Charge::state(game, again), Err(PartStateError::Stale));
        assert!(indexed(game, Dim::Overworld).is_empty());
        let saved = saved
            .into_iter()
            .flat_map(|(_, tags)| tags)
            .find(|tag| horizon_client::entity_nbt::uuid(tag) == Some([22, 0, 0, 1]))
            .unwrap();
        game.entity_attachments.remove(&again.id());
        game.world
            .loaded_entities
            .push(((0, 0), vec![saved.into()]));
        game.spawn_loaded_entities();
        assert_eq!(game.mobs.len(), 1, "the reloaded cow alone");
        let reloaded = game.mobs[0].id;
        let reloaded = game.entity(reloaded).unwrap();
        assert_ne!(reloaded, again);
        assert_eq!(Charge::state(game, again), Err(PartStateError::Stale));
        assert_eq!(stored(game, reloaded), Some(7));
        assert_eq!(
            read_state(game, reloaded, &SCRATCH),
            Ok(None),
            "session only"
        );
    });
}

#[test]
#[ignore = "daily"]
fn a_spawn_or_removal_inside_an_aborted_action_keeps_the_bindings() {
    const NAME: &str = "game::entity_part_state_tests::a_spawn_or_removal_inside_an_aborted_action_keeps_the_bindings";
    with_state_mod(NAME, |game| {
        let cow = mob(game, "minecraft:cow", 31);
        assert_eq!(Charge::edit_state(game, cow, |_, n| *n = 7), Ok(()));

        // Removed inside an action: stale at once, but bound until the root commits; the abort
        // brings it back with its value.
        let seal = game.transaction_open();
        assert!(game.entity_discard(cow));
        assert_eq!(Charge::state(game, cow), Err(PartStateError::Stale));
        assert_eq!(
            Charge::edit_state(game, cow, |_, n| *n = 1),
            Err(PartStateError::Stale)
        );
        assert_eq!(
            indexed(game, Dim::Overworld),
            [(cow.id(), 7)],
            "nothing retired yet"
        );
        // A spawn inside it exists at once and keeps state; the abort removes both.
        let (Tag::Compound(nbt), _) = horizon_core::nbt::parse_snbt("{}").unwrap() else {
            unreachable!()
        };
        let pig = game
            .summon(
                "minecraft:overworld",
                "minecraft:pig",
                [1.5, 70.0, 1.5],
                nbt,
            )
            .unwrap();
        let pig = game.entity(pig).unwrap();
        assert_eq!(Charge::edit_state(game, pig, |_, n| *n = 5), Ok(()));
        assert_eq!(
            indexed(game, Dim::Overworld),
            [(cow.id(), 7), (pig.id(), 5)]
        );
        game.transaction_close(&seal, false);
        assert_eq!(Charge::state(game, cow), Ok(7));
        assert_eq!(Charge::state(game, pig), Err(PartStateError::Stale));
        assert!(!game.entity_attachments.contains_key(&pig.id()));
        assert_eq!(indexed(game, Dim::Overworld), [(cow.id(), 7)]);

        // A nested removal committed into its parent still waits; the root commit retires it.
        let outer = game.transaction_open();
        let inner = game.transaction_open();
        assert!(game.entity_discard(cow));
        game.transaction_close(&inner, true);
        assert_eq!(indexed(game, Dim::Overworld), [(cow.id(), 7)]);
        game.transaction_close(&outer, true);
        assert_eq!(Charge::state(game, cow), Err(PartStateError::Stale));
        // The removal ran at the root commit; its row goes when the mob does, at the sweep.
        game.native_removals_flush();
        game.tickets.update(None);
        game.tick_creature_entities().unwrap();
        assert!(game.mobs.iter().all(|m| m.id != cow.id()));
        assert!(indexed(game, Dim::Overworld).is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn a_lone_mob_or_general_row_moves_under_a_new_id_and_a_ridden_pig_keeps_its_handle() {
    const NAME: &str = "game::entity_part_state_tests::a_lone_mob_or_general_row_moves_under_a_new_id_and_a_ridden_pig_keeps_its_handle";
    with_state_mod(NAME, |game| {
        // A lone mob: rebuilt under a new id in the other level, its state with it.
        let cow = mob(game, "minecraft:cow", 41);
        assert_eq!(Charge::edit_state(game, cow, |_, n| *n = 7), Ok(()));
        let state = game.take_transferred_entity_state(cow.id());
        game.mobs.retain(|m| m.id != cow.id());
        game.detach_entity(cow.id());
        assert!(indexed(game, Dim::Overworld).is_empty());
        game.enter_level(Dim::Nether).unwrap();
        let moved = game
            .load_mob_transferred(mob_tag("minecraft:cow", 41), None, Some(state))
            .unwrap();
        let moved = game.entity(moved).unwrap();
        assert_ne!(moved, cow);
        assert_eq!(stored(game, moved), Some(7));
        assert_eq!(indexed(game, Dim::Nether), [(moved.id(), 7)]);
        assert_eq!(Charge::state(game, cow), Err(PartStateError::Stale));
        assert_eq!(
            Charge::edit_state(game, cow, |_, n| *n = 1),
            Err(PartStateError::Stale)
        );
        assert!(
            !game.entity_attachments.contains_key(&cow.id()),
            "no map on the old handle"
        );
        game.enter_level(Dim::Overworld).unwrap();

        // A general row: bound at its first store, then moved with its binding.
        let next = game.next_entity;
        game.spawn_loaded_entity((0, 0), robot_tag(42), None);
        let robot = game.resident_entities.general.rows[0].id;
        assert!(robot > next);
        let robot = game.entity(robot).unwrap();
        assert_eq!(Charge::state(game, robot), Ok(0));
        assert_eq!(Charge::edit_state(game, robot, |_, n| *n = 9), Ok(()));
        assert_eq!(
            indexed(game, Dim::Overworld),
            [(robot.id(), 9)],
            "first store"
        );
        let state = game.take_transferred_entity_state(robot.id());
        game.resident_entities
            .general
            .rows
            .retain(|row| row.id != robot.id());
        game.detach_entity(robot.id());
        game.enter_level(Dim::Nether).unwrap();
        game.world
            .chunks
            .insert((0, 0), Arc::new(Chunk::empty(0, 0)));
        game.spawn_loaded_entity((0, 0), robot_tag(42), Some(state));
        let arrived = game.resident_entities.general.rows[0].id;
        let arrived = game.entity(arrived).unwrap();
        assert_ne!(arrived, robot);
        assert_eq!(stored(game, arrived), Some(9));
        assert_eq!(
            indexed(game, Dim::Nether),
            [(moved.id(), 7), (arrived.id(), 9)],
            "bound on transfer adopt"
        );
        game.enter_level(Dim::Overworld).unwrap();
        assert_eq!(Charge::state(game, robot), Err(PartStateError::Stale));

        // A saddled pig a player rides keeps its id across: a guest on it steps into a portal
        // with it, the carrier path starts the guest's trip and parks the pig, and the arrival
        // (`arrive_vehicle`, what the trip's arrival runs once the exit is found) brings it to
        // the other level under the same handle.
        let rider = super::native_player_death::tests::add_guest(game);
        let mut tag = mob_tag("minecraft:pig", 43);
        if let Tag::Compound(map) = &mut tag {
            map.insert(
                "equipment".into(),
                Tag::compound([(
                    "saddle",
                    Tag::compound([("id", Tag::String("minecraft:saddle".into()))]),
                )]),
            );
            map.insert("NoAI".into(), Tag::Byte(1));
        }
        let pig = game.load_mob(tag, None).unwrap();
        let pig = game.entity(pig).unwrap();
        assert_eq!(Charge::edit_state(game, pig, |_, n| *n = 5), Ok(()));
        let conn = game.lan.as_ref().unwrap().hub.guests[0].conn;
        assert_eq!(game.player(123), Some(rider));
        game.lan.as_mut().unwrap().hub.guests.update(0, |guest| {
            guest.vehicle = Some(pig.id());
        });
        super::native_entity_refs::tests::prepare_entity_ticking_neighborhood(
            game,
            horizon_core::position::ChunkPos::new(0, 0),
        );
        game.tickets.add_portal((0, 0));
        game.update_chunk_tickets();
        game.portals
            .entities
            .entry(pig.id())
            .or_default()
            .set_inside(
                horizon_core::position::BlockPos::new(1, 70, 3),
                horizon_client::nether_portal::ENTITY_DELAY,
            );
        game.handle_entity_portals();
        game.carry_riders_through_portals();
        assert!(
            game.on_server_trip(conn),
            "the rider's trip carries the pig"
        );
        assert!(
            game.vehicle_waits_in(Dim::Overworld),
            "the pig waits for it"
        );
        assert_eq!(
            stored(game, pig),
            Some(5),
            "still in its level while it waits"
        );
        game.enter_level(Dim::Nether).unwrap();
        game.arrive_vehicle(Dim::Overworld, pig.id(), Vec3::new(1.0, 70.0, 3.0), 0.0);
        assert_eq!(
            game.lan.as_ref().unwrap().hub.guests[0].vehicle,
            Some(pig.id())
        );
        assert_eq!(game.entity(pig.id()), Some(pig));
        assert_eq!(stored(game, pig), Some(5));
        assert_eq!(Charge::edit_state(game, pig, |_, n| *n += 1), Ok(()));
        assert_eq!(stored(game, pig), Some(6));
        assert!(indexed(game, Dim::Nether).contains(&(pig.id(), 6)));
        game.enter_level(Dim::Overworld).unwrap();
        assert_eq!(Charge::state(game, pig), Err(PartStateError::OtherLevel));
        assert!(indexed(game, Dim::Overworld).is_empty());
    });
}

#[test]
#[ignore = "daily"]
fn a_transfer_inside_an_aborted_action_leaves_the_state_with_the_entity() {
    const NAME: &str = "game::entity_part_state_tests::a_transfer_inside_an_aborted_action_leaves_the_state_with_the_entity";
    with_state_mod(NAME, |game| {
        let cow = mob(game, "minecraft:cow", 51);
        assert_eq!(Charge::edit_state(game, cow, |_, n| *n = 7), Ok(()));
        let seal = game.transaction_open();
        assert_eq!(Charge::edit_state(game, cow, |_, n| *n = 9), Ok(()));
        let state = game.take_transferred_entity_state(cow.id());
        game.mobs.retain(|m| m.id != cow.id());
        game.detach_entity(cow.id());
        assert_eq!(
            indexed(game, Dim::Overworld),
            [(cow.id(), 9)],
            "the source index keeps its row until the root closes"
        );
        game.enter_level(Dim::Nether).unwrap();
        let moved = game
            .load_mob_transferred(mob_tag("minecraft:cow", 51), None, Some(state))
            .unwrap();
        let moved = game.entity(moved).unwrap();
        assert_eq!(stored(game, moved), Some(9));
        assert_eq!(Charge::edit_state(game, moved, |_, n| *n = 11), Ok(()));
        game.enter_level(Dim::Overworld).unwrap();
        game.transaction_close(&seal, false);

        // Travel is not undone: the map stays with the cow under its new id, with the writes the
        // action made rolled back; the old handle stays stale and its row is gone.
        assert!(indexed(game, Dim::Overworld).is_empty());
        assert_eq!(Charge::state(game, cow), Err(PartStateError::Stale));
        assert!(!game.entity_attachments.contains_key(&cow.id()));
        game.enter_level(Dim::Nether).unwrap();
        assert_eq!(stored(game, moved), Some(7));
        assert_eq!(indexed(game, Dim::Nether), [(moved.id(), 7)]);

        // Two transfers in one action, A to B and B to C: the second take keeps B's map as it
        // is then (9), but the abort restores what the action first found (7), at C.
        game.enter_level(Dim::Overworld).unwrap();
        let a = mob(game, "minecraft:cow", 52);
        assert_eq!(Charge::edit_state(game, a, |_, n| *n = 7), Ok(()));
        let seal = game.transaction_open();
        assert_eq!(Charge::edit_state(game, a, |_, n| *n = 9), Ok(()));
        let state = game.take_transferred_entity_state(a.id());
        game.mobs.retain(|m| m.id != a.id());
        game.detach_entity(a.id());
        game.enter_level(Dim::Nether).unwrap();
        let b = game
            .load_mob_transferred(mob_tag("minecraft:cow", 52), None, Some(state))
            .unwrap();
        let b = game.entity(b).unwrap();
        assert_eq!(stored(game, b), Some(9));
        let state = game.take_transferred_entity_state(b.id());
        game.mobs.retain(|m| m.id != b.id());
        game.detach_entity(b.id());
        game.enter_level(Dim::Overworld).unwrap();
        let c = game
            .load_mob_transferred(mob_tag("minecraft:cow", 52), None, Some(state))
            .unwrap();
        let c = game.entity(c).unwrap();
        assert_eq!(stored(game, c), Some(9));
        game.transaction_close(&seal, false);

        assert!(game.transactions.is_empty());
        assert_eq!(stored(game, c), Some(7), "the value the action first found");
        assert_eq!(indexed(game, Dim::Overworld), [(c.id(), 7)]);
        for stale in [a, b] {
            assert_eq!(Charge::state(game, stale), Err(PartStateError::Stale));
            assert!(
                !game.entity_attachments.contains_key(&stale.id()),
                "nothing left behind"
            );
        }
        game.enter_level(Dim::Nether).unwrap();
        assert_eq!(
            indexed(game, Dim::Nether),
            [(moved.id(), 7)],
            "no row for B"
        );

        // Taken inside an action that is rolled back before the arrival is adopted, as a portal
        // arrival waits for a later pass: the arrival carries what the action first found.
        game.enter_level(Dim::Overworld).unwrap();
        let late = mob(game, "minecraft:cow", 53);
        assert_eq!(Charge::edit_state(game, late, |_, n| *n = 7), Ok(()));
        let seal = game.transaction_open();
        assert_eq!(Charge::edit_state(game, late, |_, n| *n = 9), Ok(()));
        let state = game.take_transferred_entity_state(late.id());
        game.mobs.retain(|m| m.id != late.id());
        game.detach_entity(late.id());
        game.transaction_close(&seal, false);
        assert!(game.transactions.is_empty());
        assert_eq!(
            indexed(game, Dim::Overworld),
            [(c.id(), 7)],
            "its row retired"
        );
        assert!(!game.entity_attachments.contains_key(&late.id()));
        game.enter_level(Dim::Nether).unwrap();
        let arrived = game
            .load_mob_transferred(mob_tag("minecraft:cow", 53), None, Some(state))
            .unwrap();
        let arrived = game.entity(arrived).unwrap();
        assert_eq!(stored(game, arrived), Some(7), "not the aborted 9");
        assert_eq!(
            indexed(game, Dim::Nether),
            [(moved.id(), 7), (arrived.id(), 7)]
        );

        // Both at once: A to B adopted inside the action, then B taken again and the action
        // rolled back before C arrives. The arrival carries what the action first found at A.
        game.enter_level(Dim::Overworld).unwrap();
        let first = mob(game, "minecraft:cow", 54);
        assert_eq!(Charge::edit_state(game, first, |_, n| *n = 7), Ok(()));
        let seal = game.transaction_open();
        assert_eq!(Charge::edit_state(game, first, |_, n| *n = 9), Ok(()));
        let state = game.take_transferred_entity_state(first.id());
        game.mobs.retain(|m| m.id != first.id());
        game.detach_entity(first.id());
        game.enter_level(Dim::Nether).unwrap();
        let second = game
            .load_mob_transferred(mob_tag("minecraft:cow", 54), None, Some(state))
            .unwrap();
        let second = game.entity(second).unwrap();
        assert_eq!(stored(game, second), Some(9));
        let state = game.take_transferred_entity_state(second.id());
        game.mobs.retain(|m| m.id != second.id());
        game.detach_entity(second.id());
        game.enter_level(Dim::Overworld).unwrap();
        game.transaction_close(&seal, false);
        assert!(game.transactions.is_empty());
        let third = game
            .load_mob_transferred(mob_tag("minecraft:cow", 54), None, Some(state))
            .unwrap();
        let third = game.entity(third).unwrap();
        assert_eq!(stored(game, third), Some(7), "not the aborted 9");
        for stale in [first, second] {
            assert_eq!(Charge::state(game, stale), Err(PartStateError::Stale));
            assert!(
                !game.entity_attachments.contains_key(&stale.id()),
                "nothing left behind"
            );
        }
        assert_eq!(
            indexed(game, Dim::Overworld),
            [(c.id(), 7), (third.id(), 7)]
        );
        game.enter_level(Dim::Nether).unwrap();
        assert_eq!(
            indexed(game, Dim::Nether),
            [(moved.id(), 7), (arrived.id(), 7)],
            "no row for B"
        );
    });
}

#[test]
#[ignore = "daily"]
fn entity_part_state_on_a_player_a_vanilla_family_or_an_unknown_type_refuses_the_load() {
    const NAME: &str = "game::entity_part_state_tests::entity_part_state_on_a_player_a_vanilla_family_or_an_unknown_type_refuses_the_load";
    const CHILD: &str = "HORIZON_ENTITY_PART_STATE_KIND";
    if super::native_entity_refs::tests::run_fresh_cases(
        NAME,
        CHILD,
        &["player", "boat", "minecart", "unknown"],
    ) {
        return;
    }
    let (kind, reason) = match std::env::var(CHILD).unwrap().as_str() {
        "player" => (
            "minecraft:player",
            "players: respawn identity is not decided",
        ),
        "boat" => ("minecraft:oak_boat", "no entity part runs on this kind"),
        "minecart" => ("minecraft:minecart", "no entity part runs on this kind"),
        _ => ("part_state:nothing", "no such entity type"),
    };
    let loaded = std::panic::catch_unwind(AssertUnwindSafe(|| {
        with_mod(
            StateMod {
                refused: Some(kind),
            },
            |_| {},
        );
    }));
    let panic = loaded.expect_err("the load is refused");
    let message = panic.downcast_ref::<String>().cloned().unwrap_or_default();
    let refusal = format!(
        "part_state: entity part {}::Charge on {kind}: state part_state:charge of type i32 \
         refused: entity type {kind}: {reason}",
        module_path!()
    );
    assert!(message.contains("Mod parts refused:"), "{message}");
    assert!(message.contains(&refusal), "{message}");
    // The accepted kinds of the same mod are not named.
    for accepted in ["minecraft:cow", "minecraft:pig", "part_state:robot"] {
        assert!(!message.contains(&format!("on {accepted}:")), "{message}");
    }
}
