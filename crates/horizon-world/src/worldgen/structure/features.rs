//! The `template` and `fossil` configured features.
use super::{
    BoundingBox, Mirror, Rotation, StructureResources, StructureTemplate,
    processor::{PlaceSettings, ProcessorList, place_in_world},
};
use crate::{
    heightmap::HeightmapType,
    worldgen::{
        DecodeError,
        block_feature::FeatureBlockBindings,
        feature::FeatureWorld,
        placement::{PlacementContext, PlacementQueryError},
        weighted::WeightedList,
    },
};
use horizon_core::random::RandomSource;
use serde_json::{Value, value::RawValue};
use std::sync::Arc;

fn resources(bindings: &FeatureBlockBindings<'_>) -> Result<Arc<StructureResources>, DecodeError> {
    bindings
        .structures
        .clone()
        .ok_or(DecodeError::Field("structure template bindings"))
}
fn template(
    resources: &StructureResources,
    id: &str,
) -> Result<Arc<StructureTemplate>, DecodeError> {
    resources
        .template(id)
        .ok_or_else(|| DecodeError::Registry(format!("missing structure template {id}")))
}

#[derive(Clone, Debug)]
pub struct TemplateFeature {
    templates: WeightedList<(Arc<StructureTemplate>, Vec<Rotation>)>,
    processors: ProcessorList,
    resources: Arc<StructureResources>,
}
impl TemplateFeature {
    pub fn decode(json: &str, bindings: &FeatureBlockBindings<'_>) -> Result<Self, DecodeError> {
        let resources = resources(bindings)?;
        let value: Value = serde_json::from_str(json)?;
        let raw: Box<RawValue> = serde_json::from_str(&value["templates"].to_string())?;
        let templates = WeightedList::decode(&raw, |entry| {
            let entry: Value = serde_json::from_str(entry.get())?;
            let id = entry["id"]
                .as_str()
                .ok_or(DecodeError::Field("template id"))?;
            let rotations = match entry.get("rotations").and_then(Value::as_array) {
                Some(list) => list
                    .iter()
                    .map(|r| {
                        r.as_str()
                            .and_then(Rotation::from_name)
                            .ok_or(DecodeError::Field("rotation"))
                    })
                    .collect::<Result<Vec<_>, _>>()?,
                None => Rotation::ALL.to_vec(),
            };
            Ok((template(&resources, id)?, rotations))
        })?;
        let processors = match value.get("processors") {
            Some(p) => ProcessorList::decode(&p.to_string(), &resources, bindings)?,
            None => ProcessorList::default(),
        };
        Ok(Self {
            templates,
            processors,
            resources,
        })
    }
    pub fn place<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        random: &mut impl RandomSource,
        origin: [i32; 3],
    ) -> Result<bool, PlacementQueryError> {
        let invalid = |_| PlacementQueryError::Failed("template weights".into());
        let (template, rotations) = self.templates.sample(random).map_err(invalid)?;
        let rotation = rotations[random
            .next_int_bound(rotations.len() as i32)
            .map_err(invalid)? as usize];
        // rotation.rotate(axis.getNegative()) * (size along axis / 2), for X then Z.
        let mut pos = origin;
        for (axis, negative) in [
            (0, horizon_core::geometry::Direction::West),
            (2, horizon_core::geometry::Direction::North),
        ] {
            let step = rotation.rotate_direction(negative).normal();
            let length = template.size[axis] / 2;
            for i in 0..3 {
                pos[i] += step[i] * length;
            }
        }
        let mut settings = PlaceSettings::new(rotation).with_processors(&self.processors);
        settings.random = Some(random);
        place_in_world(
            world,
            &self.resources,
            template,
            pos,
            pos,
            &mut settings,
            None,
            3,
        )
    }
}

#[derive(Clone, Debug)]
pub struct FossilFeature {
    fossils: Vec<(Arc<StructureTemplate>, Arc<StructureTemplate>)>,
    fossil_processors: ProcessorList,
    overlay_processors: ProcessorList,
    max_empty_corners: i32,
    resources: Arc<StructureResources>,
}
impl FossilFeature {
    pub fn decode(json: &str, bindings: &FeatureBlockBindings<'_>) -> Result<Self, DecodeError> {
        let resources = resources(bindings)?;
        let value: Value = serde_json::from_str(json)?;
        let list = |key: &'static str| -> Result<Vec<Arc<StructureTemplate>>, DecodeError> {
            value[key]
                .as_array()
                .ok_or(DecodeError::Field(key))?
                .iter()
                .map(|id| template(&resources, id.as_str().unwrap_or_default()))
                .collect()
        };
        let (base, overlay) = (list("fossil_structures")?, list("overlay_structures")?);
        if base.is_empty() || base.len() != overlay.len() {
            return Err(DecodeError::Field("fossil structure lists"));
        }
        Ok(Self {
            fossils: base.into_iter().zip(overlay).collect(),
            fossil_processors: ProcessorList::decode(
                &value["fossil_processors"].to_string(),
                &resources,
                bindings,
            )?,
            overlay_processors: ProcessorList::decode(
                &value["overlay_processors"].to_string(),
                &resources,
                bindings,
            )?,
            max_empty_corners: value["max_empty_corners_allowed"].as_i64().unwrap_or(0) as i32,
            resources,
        })
    }
    pub fn place<W: FeatureWorld + PlacementContext>(
        &self,
        world: &mut W,
        random: &mut impl RandomSource,
        origin: [i32; 3],
    ) -> Result<bool, PlacementQueryError> {
        let invalid = |_| PlacementQueryError::Failed("fossil bounds".into());
        let rotation = Rotation::random(random);
        let index = random
            .next_int_bound(self.fossils.len() as i32)
            .map_err(invalid)? as usize;
        let (base, overlay) = &self.fossils[index];
        let (cx, cz) = (origin[0] >> 4, origin[2] >> 4);
        let bounds = BoundingBox::new(
            [cx * 16 - 16, world.min_y()?, cz * 16 - 16],
            [cx * 16 + 15 + 16, world.max_y()?, cz * 16 + 15 + 16],
        );
        let size = base.size(rotation);
        let low = [origin[0] - size[0] / 2, 0, origin[2] - size[2] / 2];
        let mut lowest = origin[1];
        for x in 0..size[0] {
            for z in 0..size[2] {
                lowest = lowest.min(world.height(
                    HeightmapType::OceanFloorWg,
                    low[0] + x,
                    low[2] + z,
                )?);
            }
        }
        let y =
            (lowest - 15 - random.next_int_bound(10).map_err(invalid)?).max(world.min_y()? + 10);
        let target = StructureTemplate::zero_position_with_transform(
            [low[0], y, low[2]],
            Mirror::None,
            rotation,
            base.size[0],
            base.size[2],
        );
        let mut empty = 0;
        for corner in base
            .bounding_box(target, rotation, [0; 3], Mirror::None)
            .corners()
        {
            let state = world.block_state(corner)?;
            let name = &self.resources.registry.state(state).unwrap().name;
            if matches!(
                name.as_str(),
                "minecraft:air"
                    | "minecraft:cave_air"
                    | "minecraft:void_air"
                    | "minecraft:lava"
                    | "minecraft:water"
            ) {
                empty += 1;
            }
        }
        if empty > self.max_empty_corners {
            return Ok(false);
        }
        for (template, processors) in [
            (base, &self.fossil_processors),
            (overlay, &self.overlay_processors),
        ] {
            let mut settings = PlaceSettings::new(rotation).with_processors(processors);
            settings.bounding_box = Some(bounds);
            settings.random = Some(&mut *random);
            place_in_world(
                world,
                &self.resources,
                template,
                target,
                target,
                &mut settings,
                None,
                260,
            )?;
        }
        Ok(true)
    }
}
