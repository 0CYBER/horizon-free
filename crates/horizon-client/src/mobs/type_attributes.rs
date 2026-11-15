//! What a [`MobType`] takes from the attribute catalog: the spawn fields of
//! its attribute row and the bounds its mobs' own attribute record clamps to.
use super::{MobType, finalize::AttrBounds};
use horizon_world::attributes::entity::Catalog;
use std::{collections::HashMap, sync::Arc};

/// The fields of [`MobType`] its type's attribute row gives, built by
/// [`MobType::attribute_fields`]: the one map from an attribute to the
/// field a mob spawns with.
pub(crate) struct MobTypeAttributeFields {
    pub max_health: f32,
    pub speed: f64,
    pub flying_speed: f64,
    pub attack_damage: f32,
    pub attack_knockback: f64,
    pub follow_range: f64,
    pub step_height: f64,
    pub armor: f32,
    pub armor_toughness: f32,
    pub knockback_resistance: f64,
    pub jump_strength: f64,
    pub gravity: f64,
    pub safe_fall_distance: f64,
    pub fall_damage_multiplier: f64,
    pub water_efficiency: f64,
}

impl MobType {
    /// The spawn fields an entity type's attribute row carries (its
    /// `entity-types.json` row): the one map from an attribute to the field
    /// a mob of the type spawns with, over the loader's own fallback. The
    /// json's parser and the rest loader of a declared set (arismoko/horizon#845) both take
    /// it, so the fields cannot drift from the row they read.
    pub(crate) fn attribute_fields(value: impl Fn(&str) -> Option<f64>) -> MobTypeAttributeFields {
        let attr = |k: &str| value(&format!("minecraft:{k}"));
        MobTypeAttributeFields {
            max_health: attr("max_health").unwrap_or(10.0) as f32,
            speed: attr("movement_speed").unwrap_or(0.25),
            flying_speed: attr("flying_speed").unwrap_or(0.4),
            attack_damage: attr("attack_damage").unwrap_or(0.0) as f32,
            attack_knockback: attr("attack_knockback").unwrap_or(0.0),
            follow_range: attr("follow_range").unwrap_or(16.0),
            step_height: attr("step_height").unwrap_or(0.6),
            armor: attr("armor").unwrap_or(0.0) as f32,
            armor_toughness: attr("armor_toughness").unwrap_or(0.0) as f32,
            knockback_resistance: attr("knockback_resistance").unwrap_or(0.0),
            jump_strength: attr("jump_strength").unwrap_or(0.42),
            gravity: attr("gravity").unwrap_or(0.08),
            safe_fall_distance: attr("safe_fall_distance").unwrap_or(3.0),
            fall_damage_multiplier: attr("fall_damage_multiplier").unwrap_or(1.0),
            water_efficiency: attr("water_movement_efficiency").unwrap_or(0.0),
        }
    }

    /// The spawn fields of `row`, each value sanitized by `catalog`'s
    /// definition (`RangedAttribute.sanitizeValue`) before it narrows, so a
    /// mob spawns with what its attribute map answers. A value the catalog
    /// has no definition for stays as the row gives it.
    pub(crate) fn catalog_fields(
        row: &HashMap<String, f64>,
        catalog: &Catalog,
    ) -> MobTypeAttributeFields {
        Self::attribute_fields(|name| {
            let base = *row.get(name)?;
            Some(catalog.definition(name).map_or(base, |d| d.sanitize(base)))
        })
    }

    /// Takes the type's row from the live attribute map's catalog (arismoko/horizon#845):
    /// the merged row the map itself answers (the template's with the
    /// declared set on top), as its attribute map and its spawn fields, and
    /// the catalog's bounds, so the records cannot diverge. Bases stay raw
    /// for base-value reads; spawn fields use the catalog's computed-value
    /// sanitizer before narrowing.
    pub fn take_attribute_row(&mut self, attributes: Arc<HashMap<String, f64>>, catalog: &Catalog) {
        let fields = Self::catalog_fields(&attributes, catalog);
        self.attributes = attributes;
        self.attribute_bounds = AttrBounds::of(catalog);
        self.max_health = fields.max_health;
        self.speed = fields.speed;
        self.flying_speed = fields.flying_speed;
        self.attack_damage = fields.attack_damage;
        self.attack_knockback = fields.attack_knockback;
        self.follow_range = fields.follow_range;
        self.step_height = fields.step_height;
        self.armor = fields.armor;
        self.armor_toughness = fields.armor_toughness;
        self.knockback_resistance = fields.knockback_resistance;
        self.jump_strength = fields.jump_strength;
        self.gravity = fields.gravity;
        self.safe_fall_distance = fields.safe_fall_distance;
        self.fall_damage_multiplier = fields.fall_damage_multiplier;
        self.water_efficiency = fields.water_efficiency;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A vanilla-bounded catalog for `max_health` (here 1..10) and
    /// `armor_toughness` (0..20).
    fn catalog() -> Catalog {
        let bits = |v: f64| format!("{:016x}", v.to_bits());
        Catalog::from_json(&serde_json::json!({
            "data_version": horizon_world::DATA_VERSION,
            "attributes": [
                {"id": "minecraft:max_health", "default": bits(10.0), "min": bits(1.0), "max": bits(10.0), "syncable": true},
                {"id": "minecraft:armor_toughness", "default": bits(0.0), "min": bits(0.0), "max": bits(20.0), "syncable": true},
            ],
            "entities": {},
        }))
        .unwrap()
    }

    /// The loader's spawn fields go through the catalog it is given, as
    /// `take_attribute_row`'s do: a row past the bounds spawns its mobs
    /// inside them (20 health under a maximum of 10, 100 toughness under 20),
    /// and a NaN takes the minimum, so a mob with no attribute map yet reads
    /// what its map would answer.
    #[test]
    fn a_types_spawn_fields_take_the_catalogs_bounds() {
        let root = tempfile::tempdir().unwrap();
        std::fs::write(
            root.path().join("entity-types.json"),
            r#"{"minecraft:zombie":{"width":0.6,"height":1.95,"eye_height":1.74,"category":"monster","fire_immune":false,"attributes":{"minecraft:max_health":20.0,"minecraft:armor_toughness":100.0}}}"#,
        )
        .unwrap();
        let catalog = catalog();
        let types = super::super::MobTypes::load_for_server(root.path(), &catalog).unwrap();
        let zombie = &types.types["minecraft:zombie"];
        assert_eq!(zombie.max_health, 10.0);
        assert_eq!(zombie.armor_toughness, 20.0);
        let mob = super::super::Mob::new(
            1,
            Arc::new(zombie.clone()),
            horizon_core::geometry::Vec3::ZERO,
            0.0,
        );
        assert_eq!(super::super::armor::values(&mob).1, 20.0);
        let mut kind = zombie.clone();
        let row = HashMap::from([("minecraft:armor_toughness".to_owned(), f64::NAN)]);
        kind.take_attribute_row(Arc::new(row), &catalog);
        assert_eq!(kind.armor_toughness, 0.0);
    }
}
