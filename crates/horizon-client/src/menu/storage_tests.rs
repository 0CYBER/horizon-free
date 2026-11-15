//! Version checks exercise the real menu's public mutation paths.
use super::tests::{bare_env, stack};
use super::*;

#[test]
fn cursor_and_slot_aba_invalidate_an_immutable_menu_read() {
    bare_env(|env| {
        let mut menu = Menu::new(Kind::Chest(3));
        menu.carried = stack("stone", 3);
        let read = menu.mutation_stamp();
        menu.carried = stack("dirt", 3);
        menu.carried = stack("stone", 3);
        assert_ne!(menu.mutation_stamp(), read);
        assert_eq!(menu.carried, stack("stone", 3));

        menu.set_slot(env, 0, stack("stone", 3));
        let read = menu.mutation_stamp();
        menu.set_slot(env, 0, stack("dirt", 3));
        menu.set_slot(env, 0, stack("stone", 3));
        assert_ne!(menu.mutation_stamp(), read);
        assert_eq!(menu.slot(0), stack("stone", 3).as_ref());
    });
}

#[test]
fn capacity_and_nested_public_state_edits_invalidate_reads() {
    let mut menu = Menu::new(Kind::Anvil);
    menu.set_container_limits(vec![Some(64); 3]);
    let read = menu.mutation_stamp();
    menu.set_container_limits(vec![Some(4); 3]);
    menu.set_container_limits(vec![Some(64); 3]);
    assert_ne!(menu.mutation_stamp(), read);

    let read = menu.mutation_stamp();
    menu.anvil.name = Some("temporary".into());
    menu.anvil.name = None;
    menu.data[0] = 5;
    menu.data[0] = 0;
    assert_ne!(menu.mutation_stamp(), read);
    assert_eq!(menu.anvil.name, None);
    assert_eq!(menu.data[0], 0);
}

#[test]
fn replacing_a_menu_with_a_snapshot_does_not_restore_its_version() {
    let mut menu = Menu::new(Kind::Chest(3));
    menu.carried = stack("stone", 3);
    let snapshot = menu.clone();
    let read = menu.mutation_stamp();
    menu.carried = stack("dirt", 2);
    menu = snapshot;
    assert_ne!(menu.mutation_stamp(), read);
    assert_eq!(menu.carried, stack("stone", 3));
}

#[test]
fn panicking_mutable_access_keeps_the_read_invalidated() {
    let mut menu = Menu::new(Kind::Chest(3));
    menu.carried = stack("stone", 3);
    let read = menu.mutation_stamp();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let cursor = menu.carried.as_mut().unwrap();
        cursor.count = 4;
        cursor.count = 3;
        panic!("listener after ABA");
    }));
    assert!(result.is_err());
    assert_ne!(menu.mutation_stamp(), read);
    assert_eq!(menu.carried, stack("stone", 3));
}

#[test]
fn immutable_queries_leave_a_menu_read_valid() {
    bare_env(|env| {
        let mut menu = Menu::new(Kind::Chest(3));
        menu.carried = stack("stone", 3);
        let read = menu.mutation_stamp();
        assert_eq!(menu.carried.as_ref().unwrap().count, 3);
        assert_eq!(menu.slot(0), None);
        assert_eq!(menu.slot_count(), 63);
        assert!(menu.may_place(env, 0, menu.carried.as_ref().unwrap()));
        assert!(!menu.quick_crafting());
        assert_eq!(menu.mutation_stamp(), read);
    });
}

#[test]
fn an_equal_slot_write_invalidates_but_plain_placement_still_works() {
    bare_env(|env| {
        let mut menu = Menu::new(Kind::Chest(3));
        menu.carried = stack("stone", 3);
        let read = menu.mutation_stamp();
        menu.set_slot(env, 0, None);
        assert_ne!(menu.mutation_stamp(), read);
        menu.click(env, 0, 0, Input::Pickup).unwrap();
        assert_eq!(menu.carried, None);
        assert_eq!(menu.slot(0), stack("stone", 3).as_ref());
    });
}
