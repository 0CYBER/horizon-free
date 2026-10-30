//! Loom and crafter screen drawing for tools/test_screen_layout_parity.py:
//! reads one JSON case per line and prints the `horizon_client::screen_layout`
//! calls in the format of `VanillaLoomScreenOracle`.
//!
//! usage: screen_layout_fixture RESOURCES INPUT OUTPUT
use horizon_client::{
    banner::{BannerLayer, Banners, banner_color},
    screen_layout::{self, CrafterView, LoomView, ScreenDraw},
};
use serde_json::{Value, json};
use std::{fs, path::Path, sync::Arc};

fn draw_json(d: &ScreenDraw) -> Value {
    let round = |v: f32| ((v * 1e6).round() / 1e6) as f64;
    match d {
        ScreenDraw::Gradient { x1, y1 } => json!({"t": "gradient", "x1": x1, "y1": y1}),
        ScreenDraw::Blit {
            texture,
            x,
            y,
            w,
            h,
            tw,
            th,
        } => {
            json!({"t": "blit", "texture": texture, "x": x, "y": y, "w": w, "h": h, "u": 0.0, "v": 0.0, "tw": tw, "th": th})
        }
        ScreenDraw::Sprite { id, x, y, w, h } => {
            json!({"t": "sprite", "id": id, "x": x, "y": y, "w": w, "h": h})
        }
        ScreenDraw::Banner {
            base,
            layers,
            x0,
            y0,
            x1,
            y1,
        } => {
            let layers: Vec<Value> = layers
                .iter()
                .map(|l| json!([&*l.pattern, &*l.color]))
                .collect();
            json!({"t": "banner", "base": base, "layers": layers, "x0": x0, "y0": y0, "x1": x1, "y1": y1})
        }
        ScreenDraw::Tooltip { key } => json!({"t": "tooltip", "key": key}),
        ScreenDraw::SpriteOf { id } => {
            json!({"t": "sprite_of", "id": id.trim_start_matches("minecraft:")})
        }
        ScreenDraw::Fill {
            x0,
            y0,
            x1,
            y1,
            color,
        } => json!({"t": "fill", "x0": x0, "y0": y0, "x1": x1, "y1": y1, "color": color}),
        ScreenDraw::Atlas { x0, y0, x1, y1, uv } => json!({
            "t": "atlas", "x0": x0, "y0": y0, "x1": x1, "y1": y1,
            "u0": round(uv[0]), "u1": round(uv[1]), "v0": round(uv[2]), "v1": round(uv[3])
        }),
    }
}

pub fn main() -> horizon_client::Result<()> {
    let args: Vec<String> = crate::args().collect();
    let usage = "usage: screen_layout_fixture RESOURCES INPUT OUTPUT";
    let banners = Banners::load(Path::new(args.get(1).ok_or(usage)?));
    let input = fs::read_to_string(args.get(2).ok_or(usage)?)?;
    let mut out = String::new();
    for line in input.lines().filter(|l| !l.trim().is_empty()) {
        let c: Value = serde_json::from_str(line)?;
        let int = |k: &str| c[k].as_i64().unwrap_or(0) as i32;
        let draws = if c["screen"] == "loom" {
            let patterns: Vec<Arc<str>> = c["patterns"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|p| p.as_str().map(Arc::from))
                .collect();
            let layers: Vec<BannerLayer> = c["resultLayers"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|l| BannerLayer::new(l[0].as_str().unwrap_or(""), l[1].as_str().unwrap_or("")))
                .collect();
            let base = c["result"]
                .as_str()
                .and_then(banner_color)
                .unwrap_or("white");
            let dye = c["dyeStack"]
                .as_str()
                .and_then(|d| d.strip_prefix("minecraft:")?.strip_suffix("_dye"))
                .unwrap_or("white");
            let view = LoomView {
                width: int("width"),
                height: int("height"),
                left: int("left"),
                top: int("top"),
                has_banner: c["banner"].is_string(),
                has_dye: c["dye"].is_string(),
                has_pattern: c["pattern"].is_string(),
                patterns: &patterns,
                selected: int("selected"),
                display: c["display"].as_bool().unwrap_or(false),
                has_max: c["hasMax"].as_bool().unwrap_or(false),
                scroll: c["scroll"].as_f64().unwrap_or(0.0) as f32,
                start_row: int("startRow") as usize,
                result: c.get("resultLayers").map(|_| (base, layers.as_slice())),
                dye,
                mouse: (int("mouseX") as f32, int("mouseY") as f32),
            };
            screen_layout::loom(&view, &banners)
        } else {
            let mut disabled = [false; 9];
            for (i, d) in c["disabled"]
                .as_array()
                .into_iter()
                .flatten()
                .enumerate()
                .take(9)
            {
                disabled[i] = d.as_bool().unwrap_or(false);
            }
            screen_layout::crafter(&CrafterView {
                width: int("width"),
                height: int("height"),
                left: int("left"),
                top: int("top"),
                disabled,
                powered: c["powered"].as_bool().unwrap_or(false),
            })
        };
        let list: Vec<Value> = draws.iter().map(draw_json).collect();
        out += &json!({ "draws": list }).to_string();
        out.push('\n');
    }
    fs::write(args.get(3).ok_or(usage)?, out)?;
    Ok(())
}
