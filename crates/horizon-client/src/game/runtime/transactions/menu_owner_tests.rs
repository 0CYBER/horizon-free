use super::*;
use horizon_client::{
    items::ItemStack,
    menu::{Kind, Menu},
};

#[test]
fn taking_and_returning_the_same_menu_invalidates_the_owner_read() {
    let mut owner = MenuOwner::<Option<Box<Menu>>>::default();
    *owner = Some(Box::new(Menu::new(Kind::Inventory)));
    let id = owner.id();
    let stamp = owner.stamp();
    let menu_stamp = owner.as_ref().unwrap().mutation_stamp();
    let menu = owner.take().unwrap();
    *owner = Some(menu);
    assert_eq!(owner.id(), id);
    assert_ne!(owner.stamp(), stamp);
    assert_eq!(owner.as_ref().unwrap().mutation_stamp(), menu_stamp);
}

#[test]
fn undo_restores_cursor_without_restoring_the_owner_revision() {
    let mut owner = MenuOwner::<Option<Box<Menu>>>::default();
    let mut menu = Box::new(Menu::new(Kind::Inventory));
    menu.carried = Some(ItemStack::new("minecraft:stone", 3));
    *owner = Some(menu);
    let undo = (*owner).clone();
    let stamp = owner.stamp();
    owner.as_mut().unwrap().carried = Some(ItemStack::new("minecraft:dirt", 2));
    let edited = owner.stamp();
    owner.restore(undo);
    assert_ne!(owner.stamp(), stamp);
    assert_ne!(owner.stamp(), edited);
    assert_eq!(
        owner.as_ref().unwrap().carried,
        Some(ItemStack::new("minecraft:stone", 3))
    );
}

#[test]
fn panic_during_an_owner_replacement_does_not_revalidate_an_old_read() {
    let mut owner = MenuOwner::<Option<Box<Menu>>>::default();
    *owner = Some(Box::new(Menu::new(Kind::Inventory)));
    let stamp = owner.stamp();
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let menu = owner.take().unwrap();
        *owner = Some(menu);
        panic!("listener after replacement ABA");
    }));
    assert!(result.is_err());
    assert!(owner.is_some());
    assert_ne!(owner.stamp(), stamp);
}
