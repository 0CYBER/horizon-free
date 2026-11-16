//! Atlas sprite sources (`SpriteSourceList`): the `atlases/*.json`
//! definitions listing which sprites an atlas holds. `directory`, `single`,
//! `filter`, `unstitch` and `paletted_permutations` sources, and the types mods
//! register ([`crate::mod_sprite_sources`]), run in order against the resource
//! tree; a later source replaces an earlier sprite of the same id and a filter
//! removes the matching ones.
use horizon_assets::palette::apply_palette;
use horizon_client_core::pattern::Pattern;
use horizon_core::nbt::Tag;
use mod_api::{Identifier, SpriteSourceOutput};
use serde_json::Value;
use std::{
    collections::BTreeMap,
    path::{Path, PathBuf},
    sync::Arc,
};

/// One region of an `unstitch` source, in units of the divisors.
#[derive(Clone, Debug, PartialEq)]
pub struct Region {
    pub sprite: String,
    pub x: f64,
    pub y: f64,
    pub width: f64,
    pub height: f64,
}
#[derive(Clone, Debug)]
pub enum SpriteSource {
    Directory {
        source: String,
        prefix: String,
    },
    Single {
        resource: String,
        sprite: Option<String>,
    },
    Filter {
        namespace: Option<Pattern>,
        path: Option<Pattern>,
    },
    Unstitch {
        resource: String,
        regions: Vec<Region>,
        divisor_x: f64,
        divisor_y: f64,
    },
    PalettedPermutations {
        textures: Vec<String>,
        palette_key: String,
        permutations: BTreeMap<String, String>,
        separator: String,
    },
    /// An entry of a type a mod registered: its source answers it when the atlas is listed.
    Registered {
        /// The type id (`mymod:checker`).
        kind: String,
        /// The entry's object, converted once at load.
        config: Arc<Tag>,
    },
}
/// Where a listed sprite's pixels come from.
#[derive(Clone, Debug, PartialEq)]
pub enum SpriteOrigin {
    /// A whole texture (`textures/<path>.png`), by texture identifier.
    File(String),
    /// `Unstitcher.RegionInstance`: part of a texture.
    Region {
        texture: String,
        region: Region,
        divisor_x: f64,
        divisor_y: f64,
    },
    /// `PalettedSpriteSupplier`: a texture with its colours remapped.
    Paletted {
        texture: String,
        palette_key: String,
        palette: String,
    },
    /// Pixels a mod's sprite source made.
    Pixels(Image),
}

/// `Identifier.parse` default namespace.
pub fn qualified(id: &str) -> String {
    if id.contains(':') {
        id.to_owned()
    } else {
        format!("minecraft:{id}")
    }
}
fn string(v: &Value, key: &str) -> Option<String> {
    v.get(key)?.as_str().map(str::to_owned)
}
fn number(v: &Value, key: &str, default: f64) -> Option<f64> {
    match v.get(key) {
        None => Some(default),
        Some(n) => n.as_f64(),
    }
}

/// Why an atlas definition can't be decoded ([`decode`]).
#[derive(Debug)]
pub enum DefinitionError {
    /// The file isn't JSON.
    Json(serde_json::Error),
    /// It has no `sources` list.
    NoSources,
    /// The entry at `index` doesn't decode as its type.
    Malformed { index: usize },
    /// The entry at `index` is of a type neither built in nor registered by a mod.
    UnknownType { index: usize, kind: String },
}
impl std::fmt::Display for DefinitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Json(error) => error.fmt(f),
            Self::NoSources => f.write_str("no `sources` list"),
            Self::Malformed { index } => write!(f, "sources[{index}]: malformed source"),
            Self::UnknownType { index, kind } => {
                write!(f, "sources[{index}]: unknown sprite source type {kind}")
            }
        }
    }
}
impl std::error::Error for DefinitionError {}

/// `SpriteSources.FILE_CODEC`: the `sources` list of an atlas definition.
/// A malformed source fails the whole file, as the codec does; the error names
/// the first one, and its type when no source has it.
pub fn decode(text: &str) -> Result<Vec<SpriteSource>, DefinitionError> {
    let json: Value = serde_json::from_str(text).map_err(DefinitionError::Json)?;
    let sources = json
        .get("sources")
        .and_then(Value::as_array)
        .ok_or(DefinitionError::NoSources)?;
    sources
        .iter()
        .enumerate()
        .map(|(index, source)| {
            let kind = source.get("type").and_then(Value::as_str).map(qualified);
            decode_source(source).ok_or_else(|| match kind {
                Some(kind) if !known(&kind) => DefinitionError::UnknownType { index, kind },
                _ => DefinitionError::Malformed { index },
            })
        })
        .collect()
}
/// Whether a source of type `kind` can be decoded: a built-in one or one a mod registered.
fn known(kind: &str) -> bool {
    matches!(
        kind,
        "minecraft:directory"
            | "minecraft:single"
            | "minecraft:filter"
            | "minecraft:unstitch"
            | "minecraft:paletted_permutations"
    ) || crate::mod_sprite_sources::is_registered(kind)
}
fn decode_source(s: &Value) -> Option<SpriteSource> {
    Some(match qualified(s.get("type")?.as_str()?).as_str() {
        "minecraft:directory" => SpriteSource::Directory {
            source: string(s, "source")?,
            prefix: string(s, "prefix")?,
        },
        "minecraft:single" => SpriteSource::Single {
            resource: qualified(&string(s, "resource")?),
            sprite: match s.get("sprite") {
                None => None,
                Some(v) => Some(qualified(v.as_str()?)),
            },
        },
        "minecraft:filter" => {
            let p = s.get("pattern")?;
            let pattern = |key: &str| -> Option<Option<Pattern>> {
                match p.get(key) {
                    None => Some(None),
                    Some(v) => Pattern::new(v.as_str()?).map(Some),
                }
            };
            SpriteSource::Filter {
                namespace: pattern("namespace")?,
                path: pattern("path")?,
            }
        }
        "minecraft:unstitch" => {
            let regions = s
                .get("regions")?
                .as_array()?
                .iter()
                .map(|r| {
                    Some(Region {
                        sprite: qualified(&string(r, "sprite")?),
                        x: r.get("x")?.as_f64()?,
                        y: r.get("y")?.as_f64()?,
                        width: r.get("width")?.as_f64()?,
                        height: r.get("height")?.as_f64()?,
                    })
                })
                .collect::<Option<Vec<_>>>()?;
            if regions.is_empty() {
                return None;
            }
            SpriteSource::Unstitch {
                resource: qualified(&string(s, "resource")?),
                regions,
                divisor_x: number(s, "divisor_x", 1.0)?,
                divisor_y: number(s, "divisor_y", 1.0)?,
            }
        }
        "minecraft:paletted_permutations" => SpriteSource::PalettedPermutations {
            textures: s
                .get("textures")?
                .as_array()?
                .iter()
                .map(|t| t.as_str().map(qualified))
                .collect::<Option<_>>()?,
            palette_key: qualified(&string(s, "palette_key")?),
            permutations: s
                .get("permutations")?
                .as_object()?
                .iter()
                .map(|(k, v)| Some((k.clone(), qualified(v.as_str()?))))
                .collect::<Option<_>>()?,
            separator: match s.get("separator") {
                None => "_".to_owned(),
                Some(v) => v.as_str()?.to_owned(),
            },
        },
        kind if crate::mod_sprite_sources::is_registered(kind) => SpriteSource::Registered {
            kind: kind.to_owned(),
            config: Arc::new(horizon_core::json_nbt::json_to_tag(s).ok()?),
        },
        _ => return None,
    })
}

/// The texture files of a resource tree: `namespace:path` for every
/// `assets/<namespace>/textures/<path>.png`.
pub fn list_textures(source: &crate::resource_source::ResourceSource) -> Vec<String> {
    source
        .list_under("assets", &["textures"])
        .into_iter()
        .filter_map(|file| {
            let (namespace, texture) = file.strip_prefix("assets/")?.split_once("/textures/")?;
            Some(format!("{namespace}:{}", texture.strip_suffix(".png")?))
        })
        .collect()
}
/// The file of a texture identifier. A Custom Item Textures id (`cit_assets`) names its
/// resource path, wherever the folder: `directory` is not used for it. An id whose path would
/// leave the root (a pack's model may write any) names a file that is never there.
pub fn texture_path(root: &Path, texture: &str, directory: &str) -> PathBuf {
    root.join(texture_resource(texture, directory))
}
/// Resource name of a texture, including Custom Item Textures outside `textures/`.
pub fn texture_resource(texture: &str, directory: &str) -> String {
    // No file can have a NUL in its name, so this one is never there.
    const NOWHERE: &str = "assets/minecraft/textures/horizon/\0refused.png";
    if texture.starts_with("horizon_cit:") {
        return crate::cit_assets::resource(texture, "png").unwrap_or_else(|| NOWHERE.to_owned());
    }
    let (namespace, path) = texture.split_once(':').unwrap_or(("minecraft", texture));
    let rel = format!("assets/{namespace}/{directory}/{path}.png");
    if horizon_optifine::paths::valid(&rel) {
        rel
    } else {
        NOWHERE.to_owned()
    }
}

/// What a mod's source answers for an entry of its type: the type, the entry's
/// object and the sprites listed before it ([`crate::mod_sprite_sources::ask`]).
pub type Ask<'a> = dyn FnMut(&str, &Tag, &[Identifier]) -> Vec<SpriteSourceOutput> + 'a;

/// The `ask` of an atlas Horizon doesn't stitch from its definition (its block, item and GUI
/// atlases): a mod's source there adds nothing, so what is listed matches what is stitched.
pub fn unasked(_: &str, _: &Tag, _: &[Identifier]) -> Vec<SpriteSourceOutput> {
    Vec::new()
}

/// `SpriteSourceList.list`: every sprite id of the atlas and its origin.
/// `textures` are the available texture identifiers ([`list_textures`]);
/// `ask` answers the entries of a type a mod registered.
pub fn list(
    sources: &[SpriteSource],
    textures: &[String],
    ask: &mut Ask<'_>,
) -> BTreeMap<String, SpriteOrigin> {
    let mut out = BTreeMap::new();
    let exists = |t: &str| textures.binary_search_by(|x| x.as_str().cmp(t)).is_ok();
    for source in sources {
        match source {
            SpriteSource::Directory { source, prefix } => {
                // FileToIdConverter("textures/" + source, ".png").
                let dir = format!("{source}/");
                for t in textures {
                    let (namespace, path) = t.split_once(':').unwrap_or(("minecraft", t));
                    if let Some(rest) = path.strip_prefix(&dir) {
                        out.insert(
                            format!("{namespace}:{prefix}{rest}"),
                            SpriteOrigin::File(t.clone()),
                        );
                    }
                }
            }
            SpriteSource::Single { resource, sprite } => {
                if exists(resource) {
                    out.insert(
                        sprite.clone().unwrap_or_else(|| resource.clone()),
                        SpriteOrigin::File(resource.clone()),
                    );
                }
            }
            SpriteSource::Filter { namespace, path } => {
                out.retain(|id: &String, _| {
                    let (ns, p) = id.split_once(':').unwrap_or(("minecraft", id));
                    !(namespace.as_ref().is_none_or(|m| m.is_match(ns))
                        && path.as_ref().is_none_or(|m| m.is_match(p)))
                });
            }
            SpriteSource::Unstitch {
                resource,
                regions,
                divisor_x,
                divisor_y,
            } => {
                if exists(resource) {
                    for region in regions {
                        out.insert(
                            region.sprite.clone(),
                            SpriteOrigin::Region {
                                texture: resource.clone(),
                                region: region.clone(),
                                divisor_x: *divisor_x,
                                divisor_y: *divisor_y,
                            },
                        );
                    }
                }
            }
            SpriteSource::PalettedPermutations {
                textures: bases,
                palette_key,
                permutations,
                separator,
            } => {
                for base in bases {
                    if !exists(base) {
                        continue;
                    }
                    for (suffix, palette) in permutations {
                        out.insert(
                            format!("{base}{separator}{suffix}"),
                            SpriteOrigin::Paletted {
                                texture: base.clone(),
                                palette_key: palette_key.clone(),
                                palette: palette.clone(),
                            },
                        );
                    }
                }
            }
            SpriteSource::Registered { kind, config } => {
                let listed: Vec<Identifier> = out
                    .keys()
                    .filter_map(|id| Identifier::parse(id).ok())
                    .collect();
                for output in ask(kind, config, &listed) {
                    apply(kind, output, &exists, &mut out);
                }
            }
        }
    }
    out
}

/// One step of a mod's source on the listed sprites, as `SpriteSource.Output` takes it. What
/// can't be a sprite is logged and skipped by itself.
fn apply(
    kind: &str,
    output: SpriteSourceOutput,
    exists: &dyn Fn(&str) -> bool,
    out: &mut BTreeMap<String, SpriteOrigin>,
) {
    match output {
        SpriteSourceOutput::Texture { sprite, texture } => {
            let texture = texture.to_string();
            if exists(&texture) {
                out.insert(sprite.to_string(), SpriteOrigin::File(texture));
            } else {
                log::warn!("sprite source {kind}: sprite {sprite}: no texture {texture}; skipped");
            }
        }
        SpriteSourceOutput::Pixels {
            sprite,
            width,
            height,
            rgba,
        } => {
            // A size from mod code: checked arithmetic, so no input can overflow or panic.
            let length = (width as usize)
                .checked_mul(height as usize)
                .and_then(|pixels| pixels.checked_mul(4));
            if width == 0 || height == 0 {
                log::warn!("sprite source {kind}: sprite {sprite} has no pixels; skipped");
            } else if length != Some(rgba.len()) {
                log::warn!(
                    "sprite source {kind}: sprite {sprite}: {} bytes for {width}x{height} pixels \
                     (4 a pixel); skipped",
                    rgba.len()
                );
            } else if i64::from(width.max(height)) > MAX_SPRITE_SIDE {
                log::warn!(
                    "sprite source {kind}: sprite {sprite}: {width}x{height} exceeds the \
                     {MAX_SPRITE_SIDE}-pixel sprite side; skipped"
                );
            } else {
                out.insert(
                    sprite.to_string(),
                    SpriteOrigin::Pixels((width, height, rgba)),
                );
            }
        }
        SpriteSourceOutput::Remove { sprite } => {
            out.remove(&sprite.to_string());
        }
    }
}

/// An RGBA image.
pub type Image = (u32, u32, Vec<u8>);

/// The largest side of a sprite an `unstitch` region may cut or a mod's source may make, in
/// pixels. Vanilla's `Unstitcher` and `SpriteSource.Output` have no limit of their own, but its
/// stitcher can't place a sprite larger than the device's maximum texture size, and the renderer
/// asks for the adapter's own limits. 32768 is the largest `max_texture_dimension_2d` current
/// drivers report (NVIDIA's Vulkan; Direct3D 12 and Metal stop at 16384), so no sprite vanilla
/// would stitch is refused. A larger declared sprite is skipped with a logged reason, never
/// allocated.
pub const MAX_SPRITE_SIDE: i64 = 32768;

fn argb(rgba: &[u8]) -> Vec<u32> {
    rgba.as_chunks::<4>()
        .0
        .iter()
        .map(|p| u32::from_be_bytes([p[3], p[0], p[1], p[2]]))
        .collect()
}
/// The pixels of a generated sprite (`Region`, `Paletted` or a mod's `Pixels`);
/// `None` for a plain file, which loads as itself.
pub fn generate(
    origin: &SpriteOrigin,
    load: &dyn Fn(&str, &str) -> Option<Image>,
) -> Option<Image> {
    match origin {
        SpriteOrigin::File(_) => None,
        SpriteOrigin::Pixels(image) => Some(image.clone()),
        SpriteOrigin::Region {
            texture,
            region,
            divisor_x,
            divisor_y,
        } => {
            let (w, h, pixels) = load(texture, "textures")?;
            let x_scale = f64::from(w) / divisor_x;
            let y_scale = f64::from(h) / divisor_y;
            // A float-to-integer cast saturates (NaN is 0), so every value here is in range.
            let x = (region.x * x_scale).floor() as i64;
            let y = (region.y * y_scale).floor() as i64;
            let width = (region.width * x_scale).floor() as i64;
            let height = (region.height * y_scale).floor() as i64;
            if width <= 0 || height <= 0 {
                return None;
            }
            // The region and divisors come from an atlas definition a mod or pack writes: refuse a
            // sprite over the stated side before sizing anything.
            if width > MAX_SPRITE_SIDE || height > MAX_SPRITE_SIDE {
                log::warn!(
                    "atlas region {} of {texture}: {width}x{height} exceeds the {MAX_SPRITE_SIDE}-pixel \
                     sprite side; skipped",
                    region.sprite
                );
                return None;
            }
            // Up to 4 GiB at the largest side: as vanilla's `NativeImage` allocation does, a
            // refused allocation skips the sprite instead of ending the process.
            let bytes = (width * height * 4) as usize;
            let mut out = Vec::new();
            if out.try_reserve_exact(bytes).is_err() {
                log::warn!(
                    "atlas region {} of {texture}: {width}x{height} could not be allocated; skipped",
                    region.sprite
                );
                return None;
            }
            out.resize(bytes, 0u8);
            for row in 0..height {
                for col in 0..width {
                    // The offset is the pack's too: saturate, and the bounds check skips it.
                    let (sx, sy) = (x.saturating_add(col), y.saturating_add(row));
                    if sx < 0 || sy < 0 || sx >= i64::from(w) || sy >= i64::from(h) {
                        continue;
                    }
                    let s = ((sy * i64::from(w) + sx) * 4) as usize;
                    let d = ((row * width + col) * 4) as usize;
                    out[d..d + 4].copy_from_slice(&pixels[s..s + 4]);
                }
            }
            Some((width as u32, height as u32, out))
        }
        SpriteOrigin::Paletted {
            texture,
            palette_key,
            palette,
        } => {
            let (w, h, pixels) = load(texture, "textures")?;
            let (_, _, key) = load(palette_key, "textures/palettes")?;
            let (_, _, target) = load(palette, "textures/palettes")?;
            let mut mapped = argb(&pixels);
            let key = argb(&key);
            let target = argb(&target);
            apply_palette(&mut mapped, &key, &target)?;
            let out: Vec<u8> = mapped
                .iter()
                .flat_map(|p| {
                    let [a, r, g, b] = p.to_be_bytes();
                    [r, g, b, a]
                })
                .collect();
            drop(mapped);
            Some((w, h, out))
        }
    }
}

/// Every atlas definition of a resource tree (`atlases/*.json` in every
/// namespace), by atlas id.
pub fn atlases(
    source: &crate::resource_source::ResourceSource,
) -> BTreeMap<String, Vec<SpriteSource>> {
    source
        .list_under("assets", &["atlases"])
        .into_iter()
        .filter_map(|file| {
            let (namespace, path) = file.strip_prefix("assets/")?.split_once("/atlases/")?;
            let name = path.strip_suffix(".json")?;
            let sources = match decode(&source.read_to_string(&file).ok()?) {
                Ok(sources) => sources,
                Err(error) => {
                    log::warn!("Horizon: atlas {namespace}:{name} skipped: {error}");
                    return None;
                }
            };
            Some((format!("{namespace}:{name}"), sources))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn discovers_archive_textures_and_nested_atlas_definitions_in_place() {
        use std::io::{Cursor, Write};
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        for (name, bytes) in [
            ("assets/demo/textures/item/x.png", b"png".as_slice()),
            ("assets/zzz/textures/item/z.png", b"png"),
            ("assets/demo/atlases/custom/nested.json", br#"{"sources":[{"type":"single","resource":"demo:item/x","sprite":"demo:renamed"}]}"#),
            ("assets/demo/atlases/ignored.txt", b"ignored"),
        ] {
            writer.start_file(name, zip::write::SimpleFileOptions::default()).unwrap();
            writer.write_all(bytes).unwrap();
        }
        let source = crate::resource_source::ResourceSource::opened_archive(
            crate::zip::Zip::from_bytes(writer.finish().unwrap().into_inner()).unwrap(),
        );
        let textures = list_textures(&source);
        assert_eq!(textures, ["demo:item/x", "zzz:item/z"]);
        let atlases = atlases(&source);
        assert_eq!(atlases.keys().collect::<Vec<_>>(), [&"demo:custom/nested"]);
        assert!(
            matches!(list(&atlases["demo:custom/nested"], &textures, &mut unasked).get("demo:renamed"), Some(SpriteOrigin::File(texture)) if texture == "demo:item/x")
        );
    }

    fn textures() -> Vec<String> {
        let mut t: Vec<String> = [
            "minecraft:item/apple",
            "minecraft:item/stick",
            "minecraft:trims/items/boots_trim",
            "minecraft:block/stone",
            "minecraft:entity/sheet",
        ]
        .iter()
        .map(|s| s.to_string())
        .collect();
        t.sort();
        t
    }
    #[test]
    fn sources_run_in_order() {
        let sources = decode(
            r#"{"sources":[
                {"type":"directory","source":"item","prefix":"item/"},
                {"type":"minecraft:single","resource":"block/stone","sprite":"item/stone"},
                {"type":"filter","pattern":{"path":"stick"}},
                {"type":"unstitch","resource":"entity/sheet","divisor_x":2,"regions":[{"sprite":"a","x":1,"y":0,"width":1,"height":1}]},
                {"type":"paletted_permutations","textures":["trims/items/boots_trim"],"palette_key":"trims/color_palettes/trim_palette","permutations":{"quartz":"trims/color_palettes/quartz"}}
            ]}"#,
        )
        .unwrap();
        let listed = list(&sources, &textures(), &mut unasked);
        let ids: Vec<&str> = listed.keys().map(String::as_str).collect();
        assert_eq!(
            ids,
            [
                "minecraft:a",
                "minecraft:item/apple",
                "minecraft:item/stone",
                "minecraft:trims/items/boots_trim_quartz"
            ]
        );
        assert_eq!(
            listed["minecraft:item/stone"],
            SpriteOrigin::File("minecraft:block/stone".into())
        );
    }
    #[test]
    fn malformed_sources_fail_the_file() {
        assert_eq!(
            decode(r#"{"sources":[{"type":"directory","source":"item"}]}"#)
                .unwrap_err()
                .to_string(),
            "sources[0]: malformed source"
        );
        assert_eq!(
            decode(r#"{"sources":[{"type":"directory","source":"a","prefix":""},{"type":"unstitch","resource":"x","regions":[]}]}"#)
                .unwrap_err()
                .to_string(),
            "sources[1]: malformed source"
        );
        assert_eq!(
            decode(r#"{"sources":[{"type":"horizon"}]}"#)
                .unwrap_err()
                .to_string(),
            "sources[0]: unknown sprite source type minecraft:horizon"
        );
    }
    #[test]
    fn unstitch_cuts_by_divisor() {
        let pixels: Vec<u8> = (0..16u8).flat_map(|i| [i, i, i, 255]).collect();
        let load = |_: &str, _: &str| Some((4u32, 4u32, pixels.clone()));
        let origin = SpriteOrigin::Region {
            texture: "t".into(),
            region: Region {
                sprite: "s".into(),
                x: 1.0,
                y: 1.0,
                width: 1.0,
                height: 1.0,
            },
            divisor_x: 2.0,
            divisor_y: 2.0,
        };
        let (w, h, out) = generate(&origin, &load).unwrap();
        assert_eq!((w, h), (2, 2));
        assert_eq!(out[0], 10);
        assert_eq!(out[4], 11);
        assert_eq!(out[8], 14);
    }

    /// An unstitch region over a 4x4 texture with divisors 1: the region's numbers are pixels.
    fn region_of(x: f64, y: f64, width: f64, height: f64) -> Option<Image> {
        let pixels: Vec<u8> = (0..16u8).flat_map(|i| [i, i, i, 255]).collect();
        let load = |_: &str, _: &str| Some((4u32, 4u32, pixels.clone()));
        let origin = SpriteOrigin::Region {
            texture: "t".into(),
            region: Region {
                sprite: "s".into(),
                x,
                y,
                width,
                height,
            },
            divisor_x: 4.0,
            divisor_y: 4.0,
        };
        generate(&origin, &load)
    }

    /// A pack's declared region can't overflow the size or ask for a huge allocation: past the
    /// stated side it is skipped, whatever the numbers, and the copy's offsets can't overflow.
    #[test]
    fn a_huge_declared_region_is_refused_not_allocated() {
        for (width, height) in [
            (f64::MAX, f64::MAX),
            (i64::MAX as f64, 2.0),
            (2.0, i64::MAX as f64),
            (3_000_000_000.0, 3_000_000_000.0),
            ((MAX_SPRITE_SIDE + 1) as f64, 1.0),
            (f64::INFINITY, 1.0),
        ] {
            assert_eq!(region_of(0.0, 0.0, width, height), None, "{width}x{height}");
        }
        // The largest side is still a sprite (one row, so the test allocates little).
        let (w, h, out) = region_of(0.0, 0.0, MAX_SPRITE_SIDE as f64, 1.0).unwrap();
        assert_eq!((w, h), (MAX_SPRITE_SIDE as u32, 1));
        assert_eq!(&out[..4], [0, 0, 0, 255]);
        // An offset near the integer limit: the pixels outside the texture stay transparent.
        let (w, h, out) = region_of(i64::MAX as f64, -(i64::MAX as f64), 2.0, 2.0).unwrap();
        assert_eq!((w, h), (2, 2));
        assert!(out.iter().all(|&byte| byte == 0));
    }
}
