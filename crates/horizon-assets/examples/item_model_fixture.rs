//! Resolve item stacks into render layers for the differential harness
//! `tools/test_item_model_parity.py`.
use horizon_assets::{
    animation::AnimationMetadata,
    item_bake::{BakedItemModel, SpriteLookup, SpriteMask, bake_item_model},
    item_model::*,
    math::Mat4,
    model::{CuboidModel, Direction},
    palette::{PalettedPermutations, apply_palette},
    resolve::{MISSING_MODEL, ModelLibrary},
};
use serde_json::{Value, json};
use std::{
    cell::RefCell,
    collections::BTreeMap,
    fs::{self, File},
    io::{BufRead, BufReader, BufWriter, Write},
    path::{Path, PathBuf},
};

fn bits(v: f32) -> u32 {
    if v.is_nan() { 0x7fc0_0000 } else { v.to_bits() }
}

struct Sprites {
    root: PathBuf,
    /// The items atlas's paletted permutations (armour trim sprites).
    permutations: Vec<PalettedPermutations>,
    cache: RefCell<BTreeMap<String, Option<SpriteMask>>>,
}
impl SpriteLookup for Sprites {
    fn mask(&self, sprite: &str) -> Option<SpriteMask> {
        if let Some(hit) = self.cache.borrow().get(sprite) {
            return hit.clone();
        }
        let mask = self.load(sprite);
        self.cache
            .borrow_mut()
            .insert(sprite.to_owned(), mask.clone());
        mask
    }
}
impl Sprites {
    fn png(&self, sprite: &str, directory: &str) -> PathBuf {
        let (namespace, path) = sprite.split_once(':').unwrap_or(("minecraft", sprite));
        self.root
            .join(format!("assets/{namespace}/{directory}/{path}.png"))
    }
    /// The image as ARGB pixels in reading order, with its size.
    fn argb(png: &Path) -> Option<(usize, usize, Vec<u32>)> {
        let mut decoder = png::Decoder::new(BufReader::new(File::open(png).ok()?));
        decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
        let mut reader = decoder.read_info().ok()?;
        let mut bytes = vec![0; reader.output_buffer_size()?];
        let info = reader.next_frame(&mut bytes).ok()?;
        let channels = info.color_type.samples();
        let pixels = bytes[..info.buffer_size()]
            .chunks_exact(channels)
            .map(|p| {
                let (r, g, b, a) = match info.color_type {
                    png::ColorType::Rgba => (p[0], p[1], p[2], p[3]),
                    png::ColorType::Rgb => (p[0], p[1], p[2], 255),
                    png::ColorType::GrayscaleAlpha => (p[0], p[0], p[0], p[1]),
                    _ => (p[0], p[0], p[0], 255),
                };
                u32::from_be_bytes([a, r, g, b])
            })
            .collect();
        Some((info.width as usize, info.height as usize, pixels))
    }
    fn load(&self, sprite: &str) -> Option<SpriteMask> {
        let png = self.png(sprite, "textures");
        let (iw, ih, argb) = match Self::argb(&png) {
            Some(image) => image,
            None => {
                // PalettedPermutations: the base texture through PaletteMapping.
                let p = self.permutations.iter().find_map(|s| s.find(sprite))?;
                let (w, h, mut pixels) = Self::argb(&self.png(p.base, "textures"))?;
                let (_, _, key) = Self::argb(&self.png(p.palette_key, "textures/palettes"))?;
                let (_, _, target) = Self::argb(&self.png(p.palette, "textures/palettes"))?;
                apply_palette(&mut pixels, &key, &target)?;
                (w, h, pixels)
            }
        };
        let pixels: Vec<u8> = argb.iter().map(|p| (p >> 24) as u8).collect();
        let meta = fs::read_to_string(png.with_extension("png.mcmeta")).ok();
        let animation = meta
            .and_then(|m| serde_json::from_str::<Value>(&m).ok())
            .and_then(|m| m.get("animation").map(|a| a.to_string()))
            .and_then(|a| AnimationMetadata::from_json(&a).ok());
        let (w, h) = match &animation {
            Some(a) => {
                let [w, h] = a.frame_size(iw as i32, ih as i32);
                (w as usize, h as usize)
            }
            None => (iw, ih),
        };
        let columns = iw / w;
        let total = columns * (ih / h);
        let clip = animation.and_then(|a| a.clip(total as u32).ok().flatten());
        let frame_indices: Vec<usize> = match clip {
            Some(clip) => {
                let mut unique = Vec::new();
                for (index, _) in clip.sequence() {
                    if !unique.contains(&(*index as usize)) {
                        unique.push(*index as usize);
                    }
                }
                unique
            }
            None => vec![0],
        };
        let frames = frame_indices
            .into_iter()
            .map(|f| {
                let (fx, fy) = (f % columns * w, f / columns * h);
                (0..h)
                    .flat_map(|y| (0..w).map(move |x| (x, y)))
                    .map(|(x, y)| pixels[(fy + y) * iw + fx + x] == 0)
                    .collect()
            })
            .collect();
        Some(SpriteMask {
            width: w,
            height: h,
            frames,
        })
    }
}

struct Env {
    grass: Vec<i32>,
    local_time: String,
    bundle_weight: bool,
    prototypes: BTreeMap<String, Components>,
}
impl ItemEnv for Env {
    fn grass_color(&self, temperature: f64, downfall: f64) -> i32 {
        let rain = downfall * temperature;
        let x = ((1.0 - temperature) * 255.0) as i64;
        let y = ((1.0 - rain) * 255.0) as i64;
        let index = (y << 8 | x) as usize;
        self.grass.get(index).copied().unwrap_or(-65281)
    }
    fn local_time(&self, _: &str, _: &str, _: Option<&str>) -> String {
        self.local_time.clone()
    }
    fn bundle_fullness(&self, stack: &StackView, components: &Components) -> f32 {
        // Sum of count / max stack size over the contents, as a double fraction.
        let Some(items) = components
            .get("minecraft:bundle_contents")
            .and_then(Value::as_array)
        else {
            return 0.0;
        };
        let _ = (stack, self.bundle_weight);
        let mut total = 0.0f64;
        for item in items {
            let id = item["id"].as_str().unwrap_or("");
            let count = item["count"].as_i64().unwrap_or(1) as f64;
            let max = self
                .prototypes
                .get(id)
                .and_then(|c| c.get("minecraft:max_stack_size"))
                .and_then(Value::as_i64)
                .unwrap_or(1) as f64;
            total += count / max;
        }
        total as f32
    }
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<_> = std::env::args().collect();
    let root = PathBuf::from(&args[1]);
    let prototypes_path = &args[2];
    let mut output = BufWriter::new(File::create(&args[4])?);

    let raw: BTreeMap<String, Components> = serde_json::from_slice(&fs::read(prototypes_path)?)?;
    let mut models = ItemModels {
        prototypes: raw,
        ..Default::default()
    };
    writeln!(
        output,
        "{}",
        json!({"row":"components","names":DATA_COMPONENT_TYPES.to_vec()})
    )?;
    let items_dir = root.join("assets/minecraft/items");
    let mut files: Vec<_> = fs::read_dir(&items_dir)?.collect::<Result<_, _>>()?;
    files.sort_by_key(|f| f.file_name());
    for file in files {
        let name = file.file_name().to_string_lossy().into_owned();
        let id = format!("minecraft:{}", name.trim_end_matches(".json"));
        let ok = match ClientItem::from_json(&fs::read_to_string(file.path())?) {
            Ok(item) => {
                models.definitions.insert(id, item);
                true
            }
            Err(_) => false,
        };
        writeln!(output, "{}", json!({"row":"decode","name":name,"ok":ok}))?;
    }
    for line in BufReader::new(File::open(format!("{}.definitions", args[3]))?).lines() {
        let row: Value = serde_json::from_str(&line?)?;
        let name = row["name"].as_str().unwrap();
        let ok = match ClientItem::from_json(row["json"].as_str().unwrap()) {
            Ok(item) => {
                models
                    .definitions
                    .insert(horizon_assets::item_model::identifier(name)?, item);
                true
            }
            Err(_) => false,
        };
        writeln!(output, "{}", json!({"row":"decode","name":name,"ok":ok}))?;
    }

    let mut cuboids = BTreeMap::new();
    let models_dir = root.join("assets/minecraft/models");
    let mut stack = vec![models_dir.clone()];
    while let Some(dir) = stack.pop() {
        for entry in fs::read_dir(&dir)? {
            let path = entry?.path();
            if path.is_dir() {
                stack.push(path);
            } else if path.extension().is_some_and(|e| e == "json") {
                let relative = path.strip_prefix(&models_dir)?.with_extension("");
                let id = format!(
                    "minecraft:{}",
                    relative.to_string_lossy().replace('\\', "/")
                );
                if let Ok(model) = CuboidModel::from_json(&fs::read_to_string(&path)?) {
                    cuboids.insert(id, model);
                }
            }
        }
    }
    let library = ModelLibrary::with_item_generator(cuboids)?;
    let sprites = Sprites {
        root: root.clone(),
        permutations: PalettedPermutations::from_atlas(
            &fs::read_to_string(root.join("assets/minecraft/atlases/items.json"))
                .unwrap_or_default(),
        ),
        cache: RefCell::default(),
    };
    let grass = {
        let mut decoder = png::Decoder::new(BufReader::new(File::open(
            root.join("assets/minecraft/textures/colormap/grass.png"),
        )?));
        decoder.set_transformations(png::Transformations::EXPAND);
        let mut reader = decoder.read_info()?;
        let mut bytes = vec![0; reader.output_buffer_size().unwrap()];
        let info = reader.next_frame(&mut bytes)?;
        let channels = info.color_type.samples();
        bytes[..info.buffer_size()]
            .chunks_exact(channels)
            .map(|p| {
                let a = if channels == 4 { p[3] } else { 255 };
                i32::from_be_bytes([a, p[0], p[1], p[2]])
            })
            .collect()
    };
    let mut env = Env {
        grass,
        local_time: String::new(),
        bundle_weight: true,
        prototypes: models.prototypes.clone(),
    };
    let mut baked_cache: BTreeMap<String, BakedItemModel> = BTreeMap::new();
    let mut bake = |name: &str| -> BakedItemModel {
        baked_cache
            .entry(name.to_owned())
            .or_insert_with(|| {
                bake_item_model(&library, &sprites, name)
                    .or_else(|_| bake_item_model(&library, &sprites, MISSING_MODEL))
                    .expect("missing model bakes")
            })
            .clone()
    };

    for line in BufReader::new(File::open(&args[3])?).lines() {
        let row: Value = serde_json::from_str(&line?)?;
        let id = row["id"].as_str().unwrap();
        let mut stack = StackView::new(
            row["item"].as_str().unwrap(),
            row["count"].as_i64().unwrap() as i32,
        );
        if let Some(components) = row["components"].as_object() {
            for (key, value) in components {
                match key.strip_prefix('!') {
                    Some(removed) => stack.patch.insert(removed.to_owned(), None),
                    None => stack.patch.insert(key.clone(), Some(value.clone())),
                };
            }
        }
        stack.bundle_selected = row["bundle_selected"].as_i64().unwrap_or(-1) as i32;
        env.local_time = row["local_time"].as_str().unwrap_or("").to_owned();
        let context = DisplayContext::by_name(row["context"].as_str().unwrap()).unwrap();
        let owner = row["owner"].as_object().map(|o| {
            let derived = &row["derived"];
            OwnerState {
                entity_type: o["entity_type"].as_str().unwrap().to_owned(),
                main_arm_left: o["main_arm"] == "left",
                using_this_item: o["using"].as_bool().unwrap(),
                use_item_remaining_ticks: o["remaining"].as_i64().unwrap() as i32,
                use_duration: derived["use_duration"].as_i64().unwrap_or(0) as i32,
                crossbow_charge_duration: derived["charge_duration"].as_i64().unwrap_or(0) as i32,
                ..Default::default()
            }
        });
        let level = row["dimension"].as_str().map(|d| LevelState {
            dimension: d.to_owned(),
            ..Default::default()
        });
        let ctx = Context {
            display: context,
            owner: owner.as_ref(),
            level: level.as_ref(),
            seed: row["seed"].as_i64().unwrap_or(0) as i32,
        };
        let resolved = models.resolve(&stack, &ctx, &env);
        let mut layers = Vec::new();
        for layer in &resolved.layers {
            let (kind, baked, extra) = match &layer.content {
                LayerContent::Model { model } => ("model", bake(model), json!({})),
                LayerContent::Missing => ("model", bake(MISSING_MODEL), json!({})),
                LayerContent::Special {
                    base,
                    model,
                    argument,
                } => (
                    "special",
                    bake(base),
                    json!({"special":model.to_json(),"argument":argument}),
                ),
            };
            let matrix: Mat4 = baked.layer_matrix(context, &layer.transform);
            let mut value = json!({
                "kind":kind,
                "tints":layer.tints,
                "foil":match layer.foil { FoilType::None=>"none", FoilType::Standard=>"standard", FoilType::Special=>"special" },
                "matrix":matrix.rows().iter().flatten().map(|v| bits(*v)).collect::<Vec<_>>(),
                "uses_block_light":baked.uses_block_light,
                "particle":baked.particle.sprite,
                "has_transform":baked.transform(context).is_some(),
            });
            if kind == "model" {
                value["quads"] = Value::Array(
                    baked
                        .quads
                        .iter()
                        .map(|q| {
                            json!({
                                "positions":q.positions.map(|v| v.map(bits)),
                                "uv":q.uv.map(|v| v.map(bits)),
                                "facing":q.facing.name(),
                                "tint":q.tint_index,
                                "sprite":q.material.sprite,
                                "shade":q.shade_direction_override.map(Direction::name),
                                "light":q.light_emission,
                            })
                        })
                        .collect(),
                );
            }
            if let (Some(target), Some(source)) = (value.as_object_mut(), extra.as_object()) {
                for (k, v) in source {
                    target.insert(k.clone(), v.clone());
                }
            }
            layers.push(value);
        }
        writeln!(
            output,
            "{}",
            json!({"row":"case","id":id,"layers":layers,"oversized":resolved.oversized_in_gui,
                "swap":{"play":models.hand_animation_on_swap(&stack),"scale":bits(models.swap_animation_scale(&stack))}})
        )?;
    }
    let _ = Path::new("");
    Ok(())
}
