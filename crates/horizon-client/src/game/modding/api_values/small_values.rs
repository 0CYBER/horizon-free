//! The recipe JSON text fields, which need a running game; the conversions live in horizon-gameplay.
#[cfg(test)]
mod recipe_tests {
    #[test]
    #[ignore = "daily"]
    fn recipe_json_text_fields() {
        crate::game::native_entity_refs::tests::with_game(|game| {
            assert_eq!(
                game.recipe_ingredient("not json").unwrap_err(),
                mod_api::RecipeJsonError::InvalidJson
            );
            assert_eq!(
                game.recipe_result("{").unwrap_err(),
                mod_api::RecipeJsonError::InvalidJson
            );
            assert_eq!(
                game.recipe_result(r#""iron_ingot""#)
                    .unwrap()
                    .unwrap()
                    .item
                    .as_ref(),
                "minecraft:iron_ingot"
            );
            let bare = game.recipe_ingredient(r#""iron_ingot""#).unwrap().unwrap();
            assert_eq!(bare.order, ["minecraft:iron_ingot"]);
            let list = game
                .recipe_ingredient(r#"["iron_ingot", "not_a_real_item", "gold_ingot"]"#)
                .unwrap()
                .unwrap();
            assert_eq!(list.order, ["minecraft:iron_ingot", "minecraft:gold_ingot"]);
            assert!(
                game.recipe_ingredient(r#""not_a_real_item""#)
                    .unwrap()
                    .is_none()
            );
            assert!(
                game.recipe_ingredient(r##""#minecraft:no_such_tag""##)
                    .unwrap()
                    .is_none()
            );
            let tag = game
                .recipe_ingredient(r##""#minecraft:planks""##)
                .unwrap()
                .unwrap();
            assert!(tag.order.iter().any(|id| id == "minecraft:oak_planks"));
            assert!(
                game.recipe_result(r#"{"id":"not_a_real_item"}"#)
                    .unwrap()
                    .is_none()
            );
            let stack = game.recipe_result(r#"{"id":"iron_ingot","count":2,"components":{"minecraft:custom_data":{"answer":42}}}"#).unwrap().unwrap();
            assert_eq!(stack.item.as_ref(), "minecraft:iron_ingot");
            assert_eq!(stack.count, 2);
            assert_eq!(
                stack
                    .data()
                    .unwrap()
                    .other("minecraft:custom_data")
                    .unwrap()
                    .get("answer"),
                Some(&horizon_core::nbt::Tag::Int(42))
            );
        });
    }
}
