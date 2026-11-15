//! Validation at the listener boundary, before the next mod sees its edits.
use super::RecipePropertyAddition;

pub(crate) fn validate_recipe_properties(
    owner: &str,
    previous: &[RecipePropertyAddition],
    additions: &mut Vec<RecipePropertyAddition>,
) {
    validate_in(owner, &crate::LoadedMods::new(), previous, additions);
}

fn validate_in(
    owner: &str,
    packages: &crate::LoadedMods,
    previous: &[RecipePropertyAddition],
    additions: &mut Vec<RecipePropertyAddition>,
) {
    let package = packages.get(owner);
    additions.retain(|addition| {
        let Some((namespace, _)) = addition.set.split_once(':') else {
            // Bare keys name vanilla sets; the engine checks that they exist.
            return true;
        };
        let existing = previous.iter().any(|old| old.set == addition.set);
        // Keep ownership aligned with PartRegistrar::owned_declaration_id (parts/owned_namespaces.rs, arismoko/horizon#2617).
        let owned = package.map_or(namespace == owner, |package| {
            package.namespaces().any(|owned| owned == namespace)
        });
        let valid = crate::Identifier::parse_namespaced(&addition.set).is_some()
            && (namespace == "minecraft" || existing || owned);
        if !valid {
            log::warn!(
                "recipe_property_sets: {owner} cannot create set {} outside its owned namespaces",
                addition.set
            );
        }
        valid
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PackageInfo, PackageKind, Side};
    use std::sync::Arc;

    #[test]
    fn owned_manifest_namespace_can_create_but_foreign_new_key_cannot() {
        let packages = crate::LoadedMods {
            entries: Arc::from([PackageInfo::new(
                "ported_farmersdelight".into(),
                "1".into(),
                PackageKind::Code,
                Side::Both,
                vec!["ported_farmersdelight".into(), "farmersdelight".into()],
                vec![],
            )]),
        };
        let previous = vec![RecipePropertyAddition::new("another:known", vec![])];
        let mut additions: Vec<_> = [
            "farmersdelight:cooking_pot_input",
            "ported_farmersdelight:input",
            "another:known",
            "another:new",
            "BAD:input",
            "minecraft:furnace_input",
        ]
        .into_iter()
        .map(|key| RecipePropertyAddition::new(key, vec![]))
        .collect();
        validate_in(
            "ported_farmersdelight",
            &packages,
            &previous,
            &mut additions,
        );
        assert_eq!(
            additions.iter().map(|a| a.set.as_str()).collect::<Vec<_>>(),
            [
                "farmersdelight:cooking_pot_input",
                "ported_farmersdelight:input",
                "another:known",
                "minecraft:furnace_input"
            ]
        );
    }
}
