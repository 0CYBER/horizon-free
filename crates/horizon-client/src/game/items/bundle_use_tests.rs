use super::*;
use crate::game::item_use::interact_vibration_tests::{EVENTS, with_game};
use std::sync::{
    Arc,
    atomic::{AtomicUsize, Ordering},
};

#[test]
#[ignore = "daily"]
fn bundle_completes_on_tick_200() {
    struct Listener(Arc<AtomicUsize>);
    #[mod_api::__subscriptions]
    impl mod_api::Hooks for Listener {
        fn living_entity_use_item_finish(
            &self,
            _: mod_api::HostSide<'_>,
            event: &mut mod_api::LivingEntityUseItemFinishEvent<'_>,
        ) {
            assert_eq!(
                mod_api::__private::item_key_id(event.result.as_ref().unwrap().item()),
                Some("minecraft:bundle")
            );
            self.0.fetch_add(1, Ordering::Relaxed);
        }
    }
    let calls = Arc::new(AtomicUsize::new(0));
    let mods: mod_api::Mods = vec![mod_api::ModEntry::new(
        "bundle-finish",
        Box::new(Listener(calls.clone())),
    )]
    .into();
    mod_platform::testing::with_mods(mods, || {
        with_game("minecraft:bundle", |game| {
            game.held_mut().as_mut().unwrap().data_mut().bundle_contents = vec![
                ItemStack::new("minecraft:stone", 1),
                ItemStack::new("minecraft:dirt", 1),
                ItemStack::new("minecraft:stick", 1),
            ];
            game.screen = Screen::Playing;
            game.right = true;
            game.lan_acting = Some(1);
            EVENTS.with(|events| events.borrow_mut().clear());
            assert!(game.start_using_item());
            for _ in 0..199 {
                game.tick_using();
            }
            assert_eq!(game.using, Some((game.selected, 199)));
            assert!(game.guest_entity_events.is_empty());
            assert_eq!(calls.load(Ordering::Relaxed), 0);
            EVENTS.with(|events| {
                assert!(
                    events
                        .borrow()
                        .iter()
                        .all(|e| e.0 != "minecraft:item_interact_finish")
                )
            });
            game.tick_using();
            assert_eq!(game.using, None);
            assert_eq!(game.guest_entity_events, vec![9]);
            assert_eq!(calls.load(Ordering::Relaxed), 1);
            EVENTS.with(|events| {
                assert_eq!(
                    events
                        .borrow()
                        .iter()
                        .filter(|e| e.0 == "minecraft:item_interact_finish")
                        .cloned()
                        .collect::<Vec<_>>(),
                    vec![(
                        "minecraft:item_interact_finish",
                        [10.5, 64.0, -3.25],
                        Some(horizon_client::mobs::Who::Player)
                    )]
                )
            });
            game.tick_using();
            assert_eq!(game.guest_entity_events, vec![9]);
            assert_eq!(calls.load(Ordering::Relaxed), 1);
        })
        .expect("prepared resources required");
    });
}
