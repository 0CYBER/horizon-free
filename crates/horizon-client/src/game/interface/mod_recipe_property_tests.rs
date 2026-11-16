//! A menu part reads the same authoritative generation as Host queries.
#[cfg(test)]
mod tests {
    use crate::game::native_entity_refs::tests::with_client_game;
    use horizon_client::items::ItemStack;
    use horizon_protocol::{connection::Pipe, packets::recipes::UpdateRecipes};
    use mod_api::{Hooks, MenuPart, MenuSlotRulesEvent, ModEntry, Mods, Registrar};
    struct Input;
    #[mod_api::horizon_part]
    impl MenuPart for Input {
        fn menu_slot_rules(&self, host: &dyn mod_api::Host, event: &mut MenuSlotRulesEvent<'_>) {
            if event.slot == 0 {
                event.may_place &=
                    host.mod_recipe_property_contains("pot:input", event.stack.item());
            }
        }
    }
    struct Pot;
    #[mod_api::__subscriptions]
    impl Hooks for Pot {
        fn register(&self, r: &mut dyn Registrar, _: &mod_api::RegistrationContext) {
            r.menu_part("pot:menu", Input);
        }
    }
    #[test]
    #[ignore = "daily"]
    fn host_and_slot_prediction_use_replacement_property_generation() {
        let mods: Mods = vec![ModEntry::new("pot", Box::new(Pot))].into();
        crate::game::api_values::install_parts(&mods).unwrap();
        crate::entry::install_client_hooks();
        mod_platform::testing::with_mods(mods, || {
            with_client_game(|game| {
                let (client, _server) = Pipe::pair();
                game.connect_local(client, "PropertyPrediction").unwrap();
                let item = mod_api::__private::item_key("minecraft:emerald").unwrap();
                let id = game
                    .remote
                    .as_ref()
                    .unwrap()
                    .builtin
                    .get("minecraft:item")
                    .unwrap()
                    .id("minecraft:emerald")
                    .unwrap();
                game.recipe_ui.server_properties.replace(UpdateRecipes {
                    properties: vec![("pot:input".into(), vec![id])],
                    stonecutter: vec![],
                });
                game.refresh_recipe_property_context();
                assert!(
                    mod_api::prelude::RecipePropertiesHost::mod_recipe_property_contains(
                        game,
                        "pot:input",
                        item
                    )
                );
                let stack = ItemStack::new("minecraft:emerald", 1);
                let accepts = |game: &crate::game::Game<'_>, stack: &ItemStack| {
                    horizon_client::menu::apply_menu_slot_rules(
                        Some(game as &dyn horizon_client::menu::SlotRules),
                        "pot:menu",
                        0,
                        stack,
                        horizon_client::menu::SlotContext::default(),
                        true,
                        64,
                        64,
                    )
                    .0
                };
                assert!(accepts(game, &stack));
                assert!(!accepts(game, &ItemStack::new("minecraft:diamond", 1)));
                game.recipe_ui
                    .server_properties
                    .replace(UpdateRecipes::default());
                game.refresh_recipe_property_context();
                assert!(!accepts(game, &stack));
                assert!(
                    !mod_api::prelude::RecipePropertiesHost::mod_recipe_property_contains(
                        game,
                        "pot:input",
                        item
                    )
                );
            })
        });
    }
}
