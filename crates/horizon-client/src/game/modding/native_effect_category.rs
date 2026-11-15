//! Registered effect metadata reads.
use super::Game;

impl Game<'_> {
    pub(super) fn effect_category(
        &self,
        key: mod_api::EffectKey,
    ) -> Option<mod_api::EffectCategory> {
        let index = mod_api::__private::effect_index(key)?;
        let definition = horizon_client_core::potions::effect_by_index(index)?;
        Some(match definition.category {
            horizon_client_core::potions::Category::Beneficial => {
                mod_api::EffectCategory::Beneficial
            }
            horizon_client_core::potions::Category::Harmful => mod_api::EffectCategory::Harmful,
            horizon_client_core::potions::Category::Neutral => mod_api::EffectCategory::Neutral,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::super::native_entity_refs::tests::with_game;
    use mod_api::EffectCategory;

    #[test]
    #[ignore = "daily"]
    fn registered_effect_categories_use_type_definitions() {
        with_game(|game| {
            for (id, expected) in [
                ("minecraft:speed", EffectCategory::Beneficial),
                ("minecraft:poison", EffectCategory::Harmful),
                ("minecraft:glowing", EffectCategory::Neutral),
            ] {
                let key = mod_platform::__private::part_tests::effect_key(id);
                assert_eq!(key.category(game), Some(expected), "{id}");
                let active = mod_api::ActiveEffect::new(key, 200, 1, false, true, true);
                assert_eq!(active.id.category(game), Some(expected));
            }
        });
    }
}
