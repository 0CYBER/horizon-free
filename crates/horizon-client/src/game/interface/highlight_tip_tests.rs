#![cfg(test)]
//! A stick's `ItemPart` answers `highlight_tip` (`IItemExtension.getHighlightTip`):
//! its text replaces the selected item's name for as long as the name shows.
use horizon_client::items::ItemStack;
use horizon_client::text::Text;
use mod_api::{ClientHost, HighlightTipEvent, Hooks, ItemPart, Registrar, RegistrationContext};
use std::sync::atomic::{AtomicU32, Ordering};

static ASKED: AtomicU32 = AtomicU32::new(0);

#[derive(Clone)]
struct Charge;
#[mod_api::horizon_part]
impl ItemPart for Charge {
    fn highlight_tip(&self, _: &dyn ClientHost, event: &mut HighlightTipEvent<'_>) {
        let asked = ASKED.fetch_add(1, Ordering::Relaxed);
        // A text that changes every tick: it must not keep the name up.
        *event.name = mod_api::Text::literal(format!("Charge: {}", 5 + asked % 2));
    }
}
struct Parts;
#[mod_api::__subscriptions]
impl Hooks for Parts {
    fn register(&self, r: &mut dyn Registrar, _: &RegistrationContext) {
        let key = r.item_key("minecraft:stick");
        r.item_part(key, Charge);
    }
}

#[test]
#[ignore = "daily"]
fn a_part_names_the_selected_stick_while_its_name_shows() {
    crate::game::in_own_process(
        "game::highlight_tip_tests::a_part_names_the_selected_stick_while_its_name_shows",
        || {
            let mods: mod_api::Mods =
                vec![mod_api::ModEntry::new("charge", Box::new(Parts))].into();
            mod_platform::testing::with_mods(mods.clone(), || {
                crate::game::api_values::install_parts(&mods).unwrap();
                crate::game::native_entity_refs::tests::with_game(|game| {
                    game.settings.notification_display_time = 1.0;
                    game.inventory[0] = Some(ItemStack::new("minecraft:stick", 1));
                    game.selected = 0;
                    game.tick_hud();
                    assert_eq!(game.tool_highlight.timer, 40);
                    assert_eq!(game.tool_highlight.tip, Some(Text::literal("Charge: 5")));
                    // Asked once a tick from 40 down to 1, then no more.
                    for _ in 0..45 {
                        game.tick_hud();
                    }
                    assert_eq!(ASKED.load(Ordering::Relaxed), 40);
                    assert_eq!(game.tool_highlight.tip, None);
                    // An apple has no part: vanilla's name, never asked.
                    game.inventory[0] = Some(ItemStack::new("minecraft:apple", 1));
                    game.tick_hud();
                    assert_eq!(game.tool_highlight.timer, 40);
                    assert_eq!(game.tool_highlight.tip, None);
                    assert_eq!(ASKED.load(Ordering::Relaxed), 40);
                });
            });
        },
    );
}
