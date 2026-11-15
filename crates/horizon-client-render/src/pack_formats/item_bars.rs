//! Polytone's item modifiers' durability bar colours, on the engine's
//! [`crate::pack_palette::ItemBars`].
use crate::resource_source::ResourceSource;
#[cfg(test)]
use std::path::Path;
use std::sync::Arc;

/// An item modifier's `bar_color` of the `damage` type: a row of colours picked by the stack's
/// damage, the first pixel undamaged, the last at full damage (measured on the real mod).
struct DamageBar {
    targets: horizon_polytone::Targets,
    row: Vec<u32>,
}

/// The packs' damage bars, the last read first.
struct Bars(Vec<DamageBar>);

impl crate::pack_palette::ItemBars for Bars {
    fn color(&self, item: &str, damage: f32) -> Option<u32> {
        let bar = self
            .0
            .iter()
            .rev()
            .find(|b| b.targets.matches(item, |_| false))?;
        let last = (bar.row.len() - 1) as f32;
        Some(bar.row[(damage.clamp(0.0, 1.0) * last).round() as usize])
    }
}

/// Reads the packs' item modifiers' durability bar colours under a pack overlay root: an
/// `<item>_bar.png` alone, or a json whose `bar_color` is `damage` with its `<name>_bar.png` (or
/// `<name>.png`) beside it. Colormap ids and inline colormaps aren't read. `None` without any.
pub fn read(root: &ResourceSource) -> Option<Arc<dyn crate::pack_palette::ItemBars>> {
    let bars: Vec<_> = root
        .list_under("assets", &["polytone/item_modifiers"])
        .into_iter()
        .filter_map(|path| item_bar(root, &path))
        .collect();
    (!bars.is_empty()).then(|| Arc::new(Bars(bars)) as _)
}

fn item_bar(root: &ResourceSource, path: &str) -> Option<DamageBar> {
    let (parent, name) = path.rsplit_once('/')?;
    let row = |png: &str| {
        let bytes = root.read(png).ok()?;
        let (width, _, rgba) = crate::hud::decode_image_bytes(&bytes).ok()?;
        let row: Vec<u32> = rgba[..width as usize * 4]
            .as_chunks::<4>()
            .0
            .iter()
            .map(|p| u32::from(p[0]) << 16 | u32::from(p[1]) << 8 | u32::from(p[2]))
            .collect();
        (!row.is_empty()).then_some(row)
    };
    if let Some(stem) = name.strip_suffix("_bar.png") {
        // A json of the same name owns its png.
        if root.read(&format!("{parent}/{stem}.json")).is_ok() {
            return None;
        }
        let namespace = path.strip_prefix("assets/")?.split('/').next()?;
        let relative = path
            .split_once("/polytone/item_modifiers/")?
            .1
            .strip_suffix("_bar.png")?;
        let id = format!("{namespace}:{relative}");
        let targets = horizon_polytone::Targets::parse(None, Some(&id)).ok()?;
        return Some(DamageBar {
            targets,
            row: row(path)?,
        });
    }
    let stem = name.strip_suffix(".json")?;
    let json: serde_json::Value = serde_json::from_slice(&root.read(path).ok()?).ok()?;
    if json.get("bar_color")?.as_str()? != "damage" {
        return None;
    }
    let png = [format!("{stem}_bar.png"), format!("{stem}.png")]
        .into_iter()
        .map(|png| format!("{parent}/{png}"))
        .find(|png| root.read(png).is_ok())?;
    let implicit = horizon_polytone::implicit_id(path, "item_modifiers");
    let targets =
        horizon_polytone::Targets::parse(json.get("targets"), implicit.as_deref()).ok()?;
    Some(DamageBar {
        targets,
        row: row(&png)?,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn png_row(path: &Path, pixels: &[[u8; 3]]) {
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        let file = std::fs::File::create(path).unwrap();
        let mut encoder = png::Encoder::new(file, pixels.len() as u32, 1);
        encoder.set_color(png::ColorType::Rgba);
        let data: Vec<u8> = pixels
            .iter()
            .flat_map(|p| [p[0], p[1], p[2], 255])
            .collect();
        encoder
            .write_header()
            .unwrap()
            .write_image_data(&data)
            .unwrap();
    }

    #[test]
    fn a_damage_bar_picks_its_pixel_by_the_stacks_damage() {
        let dir = std::env::temp_dir().join(format!("pack-bars-{}", std::process::id()));
        let modifiers = dir.join("assets/minecraft/polytone/item_modifiers");
        let rgby = [[255, 0, 0], [0, 255, 0], [0, 0, 255], [255, 255, 0]];
        png_row(&modifiers.join("diamond_pickaxe_bar.png"), &rgby);
        std::fs::write(
            modifiers.join("tools.json"),
            r#"{"targets": ["minecraft:iron_axe"], "bar_color": "damage"}"#,
        )
        .unwrap();
        png_row(&modifiers.join("tools_bar.png"), &[[1, 2, 3], [4, 5, 6]]);
        let bars = read(&ResourceSource::directory(dir.clone())).unwrap();
        // The real mod: damage 200 of 1561 shows the first pixel, 1400 the last.
        assert_eq!(
            bars.color("minecraft:diamond_pickaxe", 200.0 / 1561.0),
            Some(0xFF0000)
        );
        assert_eq!(
            bars.color("minecraft:diamond_pickaxe", 1400.0 / 1561.0),
            Some(0xFFFF00)
        );
        assert_eq!(bars.color("minecraft:iron_axe", 0.9), Some(0x040506));
        assert_eq!(bars.color("minecraft:diamond_shovel", 0.5), None);
        assert!(read(&ResourceSource::directory(dir.join("none"))).is_none());
        std::fs::remove_dir_all(&dir).ok();
    }
}
