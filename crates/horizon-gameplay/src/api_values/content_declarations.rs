//! Content declaration conversion. Admission retains the content loader's validation.
use super::TryIntoEngine;
#[path = "arm_pose.rs"]
mod arm_pose;
use horizon_mod_package::content as engine;

/// One vocabulary mapping for the content draft and the resolved simulation facts.
pub fn push_reaction(
    value: &mod_api::PushReaction,
) -> (engine::PushReaction, horizon_world::level::PushReaction) {
    use engine::PushReaction as Content;
    use horizon_world::level::PushReaction as World;
    match value {
        mod_api::PushReaction::PushPull => (Content::PushPull, World::PushPull),
        mod_api::PushReaction::Push => (Content::Push, World::Push),
        mod_api::PushReaction::Popped => (Content::Popped, World::Popped),
        mod_api::PushReaction::Immoveable => (Content::Immoveable, World::Immoveable),
        mod_api::PushReaction::IgnoreEntity => (Content::IgnoreEntity, World::IgnoreEntity),
    }
}

impl TryIntoEngine<()> for mod_api::BlockProperties {
    type Engine = engine::BlockProperties;
    type Error = String;
    fn try_into_engine(&self, _: &()) -> Result<Self::Engine, Self::Error> {
        Ok(engine::BlockProperties {
            hardness: self.hardness,
            explosion_resistance: self.explosion_resistance,
            light: self.light,
            dynamic_light: self.dynamic_light,
            requires_tool: self.requires_tool,
            friction: self.friction,
            map_color: self.map_color,
            sounds: self.sounds.clone(),
            random_ticks: self.random_ticks.as_ref().map(|v| match v {
                mod_api::RandomTicks::All(b) => engine::RandomTicks::All(*b),
                mod_api::RandomTicks::States { property, values } => engine::RandomTicks::States {
                    property: property.clone(),
                    values: values.clone(),
                },
            }),
            render_shape: self.render_shape.as_ref().map(|v| match v {
                mod_api::RenderShape::Model => engine::RenderShape::Model,
                mod_api::RenderShape::EntityblockAnimated => {
                    engine::RenderShape::EntityblockAnimated
                }
                mod_api::RenderShape::Invisible => engine::RenderShape::Invisible,
            }),
            push_reaction: self
                .push_reaction
                .as_ref()
                .map(|value| push_reaction(value).0),
            occludes: self.occludes,
            destroy_overlay: self.destroy_overlay,
        })
    }
}
impl TryIntoEngine<()> for mod_api::BlockShapes {
    type Engine = engine::BlockShapes;
    type Error = String;
    fn try_into_engine(&self, _: &()) -> Result<Self::Engine, Self::Error> {
        Ok(engine::BlockShapes(engine::StateValues {
            property: String::new(),
            properties: self.properties.clone(),
            values: self.values.clone(),
            default: self.default.clone(),
        }))
    }
}
impl TryIntoEngine<()> for mod_api::ShapeEntityKind {
    type Engine = engine::ShapeEntityKind;
    type Error = String;
    fn try_into_engine(&self, _: &()) -> Result<Self::Engine, Self::Error> {
        Ok(match self {
            Self::Item => engine::ShapeEntityKind::Item,
            Self::Player => engine::ShapeEntityKind::Player,
            Self::Living => engine::ShapeEntityKind::Living,
            Self::Other => engine::ShapeEntityKind::Other,
        })
    }
}
impl TryIntoEngine<()> for mod_api::Block {
    type Engine = engine::Block;
    type Error = String;
    fn try_into_engine(&self, _: &()) -> Result<Self::Engine, Self::Error> {
        Ok(engine::Block {
            state_light: self.state_light.as_ref().map(|light| engine::StateValues {
                property: String::new(),
                properties: light.properties.clone(),
                values: light.values.clone(),
                default: light.default,
            }),
            id: self.id.clone(),
            template: self.template.clone(),
            pick_item: self.pick_item.clone(),
            properties: self.properties.try_into_engine(&())?,
            shapes: self
                .shapes
                .as_ref()
                .map(|v| v.try_into_engine(&()))
                .transpose()?,
            collision_shapes: self
                .collision_shapes
                .as_ref()
                .map(|v| v.try_into_engine(&()))
                .transpose()?,
            outline_context_shapes: self
                .outline_context_shapes
                .iter()
                .map(|(kind, shape)| Ok((kind.try_into_engine(&())?, shape.try_into_engine(&())?)))
                .collect::<Result<_, String>>()?,
            collision_context_shapes: self
                .collision_context_shapes
                .iter()
                .map(|(kind, shape)| Ok((kind.try_into_engine(&())?, shape.try_into_engine(&())?)))
                .collect::<Result<_, String>>()?,
            interaction_shapes: self
                .interaction_shapes
                .as_ref()
                .map(|s| s.try_into_engine(&()))
                .transpose()?,
            state_properties: self
                .state_properties
                .iter()
                .map(|(k, v)| {
                    (
                        k.clone(),
                        match v {
                            mod_api::BlockStateProperty::Boolean => engine::StateProperty::Boolean,
                            mod_api::BlockStateProperty::Enum { values } => {
                                engine::StateProperty::Enum {
                                    values: values.clone(),
                                }
                            }
                        },
                    )
                })
                .collect(),
            default_state: self.default_state.clone(),
            item: self.item,
            open_sound: self.open_sound.clone(),
            close_sound: self.close_sound.clone(),
            block_item: self.block_item.clone(),
            components: component_patch("block", &self.id, &self.components)?,
            connects_to: self.connects_to.clone(),
            thickness: self.thickness,
            wood_type: self.wood_type.clone(),
            wall_block: self.wall_block.clone(),
            sign_text_color: self.sign_text_color.clone(),
            sign_text_glow: self.sign_text_glow,
            base_block: self.base_block.clone(),
        })
    }
}
impl TryIntoEngine<()> for mod_api::Item {
    type Engine = engine::Item;
    type Error = String;
    fn try_into_engine(&self, _: &()) -> Result<Self::Engine, Self::Error> {
        Ok(engine::Item {
            id: self.id.clone(),
            template: self.template.clone(),
            max_stack_size: self.max_stack_size,
            rarity: self.rarity.as_ref().map(|v| match v {
                mod_api::Rarity::Common => engine::Rarity::Common,
                mod_api::Rarity::Uncommon => engine::Rarity::Uncommon,
                mod_api::Rarity::Rare => engine::Rarity::Rare,
                mod_api::Rarity::Epic => engine::Rarity::Epic,
            }),
            components: component_patch("item", &self.id, &self.components)?,
            crafting_remainder: self.crafting_remainder.clone(),
            block: self.block.clone(),
            arm_pose: self
                .arm_pose
                .map(|pose| pose.try_into_engine(&()))
                .transpose()?,
        })
    }
}

fn component_patch(
    kind: &str,
    owner: &str,
    patch: &[(String, Option<mod_api::Tag>)],
) -> Result<std::collections::BTreeMap<String, serde_json::Value>, String> {
    let mut seen = std::collections::BTreeSet::new();
    let mut result = std::collections::BTreeMap::new();
    for (id, value) in patch {
        if id.starts_with('!') {
            return Err(format!(
                "{kind} {owner}: component id {id} starts with the removal prefix !"
            ));
        }
        // `lore` and `minecraft:lore` name the same component; the check only, the id is kept as written.
        let logical = if id.contains(':') {
            id.clone()
        } else {
            format!("minecraft:{id}")
        };
        if !seen.insert(logical) {
            return Err(format!("{kind} {owner}: duplicate component id {id}"));
        }
        let (key, value) = match value {
            Some(value) => (
                id.clone(),
                horizon_core::json_nbt::tag_to_json(value).map_err(|error| error.to_string())?,
            ),
            None => (format!("!{id}"), serde_json::json!({})),
        };
        result.insert(key, value);
    }
    Ok(result)
}

pub fn validate_decoded_effects(
    patch: Option<&mod_api::Tag>,
    stack: &horizon_items::items::ItemStack,
) -> Result<(), mod_api::StackWriteError> {
    // The component codec already decoded each supplied value. Check that admission
    // succeeded rather than parsing untrusted component payloads a second time.
    if let Some(patch) = patch
        && ((patch.get("minecraft:consumable").is_some() && stack.consumable(None).is_none())
            || (patch.get("minecraft:death_protection").is_some()
                && stack.death_protection(None).is_none()))
    {
        return Err(mod_api::StackWriteError::InvalidEffectPayload);
    }
    Ok(())
}

impl TryIntoEngine<()> for (String, mod_api::WorldFluid) {
    type Engine = engine::FluidLook;
    type Error = std::convert::Infallible;
    fn try_into_engine(&self, _: &()) -> Result<Self::Engine, Self::Error> {
        let (id, fluid) = self;
        Ok(engine::FluidLook {
            id: id.clone(),
            sprite: None,
            tint: None,
            world: Some(engine::WorldFluid {
                tick_delay: fluid.tick_delay,
                drop_off: fluid.drop_off,
                slope_find_distance: fluid.slope_find_distance,
                source_conversion: fluid.source_conversion,
                bucket: fluid.bucket,
            }),
        })
    }
}
