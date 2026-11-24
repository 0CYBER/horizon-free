//! `ClientPacketListener.handleUpdateAttributes` for entity attributes that
//! affect the client-side nameplate.
use super::{Game, snapshot_value};
use horizon_client::entities::nameplate::NameplateDistances;
use horizon_client::mobs::Species;
use horizon_protocol::packets::play;

/// `Strider.SUFFOCATING_MODIFIER_ID`: the cold strider's slowdown, which the
/// client applies itself from the synced `DATA_SUFFOCATING`
/// (`Mob::movement_speed`), as vanilla's client `Strider.tick` puts the same
/// modifier over the server's copy.
const STRIDER_SUFFOCATING: &str = "minecraft:suffocating";

/// The movement speed a rider's client moves a strider at: the server's
/// instance without the modifier the strider applies again on top of this base.
/// Counted twice, a cold strider went at 0.66 of its speed on a server (#19135).
fn mount_speed(snapshot: &play::AttributeSnapshot) -> f64 {
    if !snapshot
        .modifiers
        .iter()
        .any(|m| m.id == STRIDER_SUFFOCATING)
    {
        return snapshot_value(snapshot);
    }
    let mut own = snapshot.clone();
    own.modifiers.retain(|m| m.id != STRIDER_SUFFOCATING);
    snapshot_value(&own)
}

/// How a rider's client values the synced movement speed of a mount of this
/// species: a strider leaves out the cold modifier it applies itself
/// (`mount_speed`); any other mount keeps every synced modifier, as vanilla's
/// `handleUpdateAttributes` does, even one a command named
/// `minecraft:suffocating`.
fn mount_speed_for(species: Species) -> fn(&play::AttributeSnapshot) -> f64 {
    if species == Species::Strider {
        mount_speed
    } else {
        snapshot_value
    }
}

impl Game<'_> {
    /// Computes one synced attribute using the catalog's vanilla range.
    fn remote_attribute_value(
        &self,
        name: &str,
        values: &[play::AttributeSnapshot],
    ) -> Option<f64> {
        self.remote_attribute(name, values, snapshot_value)
    }

    /// One synced attribute, valued by `value`, in the catalog's vanilla range.
    fn remote_attribute(
        &self,
        name: &str,
        values: &[play::AttributeSnapshot],
        value: fn(&play::AttributeSnapshot) -> f64,
    ) -> Option<f64> {
        let attribute = self.motion.attributes.id(name)?;
        let snapshot = values
            .iter()
            .find(|snapshot| snapshot.attribute == attribute.registry_id())?;
        Some(
            self.motion
                .attributes
                .attribute(attribute)
                .definition
                .sanitize(value(snapshot)),
        )
    }

    /// The two attributes read by `LivingEntityRenderer` and
    /// `EntityRenderer.extractNameTags`.
    pub(super) fn remote_nameplate_values(
        &self,
        values: &[play::AttributeSnapshot],
    ) -> NameplateDistances {
        NameplateDistances {
            name_tag: self.remote_attribute_value("minecraft:name_tag_distance", values),
            below_name: self.remote_attribute_value("minecraft:below_name_distance", values),
        }
    }

    /// `ClientPacketListener.handleUpdateAttributes` for a mob. The same
    /// packet can target a player, mob or armor stand.
    pub(in crate::game) fn remote_mob_attributes(
        &mut self,
        id: i32,
        values: &[play::AttributeSnapshot],
    ) {
        let max_health = self.remote_attribute_value("minecraft:max_health", values);
        let scale = self.remote_attribute_value("minecraft:scale", values);
        let speed = self
            .mob_index
            .get(&self.mobs, id as u32)
            .map(|m| mount_speed_for(m.species()))
            .and_then(|value| self.remote_attribute("minecraft:movement_speed", values, value));
        let jump = self.remote_attribute_value("minecraft:jump_strength", values);
        let nameplate = self.remote_nameplate_values(values);
        let Some(m) = self.mob_index.get_mut(&mut self.mobs, id as u32) else {
            // An armor stand is living too: `minecraft:scale` sizes its box and model.
            self.armor_stand_attributes(id as u32, scale);
            if let Some(a) = self.armor_stands.iter_mut().find(|a| a.id == id as u32) {
                if let Some(value) = nameplate.name_tag {
                    a.nameplate.name_tag = Some(value);
                }
                if let Some(value) = nameplate.below_name {
                    a.nameplate.below_name = Some(value);
                }
            }
            return;
        };
        // A horse its rider's client moves goes at its own speed and jumps
        // its own height (`getSpeed`, `getCustomJump`).
        if horizon_client::mobs::equine::moved_by_rider(m.species()) {
            if let Some(speed) = speed {
                horizon_client::mobs::finalize::set_base(
                    m,
                    horizon_client::mobs::finalize::Attr::MovementSpeed,
                    speed,
                );
            }
            if let Some(jump) = jump {
                m.extra.vector[0] = jump;
            }
        }
        if let Some(max_health) = max_health {
            horizon_client::mobs::finalize::set_base(
                m,
                horizon_client::mobs::finalize::Attr::MaxHealth,
                max_health,
            );
        }
        if let Some(scale) = scale {
            m.extra.model_scale = scale as f32;
        }
        if let Some(value) = nameplate.name_tag {
            m.set_name_tag_distance(value);
        }
        if let Some(value) = nameplate.below_name {
            m.set_below_name_distance(value);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use horizon_client::mobs::{Mob, MobTypes, finalize};
    use std::{fs, sync::Arc};

    /// #19135: a vanilla server syncs a cold strider's movement speed with
    /// `Strider.SUFFOCATING_MODIFIER` (-0.34F, add_multiplied_base). Its rider's
    /// client moves it at `getAttributeValue(MOVEMENT_SPEED)`, which counts that
    /// modifier once.
    #[test]
    fn a_cold_striders_synced_speed_counts_its_slowdown_once() {
        let root =
            std::env::temp_dir().join(format!("horizon-strider-speed-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        fs::write(
            root.join("entity-types.json"),
            r#"{"minecraft:strider": {}}"#,
        )
        .unwrap();
        fs::write(
            root.join("entity-models.json"),
            r#"{"minecraft:strider#main": {}}"#,
        )
        .unwrap();
        fs::write(
            root.join("attributes.json"),
            format!(
                r#"{{"data_version": {}, "attributes": [], "entities": {{}}}}"#,
                horizon_core::version::DATA_VERSION
            ),
        )
        .unwrap();
        let mut types = MobTypes::load(&root).unwrap();
        fs::remove_dir_all(root).unwrap();
        let t = Arc::new(types.types.remove("minecraft:strider").unwrap());
        let mut m = Mob::new(1, t, horizon_core::geometry::Vec3::ZERO, 0.0);
        // Strider.createAttributes' 0.175F, and the synced DATA_SUFFOCATING.
        let (base, cold) = (f64::from(0.175_f32), f64::from(-0.34_f32));
        m.extra.flag = true;
        let synced = play::AttributeSnapshot {
            attribute: 0,
            base,
            modifiers: vec![play::AttributeModifier {
                id: "minecraft:suffocating".into(),
                amount: cold,
                operation: 1,
            }],
        };
        finalize::set_base(&mut m, finalize::Attr::MovementSpeed, mount_speed(&synced));
        // AttributeInstance.calculateValue: base + base * amount.
        assert_eq!(m.movement_speed(), base + base * cold);
    }

    /// A server can give any mob a modifier with the strider's id (`/attribute
    /// … modifier add minecraft:suffocating`), and vanilla's client keeps every
    /// synced modifier (`handleUpdateAttributes`). Only `Strider.tick` replaces
    /// or removes that id, so a pig or a horse counts the server's modifier once.
    #[test]
    fn only_a_strider_leaves_the_synced_suffocating_modifier_out() {
        // Pig.createAttributes' 0.25, halved by an add_multiplied_base modifier.
        let (base, amount) = (0.25, -0.5);
        let synced = play::AttributeSnapshot {
            attribute: 0,
            base,
            modifiers: vec![play::AttributeModifier {
                id: "minecraft:suffocating".into(),
                amount,
                operation: 1,
            }],
        };
        for species in [Species::Pig, Species::Horse] {
            assert_eq!(
                mount_speed_for(species)(&synced),
                base + base * amount,
                "{species:?}"
            );
        }
        assert_eq!(mount_speed_for(Species::Strider)(&synced), base);
    }
}
