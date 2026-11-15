//! Validate ownership and freeze crafting-kind registrations.
use super::*;
impl RecipeSerializersRegistrar for PartRegistrar {
    fn register_recipe_type(&mut self, name: &str) {
        let id = match self.owned_declaration_id(name) {
            Ok(id) => id,
            Err(error) => {
                self.refused.push(error);
                return;
            }
        };
        if self.parts.recipe_types.contains_key(id.as_str()) {
            self.refused
                .push(format!("{} registered recipe type {id} twice", self.mod_id));
        } else {
            self.parts.recipe_types.insert(id.into(), self.mod_index);
        }
    }

    fn add_recipe_serializer(
        &mut self,
        name: &str,
        recipe_type: &str,
        handler: Option<Arc<dyn crate::CraftingRecipeHandler>>,
    ) {
        let id = match self.owned_declaration_id(name) {
            Ok(id) => id,
            Err(error) => {
                self.refused.push(error);
                return;
            }
        };
        let Some(handler) = handler else {
            self.refused.push(format!(
                "{} recipe serializer {id} requires a Wire value codec",
                self.mod_id
            ));
            return;
        };
        if self.parts.recipe_serializers.contains_key(id.as_str()) {
            self.refused.push(format!(
                "{} registered recipe serializer {id} twice",
                self.mod_id
            ));
            return;
        }
        self.parts
            .recipe_serializers
            .insert(id.into(), (self.mod_index, recipe_type.into(), handler));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    struct Literal {
        result: crate::ItemKey,
    }
    impl crate::CraftingRecipeKind for Literal {
        type Value = u32;
        fn codec(&self) -> Option<crate::CraftingRecipeCodec<u32>> {
            Some(crate::CraftingRecipeCodec::wire())
        }
        fn decode(&self, json: &str) -> Result<u32, String> {
            let value: serde_json::Value = serde_json::from_str(json).map_err(|e| e.to_string())?;
            value["count"]
                .as_u64()
                .filter(|n| *n == 3)
                .map(|n| n as u32)
                .ok_or("count must be 3".into())
        }
        fn write(&self, value: &u32) -> Result<String, String> {
            Ok(format!("{{\"count\":{value}}}"))
        }
        fn matches(&self, value: &u32, grid: crate::CraftingGrid) -> bool {
            *value == 3
                && grid.width == 3
                && grid.height == 3
                && grid
                    .cells
                    .iter()
                    .enumerate()
                    .all(|(i, c)| c.is_some() == [0, 4, 8].contains(&i))
        }
        fn assemble(&self, value: &u32, _: crate::CraftingGrid) -> Option<crate::ItemStack> {
            crate::ItemStack::new(self.result, *value)
        }
        fn remaining_items(
            &self,
            _: &u32,
            _: crate::CraftingGrid,
        ) -> Option<Vec<Option<crate::ItemStack>>> {
            let mut cells = vec![None; 9];
            cells[4] = crate::ItemStack::new(self.result, 1);
            Some(cells)
        }
    }
    struct Recipes;
    #[mod_api::__subscriptions]
    impl crate::Hooks for Recipes {
        fn register(&self, r: &mut dyn crate::Registrar, _: &crate::RegistrationContext) {
            let key = r.item_key("test:result");
            r.register_crafting_recipe_serializer("diagonal", Literal { result: key });
        }
    }
    #[test]
    fn registered_kind_decodes_matches_assembles_and_places_literal_remainder() {
        crate::parts::tests::install_parts_fixture(&[crate::ModEntry::new(
            "test",
            Box::new(Recipes),
        )])
        .unwrap();
        mod_vocab::__private::resolve_items(["test:result"]).unwrap();
        let handler =
            mod_vocab::__private::parts_installation::crafting_recipe("test:diagonal").unwrap();
        let value = handler.decode(r#"{"count":3}"#).unwrap();
        let out = handler
            .assemble(&value, crate::CraftingGrid::new(1, 1, vec![None]).unwrap())
            .unwrap();
        assert_eq!(out.count(), 3);
        assert_eq!(
            Some(out.item()),
            mod_vocab::__private::item_key("test:result")
        );
        let mut cells = vec![None; 9];
        for i in [0, 4, 8] {
            cells[i] = Some(out.clone());
        }
        let grid = crate::CraftingGrid::new(3, 3, cells).unwrap();
        assert!(handler.matches(&value, grid.clone()));
        assert_eq!(handler.write(&value).unwrap(), r#"{"count":3}"#);
        let mut expected = vec![None; 9];
        expected[4] = crate::ItemStack::new(out.item(), 1);
        assert_eq!(
            handler.remaining_items(&value, grid.clone()),
            Some(expected)
        );
        let mut wrong = grid;
        wrong.cells[4] = None;
        assert!(!handler.matches(&value, wrong));
        assert!(handler.decode("{").is_err());
        assert!(handler.decode(r#"{"count":2}"#).is_err());
    }
    struct NoWire;
    impl crate::CraftingRecipeKind for NoWire {
        type Value = std::sync::Mutex<()>;
        fn decode(&self, _: &str) -> Result<Self::Value, String> {
            Ok(std::sync::Mutex::new(()))
        }
        fn write(&self, _: &Self::Value) -> Result<String, String> {
            Ok("{}".into())
        }
        fn matches(&self, _: &Self::Value, _: crate::CraftingGrid) -> bool {
            false
        }
        fn assemble(&self, _: &Self::Value, _: crate::CraftingGrid) -> Option<crate::ItemStack> {
            None
        }
    }
    struct Invalid;
    #[mod_api::__subscriptions]
    impl crate::Hooks for Invalid {
        fn register(&self, r: &mut dyn crate::Registrar, _: &crate::RegistrationContext) {
            let key = r.item_key("test:result");
            r.register_crafting_recipe_serializer("diagonal", Literal { result: key });
            r.register_crafting_recipe_serializer("diagonal", Literal { result: key });
            r.register_crafting_recipe_serializer("missing_wire", NoWire);
        }
    }
    #[test]
    fn duplicate_and_non_wire_kinds_refuse_by_name() {
        let errors = crate::parts::tests::install_parts_fixture(&[crate::ModEntry::new(
            "test",
            Box::new(Invalid),
        )])
        .unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.contains("test:diagonal") && e.contains("twice")),
            "{errors:?}"
        );
        assert!(
            errors
                .iter()
                .any(|e| e.contains("test:missing_wire") && e.contains("Wire")),
            "{errors:?}"
        );
    }
}

impl PartRegistrar {
    pub(super) fn validate_recipe_types(&mut self) {
        for (serializer, (_, recipe_type, _)) in &self.parts.recipe_serializers {
            if recipe_type != "minecraft:crafting"
                && !self.parts.recipe_types.contains_key(recipe_type.as_str())
            {
                self.refused.push(format!(
                    "recipe serializer {serializer} names unregistered recipe type {recipe_type}"
                ));
            }
        }
    }
}

#[cfg(test)]
mod identity_tests {
    struct Duplicate;
    #[mod_api::__subscriptions]
    impl crate::Hooks for Duplicate {
        fn register(&self, r: &mut dyn crate::Registrar, _: &crate::RegistrationContext) {
            r.register_recipe_type("machine:grinding");
            r.register_recipe_type("machine:grinding");
            r.register_raw_recipe_serializer("machine:grinder_json", "machine:grinding");
            r.register_raw_recipe_serializer("machine:grinder_json", "machine:grinding");
        }
    }
    #[test]
    fn duplicate_serializer_and_type_ids_refuse_independently_by_name() {
        let errors = crate::__private::part_tests::install_parts_fixture(&[crate::ModEntry::new(
            "machine",
            Box::new(Duplicate),
        )])
        .unwrap_err();
        assert!(
            errors
                .iter()
                .any(|e| e.contains("recipe type machine:grinding twice")),
            "{errors:?}"
        );
        assert!(
            errors
                .iter()
                .any(|e| e.contains("machine:grinder_json twice")),
            "{errors:?}"
        );
    }
}
