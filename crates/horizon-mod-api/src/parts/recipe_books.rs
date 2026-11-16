//! Concrete authoring extracted verbatim from the vocabulary layer.
use super::*;

impl RecipeBooksRegistrar for PartRegistrar {
    fn recipe_book_type(&mut self, id: &str) -> Result<RecipeBookTypeKey, String> {
        let result = (|| {
            if mod_vocab::__private::parts_installation::parts_installed() {
                return Err("registry is frozen".to_owned());
            }
            let (namespace, path) = id.split_once(':').ok_or("explicit namespace required")?;
            if id.len() > 256 {
                return Err("id exceeds 256 bytes".into());
            }
            if namespace.is_empty()
                || path.is_empty()
                || !namespace
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-".contains(&b))
                || !path
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"_.-/".contains(&b))
            {
                return Err("nonempty lowercase resource-id namespace and path required".into());
            }
            if let Some(owner) = self.recipe_book_types.get(id) {
                return Err(format!(
                    "duplicate key declared by {owner} and {}",
                    self.mod_id
                ));
            }
            if namespace == "minecraft" || !owns_namespace(self.mod_id, namespace) {
                return Err("namespace must be owned by the registering mod, not minecraft".into());
            }
            if self.recipe_book_types.len() >= 1024 {
                return Err("registry exceeds 1024 custom books".into());
            }
            self.recipe_book_types.insert(id.into(), self.mod_id.into());
            Ok(mod_vocab::__private::parts_installation::recipe_book_type(
                id.into(),
            ))
        })();
        result.map_err(|reason: String| {
            let message = format!("mod {} recipe book {id:?}: {reason}", self.mod_id);
            self.refused.push(message.clone());
            message
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn registration_rejects_invalid_and_duplicate_declarations() {
        let mut registrar = PartRegistrar::new("demo");
        let key = registrar.recipe_book_type("demo:cooking_pot").unwrap();
        assert_eq!(key.as_str(), "demo:cooking_pot");
        assert!(
            registrar
                .recipe_book_type("demo:cooking_pot")
                .unwrap_err()
                .contains("duplicate")
        );
        for id in [
            "pot",
            "demo:",
            ":pot",
            "demo:Pot",
            "minecraft:pot",
            "other:pot",
            "demo:a:b",
        ] {
            assert!(registrar.recipe_book_type(id).is_err(), "{id}");
        }
        assert!(
            registrar
                .recipe_book_type(&format!("demo:{}", "a".repeat(252)))
                .is_err()
        );
        assert_eq!(registrar.recipe_book_types.len(), 1);
        assert_eq!(registrar.refused.len(), 9);
    }

    #[test]
    fn cross_mod_duplicate_names_both_declarers() {
        let mut registrar = PartRegistrar::new("first");
        registrar.recipe_book_type("first:book").unwrap();
        registrar.mod_id = "second";
        let error = registrar.recipe_book_type("first:book").unwrap_err();
        assert!(error.contains("duplicate"));
        assert!(error.contains("first:book"));
        assert!(error.contains("first and second"));
        assert_eq!(registrar.recipe_book_types.len(), 1);
    }

    #[test]
    fn registry_capacity_does_not_evict_existing_books() {
        let mut registrar = PartRegistrar::new("demo");
        for index in 0..1024 {
            registrar
                .recipe_book_type(&format!("demo:book_{index}"))
                .unwrap();
        }
        assert!(
            registrar
                .recipe_book_type("demo:overflow")
                .unwrap_err()
                .contains("1024")
        );
        assert_eq!(registrar.recipe_book_types.len(), 1024);
    }
}
