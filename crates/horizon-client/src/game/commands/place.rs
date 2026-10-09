//! `/place template` (PlaceCommand): a saved structure template placed with
//! rotation, mirroring and integrity.
use super::{Arg, Context, Game, R, err};
use horizon_client::{save, text::Text};
use horizon_core::random::{LegacyRandom, RandomSource};
use horizon_world::{
    compression::Compression,
    nbt::{self, Limits, Tag},
    worldgen::structure::{Mirror, Rotation, StateTransforms, StructureTemplate},
};
use std::sync::{Arc, OnceLock};

fn num(n: impl std::fmt::Display) -> Text {
    Text::literal(n.to_string())
}

impl Game<'_> {
    fn state_transforms(&self) -> Option<&'static StateTransforms> {
        static TRANSFORMS: OnceLock<Option<StateTransforms>> = OnceLock::new();
        TRANSFORMS
            .get_or_init(|| {
                let json = std::fs::read_to_string(
                    self.options
                        .resources
                        .join("worldgen/blocks/structure-blocks.json"),
                )
                .ok()?;
                StateTransforms::from_json(&json, self.res.assets.registry.len()).ok()
            })
            .as_ref()
    }
    /// A template by id: a structure block's save in the world, then the
    /// vanilla templates.
    fn load_template(&self, id: &str) -> Option<Arc<StructureTemplate>> {
        let (ns, path) = id.split_once(':').unwrap_or(("minecraft", id));
        let candidates = [
            self.world_root
                .join("generated")
                .join(ns)
                .join("structures")
                .join(format!("{path}.nbt")),
            self.options
                .resources
                .join("worldgen/structure")
                .join(ns)
                .join(format!("{path}.nbt")),
        ];
        let transforms = self.state_transforms()?;
        for file in candidates {
            let Ok(bytes) = std::fs::read(&file) else {
                continue;
            };
            let Ok(raw) = Compression::Gzip.decode(&bytes, 1 << 28) else {
                continue;
            };
            let Ok((named, _)) = nbt::read_named(
                &raw,
                Limits {
                    max_bytes: 1 << 28,
                    ..Limits::default()
                },
            ) else {
                continue;
            };
            return Some(Arc::new(StructureTemplate::from_nbt(
                &named.tag,
                self.res.assets.registry,
                transforms,
            )));
        }
        None
    }
    pub(super) fn cmd_place(&mut self, ctx: &Context) -> R<i32> {
        let Some(Arg::Resource(id)) = ctx.get("template").cloned() else {
            return Ok(0);
        };
        let pos = if ctx.get("pos").is_some() {
            self.block_pos_arg(ctx, "pos")?
        } else {
            let p = self.source_position();
            [
                p[0].floor() as i32,
                p[1].floor() as i32,
                p[2].floor() as i32,
            ]
        };
        let rotation = match ctx.get("rotation") {
            Some(Arg::Str(r)) => Rotation::from_name(r).unwrap_or(Rotation::None),
            _ => Rotation::None,
        };
        let mirror = match ctx.get("mirror") {
            Some(Arg::Str(m)) => match m.as_str() {
                "left_right" => Mirror::LeftRight,
                "front_back" => Mirror::FrontBack,
                _ => Mirror::None,
            },
            _ => Mirror::None,
        };
        let integrity = match ctx.get("integrity") {
            Some(Arg::Float(f)) => *f,
            _ => 1.0,
        };
        let seed = match ctx.get("seed") {
            Some(Arg::Int(s)) => i64::from(*s),
            _ => 0,
        };
        let strict = ctx.has("strict");
        let template = self
            .load_template(&id)
            .filter(|t| !t.palettes().is_empty())
            .ok_or_else(|| err("commands.place.template.invalid", &[&id]))?;
        let transforms = self
            .state_transforms()
            .ok_or_else(|| err("commands.place.template.invalid", &[&id]))?;
        let mut random = LegacyRandom::new(if seed == 0 {
            std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .map_or(1, |d| d.as_millis() as i64)
        } else {
            seed
        });
        let palette = template.palette_index(&mut random);
        let blocks = template.palettes()[palette].blocks().to_vec();
        let mut placed = 0;
        for b in &blocks {
            if integrity < 1.0 && random.next_float() > integrity {
                continue;
            }
            let t = StructureTemplate::transform(b.pos, mirror, rotation, [0, 0, 0]);
            let at = [pos[0] + t[0], pos[1] + t[1], pos[2] + t[2]];
            if !self.world.chunks.contains_key(&(at[0] >> 4, at[2] >> 4)) {
                continue;
            }
            let state = transforms.rotate(transforms.mirror(b.state, mirror), rotation);
            if self.place_state(at, state, strict).is_ok() {
                placed += 1;
            }
            if let Some(data) = &b.nbt {
                let mut tag = Tag::Compound(data.clone());
                if let Tag::Compound(c) = &mut tag {
                    c.insert("x".into(), Tag::Int(at[0]));
                    c.insert("y".into(), Tag::Int(at[1]));
                    c.insert("z".into(), Tag::Int(at[2]));
                }
                if let Some((_, entity)) = save::decode_block_entity(&tag) {
                    self.world
                        .block_entities
                        .insert((at[0], at[1], at[2]), entity);
                }
            }
        }
        // Entities saved with the template.
        for e in &template.entities {
            let p = StructureTemplate::transform_vec(e.pos, mirror, rotation, [0, 0, 0]);
            let at = [
                f64::from(pos[0]) + p[0],
                f64::from(pos[1]) + p[1],
                f64::from(pos[2]) + p[2],
            ];
            let mut tag = Tag::Compound(e.nbt.clone());
            if let Tag::Compound(c) = &mut tag {
                c.insert(
                    "Pos".into(),
                    Tag::List(at.iter().map(|v| Tag::Double(v.to_bits())).collect()),
                );
                c.remove(&"UUID".into());
            }
            self.spawn_entity_from_nbt(tag);
        }
        if placed == 0 {
            return Err(err("commands.place.template.failed", &[]));
        }
        self.feedback(Text::translate(
            "commands.place.template.success",
            &[Text::literal(id), num(pos[0]), num(pos[1]), num(pos[2])],
        ));
        Ok(1)
    }
}
