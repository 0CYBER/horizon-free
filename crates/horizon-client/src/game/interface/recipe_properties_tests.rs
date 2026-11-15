//! Connection lifecycle of the authoritative recipe property generation.
#[cfg(test)]
mod tests {
    use crate::game::{execution_role::ExecutionRole, native_entity_refs::tests::with_game};
    use horizon_protocol::{connection::Pipe, packets::recipes::UpdateRecipes};

    #[test]
    #[ignore = "daily"]
    fn disconnect_discards_properties_before_reconnect() {
        with_game(|game| {
            game.options.execution_role = ExecutionRole::Client;
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "PropertiesA").unwrap();
            game.recipe_ui.server_properties.replace(UpdateRecipes {
                properties: vec![("first:input".into(), vec![7])],
                stonecutter: vec![],
            });
            // Closing a menu is not leaving the connection.
            game.recipe_ui.reset_screen();
            assert!(
                game.recipe_ui
                    .server_properties
                    .sets
                    .as_ref()
                    .unwrap()
                    .contains_key("first:input")
            );
            game.client_logging_out();
            assert!(game.recipe_ui.server_properties.sets.is_none());
            let (client, _server) = Pipe::pair();
            game.connect_local(client, "PropertiesB").unwrap();
            assert!(game.recipe_ui.server_properties.sets.is_none());
            game.recipe_ui
                .server_properties
                .replace(UpdateRecipes::default());
            assert!(
                game.recipe_ui
                    .server_properties
                    .sets
                    .as_ref()
                    .unwrap()
                    .is_empty()
            );
        });
    }
}
