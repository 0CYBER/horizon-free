//! Content declaration conversion. Admission retains the content loader's validation.
use super::TryIntoEngine;
#[path = "arm_pose.rs"]
mod arm_pose;
use horizon_mod_package::content as engine;

/// One vocabulary mapping for the content draft and the resolved simulation facts.
pub(in crate::game) fn push_reaction(
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

#[cfg(test)]
mod tests {
    use super::*;
    use mod_api::{Block, BlockProperties, Item, Tag};
    use std::collections::BTreeMap;

    #[test]
    fn state_light_matches_json_and_preserves_emission() {
        let mut draft = Block::new("test:lamp", "minecraft:stone");
        draft
            .state_properties
            .insert("lit".into(), mod_api::BlockStateProperty::Boolean);
        draft.properties.light = Some(3);
        draft.state_light = Some(mod_api::StateLight {
            properties: vec!["lit".into()],
            values: BTreeMap::from([("true".into(), 15)]),
            default: Some(0),
        });
        let converted = draft.try_into_engine(&()).unwrap();
        let json: engine::StateLight =
            serde_json::from_str(r#"{"properties":["lit"],"values":{"true":15},"default":0}"#)
                .unwrap();
        assert_eq!(converted.state_light.as_ref(), Some(&json));
        let table = converted.state_light.unwrap();
        assert_eq!(
            table.values.get("false").copied().or(table.default),
            Some(0)
        );
        assert_eq!(
            table.values.get("true").copied().or(table.default),
            Some(15)
        );
    }

    /// A block that leaves `item` out has its block item, declared in a file
    /// or in code (`Block::new`, `Block::default`) alike.
    #[test]
    fn a_block_has_its_item_unless_told_not_to_in_files_and_in_code() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("test.hmod");
        let manifest = "id = \"test\"\nname = \"Test\"\nversion = \"0.1.0\"\napi = \"0.2\"\nside = \"both\"\ncapabilities = [\"registries\"]\n";
        let block_file: &[u8] = br#"{"template": "minecraft:stone"}"#;
        let files = [("data/test/horizon/block/file.json", block_file)];
        std::fs::write(
            &path,
            horizon_mod_package::Package::write_files(manifest, &files).unwrap(),
        )
        .unwrap();
        let package = horizon_mod_package::Package::read(&path).unwrap();
        let from_file = horizon_mod_package::Content::read(&path, &package.manifest).unwrap();
        let mut default = Block::default();
        default.id = "test:default".into();
        default.template = "minecraft:stone".into();
        let declared = |block: Block| {
            let mut content = horizon_mod_package::Content::declared("test");
            content
                .declare(&[block.try_into_engine(&()).unwrap()], &[])
                .unwrap();
            content
        };
        let item = |id: &str| engine::Item {
            id: id.into(),
            template: "minecraft:stick".into(),
            ..Default::default()
        };
        for (mut content, id) in [
            (from_file, "test:file"),
            (
                declared(Block::new("test:new", "minecraft:stone")),
                "test:new",
            ),
            (declared(default), "test:default"),
        ] {
            // The block item holds the id: an item of the same id is refused.
            assert_eq!(
                content.declare(&[], &[item(id)]),
                Err(format!(
                    "item {id} is declared twice, or is also a block's item"
                )),
            );
        }
        let mut without = declared(Block::new("test:bare", "minecraft:stone").with_item(false));
        assert_eq!(without.declare(&[], &[item("test:bare")]), Ok(()));
    }

    #[test]
    fn duplicate_component_ids_refuse_blocks_and_items() {
        let set = || Some(Tag::List(vec![]));
        // Set then set, set then removal, removal then set, removal then removal.
        for (first, second) in [(set(), set()), (set(), None), (None, set()), (None, None)] {
            let patch = vec![
                ("minecraft:lore".into(), first),
                ("minecraft:lore".into(), second),
            ];
            let mut block = Block::new("test:block", "minecraft:stone");
            block.components = patch.clone();
            let mut item = Item::new("test:item", "minecraft:stick");
            item.components = patch;
            assert_eq!(
                block.try_into_engine(&()).unwrap_err(),
                "block test:block: duplicate component id minecraft:lore"
            );
            assert_eq!(
                item.try_into_engine(&()).unwrap_err(),
                "item test:item: duplicate component id minecraft:lore"
            );
        }
    }

    #[test]
    fn distinct_component_ids_convert_for_blocks_and_items() {
        let patch = vec![
            ("minecraft:max_stack_size".into(), Some(Tag::Int(16))),
            ("minecraft:damage".into(), None),
        ];
        let expected = BTreeMap::from([
            ("minecraft:max_stack_size".into(), serde_json::json!(16)),
            ("!minecraft:damage".into(), serde_json::json!({})),
        ]);
        let mut block = Block::new("test:block", "minecraft:stone");
        block.components = patch.clone();
        assert_eq!(block.try_into_engine(&()).unwrap().components, expected);
        let mut item = Item::new("test:item", "minecraft:stick");
        item.components = patch;
        assert_eq!(item.try_into_engine(&()).unwrap().components, expected);
    }

    #[test]
    fn bare_and_prefixed_component_ids_refuse_blocks_and_items() {
        let mut block = Block::new("test:block", "minecraft:stone");
        block.components = vec![
            ("lore".into(), Some(Tag::List(vec![]))),
            ("minecraft:lore".into(), None),
        ];
        assert_eq!(
            block.try_into_engine(&()).unwrap_err(),
            "block test:block: duplicate component id minecraft:lore"
        );
        let mut item = Item::new("test:item", "minecraft:stick");
        item.components = vec![
            (
                "!minecraft:lore".into(),
                Some(Tag::Compound(BTreeMap::new())),
            ),
            ("minecraft:lore".into(), None),
        ];
        assert_eq!(
            item.try_into_engine(&()).unwrap_err(),
            "item test:item: component id !minecraft:lore starts with the removal prefix !"
        );
    }

    #[test]
    fn context_shape_declarations_preserve_categories_and_boxes() {
        let shape = mod_api::BlockShapes {
            properties: vec![],
            values: BTreeMap::new(),
            default: Some(vec![[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]]),
        };
        let mut draft = Block::new("test:shape", "minecraft:stone");
        for kind in [
            mod_api::ShapeEntityKind::Item,
            mod_api::ShapeEntityKind::Player,
            mod_api::ShapeEntityKind::Living,
            mod_api::ShapeEntityKind::Other,
        ] {
            draft.outline_context_shapes.insert(kind, shape.clone());
            draft.collision_context_shapes.insert(kind, shape.clone());
        }
        draft.interaction_shapes = Some(shape);
        let result = draft.try_into_engine(&()).unwrap();
        let boxes = Some(vec![[1.0, 2.0, 3.0, 4.0, 5.0, 6.0]]);
        for kind in [
            engine::ShapeEntityKind::Item,
            engine::ShapeEntityKind::Player,
            engine::ShapeEntityKind::Living,
            engine::ShapeEntityKind::Other,
        ] {
            assert_eq!(result.outline_context_shapes[&kind].default, boxes);
            assert_eq!(result.collision_context_shapes[&kind].default, boxes);
        }
        assert_eq!(result.interaction_shapes.unwrap().default, boxes);
    }

    #[test]
    fn declaration_conversion_preserves_admission_outcomes() {
        // Independent old-path records: errors must match byte for byte, not just reject.
        for (id, template, light, hardness, item) in [
            ("test:block", "minecraft:stone", 15, 2.5, true),
            ("other:block", "minecraft:stone", 0, 1.0, false),
            ("test:Bad", "minecraft:stone", 0, 1.0, false),
            ("test:block", "other:stone", 0, 1.0, false),
            ("test:block", "minecraft:stone", 16, 1.0, false),
            ("test:block", "minecraft:stone", 0, f32::INFINITY, false),
        ] {
            let api = {
                let mut draft = Block::new(id, template);
                draft.item = item;
                draft.properties = BlockProperties {
                    light: Some(light),
                    hardness: Some(hardness),
                    ..Default::default()
                };
                draft
            };
            let old = engine::Block {
                id: id.into(),
                template: template.into(),
                item,
                properties: engine::BlockProperties {
                    light: Some(light),
                    hardness: Some(hardness),
                    ..Default::default()
                },
                ..Default::default()
            };
            let converted = api.try_into_engine(&()).unwrap();
            assert_eq!(converted, old);
            let mut expected = horizon_mod_package::Content::declared("test");
            let mut actual = horizon_mod_package::Content::declared("test");
            assert_eq!(
                actual.declare(&[converted], &[]),
                expected.declare(&[old], &[])
            );
            assert_eq!(actual.blocks, expected.blocks);
            if light == 16 {
                let valid = engine::Block {
                    id: "test:first".into(),
                    template: "minecraft:stone".into(),
                    ..Default::default()
                };
                let bad = api.try_into_engine(&()).unwrap();
                let mut admission = horizon_mod_package::Content::declared("test");
                let error = admission.declare(&[valid, bad], &[]).unwrap_err();
                assert_eq!(error, "block test:block: light is 0 to 15");
                assert!(admission.blocks.is_empty());
                assert!(admission.items.is_empty());
            }
        }
        for (id, template, max, remainder) in [
            ("test:item", "minecraft:stick", 64, None),
            ("other:item", "minecraft:stick", 64, None),
            ("test:item", "other:stick", 64, None),
            ("test:item", "minecraft:stick", 0, None),
            ("test:item", "minecraft:stick", 100, None),
            ("test:item", "minecraft:stick", 64, Some("test:missing")),
            ("test:item", "minecraft:stick", 64, Some("minecraft:bucket")),
        ] {
            let api = {
                let mut draft = Item::new(id, template);
                draft.max_stack_size = Some(max);
                draft.crafting_remainder = remainder.map(str::to_owned);
                draft
            };
            let old = engine::Item {
                id: id.into(),
                template: template.into(),
                max_stack_size: Some(max),
                crafting_remainder: remainder.map(str::to_owned),
                ..Default::default()
            };
            let converted = api.try_into_engine(&()).unwrap();
            assert_eq!(converted, old);
            let mut expected = horizon_mod_package::Content::declared("test");
            let mut actual = horizon_mod_package::Content::declared("test");
            assert_eq!(
                actual.declare(&[], &[converted]),
                expected.declare(&[], &[old])
            );
            assert_eq!(actual.items, expected.items);
        }
    }

    #[test]
    fn declaration_component_patch_preserves_values_and_removals() {
        let api = {
            let mut draft = Item::new("test:item", "minecraft:stick");
            draft.components = vec![
                ("minecraft:max_stack_size".into(), Some(Tag::Int(16))),
                (
                    "minecraft:custom_data".into(),
                    Some(Tag::Compound(
                        [(horizon_core::nbt::JavaString::from("energy"), Tag::Int(42))].into(),
                    )),
                ),
                ("minecraft:damage".into(), None),
            ];
            draft
        };
        let converted = api.try_into_engine(&()).unwrap();
        assert_eq!(
            converted.components,
            BTreeMap::from([
                ("minecraft:max_stack_size".into(), serde_json::json!(16)),
                (
                    "minecraft:custom_data".into(),
                    serde_json::json!({"energy": 42})
                ),
                ("!minecraft:damage".into(), serde_json::json!({})),
            ])
        );
        let mut invalid = api;
        invalid.components = vec![(
            "minecraft:custom_data".into(),
            Some(Tag::Double(f64::NAN.to_bits())),
        )];
        assert!(invalid.try_into_engine(&()).is_err());
    }

    #[test]
    fn declaration_nested_block_overrides_are_not_lost() {
        let api = {
            let mut draft = Block::new("test:block", "minecraft:stone");
            draft.properties = BlockProperties {
                friction: Some(0.8),
                explosion_resistance: Some(7.0),
                requires_tool: Some(true),
                map_color: Some(9),
                sounds: Some("minecraft:glass".into()),
                random_ticks: Some(mod_api::RandomTicks::States {
                    property: "powered".into(),
                    values: vec!["true".into()],
                }),
                render_shape: Some(mod_api::RenderShape::Invisible),
                push_reaction: Some(mod_api::PushReaction::Popped),
                occludes: Some(false),
                destroy_overlay: Some(false),
                dynamic_light: Some(true),
                ..Default::default()
            };
            draft.state_properties =
                BTreeMap::from([("powered".into(), mod_api::BlockStateProperty::Boolean)]);
            draft.default_state = BTreeMap::from([("powered".into(), "false".into())]);
            draft.shapes = Some(mod_api::BlockShapes {
                properties: vec![],
                values: BTreeMap::new(),
                default: Some(vec![[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]]),
            });
            draft
        };
        let converted = api.try_into_engine(&()).unwrap();
        assert_eq!(
            converted.properties,
            engine::BlockProperties {
                friction: Some(0.8),
                explosion_resistance: Some(7.0),
                requires_tool: Some(true),
                map_color: Some(9),
                sounds: Some("minecraft:glass".into()),
                random_ticks: Some(engine::RandomTicks::States {
                    property: "powered".into(),
                    values: vec!["true".into()]
                }),
                render_shape: Some(engine::RenderShape::Invisible),
                push_reaction: Some(engine::PushReaction::Popped),
                occludes: Some(false),
                destroy_overlay: Some(false),
                dynamic_light: Some(true),
                ..Default::default()
            }
        );
        assert_eq!(
            converted.state_properties,
            BTreeMap::from([("powered".into(), engine::StateProperty::Boolean)])
        );
        assert_eq!(
            converted.default_state,
            BTreeMap::from([("powered".into(), "false".into())])
        );
        assert_eq!(
            converted.shapes.unwrap().default,
            Some(vec![[0.0, 0.0, 0.0, 1.0, 0.5, 1.0]])
        );
    }
}

pub(super) fn validate_decoded_effects(
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

#[cfg(test)]
mod effect_write_tests {
    use super::super::{TryIntoEngine, stack_limit_tests::catalog};
    use horizon_core::nbt::JavaString;
    use mod_api::{ItemStack, StackWriteError, Tag};

    #[test]
    fn malformed_effect_override_refuses_typed_stack_write() {
        let items = catalog();
        let key = mod_api::__private::item_key("minecraft:stick").unwrap();
        for (component, effects) in [
            ("minecraft:consumable", "on_consume_effects"),
            ("minecraft:death_protection", "death_effects"),
        ] {
            let invalid = Tag::compound([(
                effects,
                Tag::List(vec![Tag::compound([
                    ("type", Tag::String("example:charge".into())),
                    ("bad", Tag::Double(f64::NAN.to_bits())),
                ])]),
            )]);
            let mut draft = ItemStack::of(key);
            draft
                .set_components([(JavaString::from(component), invalid)].into())
                .unwrap();
            assert_eq!(
                draft.try_into_engine(&items),
                Err(StackWriteError::InvalidEffectPayload)
            );
        }
    }
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
