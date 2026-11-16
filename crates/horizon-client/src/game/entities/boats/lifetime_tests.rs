use super::*;
use horizon_mod_runtime::attachments::{
    AttachmentType, Attachments, HasAttachmentLifetime, Holder, index::AttachmentIndex,
};
use std::sync::Mutex;

static VALUE: AttachmentType<i32> = AttachmentType {
    namespace: "test",
    name: "boat_lifetime",
    default: || 0,
    codec: None,
    copy_on_death: false,
    sync: false,
};

#[test]
fn a_tick_clone_follows_its_join_but_cannot_replace_a_later_resident() {
    let mut boats = crate::join_seq::JoinList::<Boat, false>::default();
    for id in [1, 2] {
        boats.push(Boat::new(id, "oak".into(), Vec3::ZERO, 0.0));
    }
    let mut clone = boats[1].clone();
    clone.age = 7;
    boats.remove(0);
    assert!(boats.store_back(1, clone));
    assert_eq!(boats[0].id, 2);
    assert_eq!(boats[0].age, 7);
    let stale = boats.pop().unwrap().clone();
    boats.push(Boat::new(3, "oak".into(), Vec3::ZERO, 0.0));
    assert!(!boats.store_back(0, stale));
    assert_eq!(boats[0].id, 3);
    assert_eq!(boats[0].age, 0);
}

#[test]
#[ignore = "daily"]
fn a_bound_boat_keeps_its_registration_through_ticks_until_discard() {
    bound_boat_case(false, |game, step| {
        game.tick_boats();
        assert_eq!(game.boats[0].age, step);
        assert_eq!(game.boats[0].hurt_time, 6 - step);
        assert!(game.boats[0].position.y < 70.0);
    });
}

#[test]
#[ignore = "daily"]
fn a_bound_boat_commits_the_driven_client_tick() {
    bound_boat_case(true, |game, step| {
        game.riding = Some(71);
        assert!(
            game.remote_ride(
                horizon_client::sprint::Keys {
                    forward: true,
                    ..Default::default()
                },
                0
            )
            .unwrap()
        );
        assert_eq!(game.boats[0].hurt_time, 6 - step);
        assert!(game.boats[0].input[2]);
        assert!(game.boats[0].position.y < 70.0);
    });
}

#[test]
#[ignore = "daily"]
fn a_bound_ridden_boat_commits_the_server_leash_tick() {
    bound_boat_case(false, |game, step| {
        game.riding = Some(71);
        game.tick_boats();
        assert_eq!(game.boats[0].age, step);
        assert_eq!(game.boats[0].hurt_time, 6);
        assert_eq!(game.boats[0].position.y, 70.0);
    });
}

fn bound_boat_case(client: bool, tick: impl Fn(&mut Game<'_>, i32)) {
    let test = |game: &mut Game<'_>| {
        let _server = if client {
            let (client, server) = horizon_protocol::connection::Pipe::pair();
            game.connect_local(client, "BoatLifetimeTest").unwrap();
            game.remote.as_mut().unwrap().joined = true;
            game.remote.as_mut().unwrap().first_passenger = true;
            Some(server)
        } else {
            None
        };
        game.tickets.update(None);
        game.boats
            .push(Boat::new(71, "oak".into(), Vec3::new(0.5, 70.0, 0.5), 0.0));
        game.boats.update(0, |boat| boat.hurt_time = 6);
        let holder = if client {
            mod_api::EntityRef::new(game.native_lifecycle.epoch().unwrap(), 71)
        } else {
            let host: &dyn mod_api::ServerHost = game;
            host.entity(71).unwrap()
        };
        let index = Arc::new(Mutex::new(AttachmentIndex::default()));
        let mut values = Attachments::default();
        VALUE.set(&mut values, Holder::Entity(71), 10).unwrap();
        let original = game.boats.update(0, |boat| {
            boat.attachment_lifetime().bind(&mut values, &index, holder);
            boat.attachment_lifetime().registration().unwrap()
        });
        let key = ("test".into(), "boat_lifetime".into());
        let before = index.lock().unwrap().snapshot(&key);
        assert_eq!(before.len(), 1);
        for step in 1..=3 {
            tick(game, step);
            let current = game
                .boats
                .update(0, |boat| boat.attachment_lifetime().registration().unwrap());
            assert!(original.same_registration(&current));
            assert_eq!(index.lock().unwrap().snapshot(&key).len(), 1);
            assert_eq!(VALUE.get(&mut values, Holder::Entity(71)), Ok(10));
        }
        let taken = game.take_entities();
        assert_eq!(index.lock().unwrap().snapshot(&key).len(), 1);
        game.restore_entities(taken);
        VALUE.set(&mut values, Holder::Entity(71), 20).unwrap();
        let after = index.lock().unwrap().snapshot(&key);
        assert_eq!(after.len(), 1);
        assert_eq!(after[0].holder, holder);
        assert_eq!(after[0].entry.read(&VALUE), Ok(&20));
        game.persist
            .removals
            .push(super::super::commands::Target::Boat(71));
        let clone = game.boats[0].clone();
        assert!(!game.store_boat_back(0, clone));
        assert_eq!(index.lock().unwrap().snapshot(&key).len(), 1);
        game.native_removals_flush();
        assert!(game.boats.is_empty());
        assert!(index.lock().unwrap().snapshot(&key).is_empty());
        VALUE.set(&mut values, Holder::Entity(71), 30).unwrap();
        assert!(index.lock().unwrap().snapshot(&key).is_empty());
        assert_eq!(before[0].entry.read(&VALUE), Ok(&10));
        assert_eq!(after[0].entry.read(&VALUE), Ok(&20));
    };
    if client {
        super::super::native_entity_refs::tests::with_client_game(test);
    } else {
        super::super::native_entity_refs::tests::with_game(test);
    }
}
