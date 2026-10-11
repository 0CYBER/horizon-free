//! Headless transport for the HUD parity gate: replays each player state
//! through `horizon_client::hud_layout` (crosshair, hotbar, health, mount
//! hearts, experience bar and level) and writes the draw calls in vanilla's
//! GuiGraphicsExtractor terms.
#![forbid(unsafe_code)]
use horizon_client::{
    hud::{HudAtlas, HudBatch},
    hud_layout::{self, GuiDraw, HealthHud, HealthInputs, HeartKind, HotbarInputs},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufWriter, Write},
    path::PathBuf,
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn main() -> Result<()> {
    let args: Vec<PathBuf> = std::env::args_os().skip(1).map(PathBuf::from).collect();
    let [root, inputs, output] = args.as_slice() else {
        return Err("usage: hud_fixture RESOURCES INPUTS OUTPUT".into());
    };
    let atlas = HudAtlas::build(root)?;
    let mut out = BufWriter::new(fs::File::create(output)?);
    for line in fs::read_to_string(inputs)?
        .lines()
        .filter(|l| !l.is_empty())
    {
        let c: Value = serde_json::from_str(line)?;
        let int = |k: &str, d: i64| c[k].as_i64().unwrap_or(d) as i32;
        let float = |k: &str, d: f64| c[k].as_f64().unwrap_or(d) as f32;
        let (w, h) = (int("width", 0), int("height", 0));
        let mode = c["gameMode"].as_str().unwrap_or("survival");
        let survival = mode == "survival" || mode == "adventure";
        let indicator = c["attackIndicator"].as_str().unwrap_or("OFF");
        let partial = float("partial", 0.0);
        let effects: Vec<&str> = c["effects"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .collect();
        let has = |e: &str| effects.contains(&e);
        let mut draws = Vec::new();
        // Spectators only see it over a spectatable target (none here).
        if c["crosshair"].as_bool().unwrap_or(false) && mode != "spectator" {
            let attack = (indicator == "CROSSHAIR").then(|| float("attackScale", 1.0));
            draws.extend(hud_layout::crosshair(w, h, attack, false));
        }
        if mode != "spectator" {
            let pop = |v: &Value| v.as_f64().map(|p| p as f32 - partial);
            let slots: [Option<f32>; 9] = std::array::from_fn(|i| pop(&c["slots"][i]));
            let inputs = HotbarInputs {
                selected: int("selected", 0),
                right_handed: c["rightHanded"].as_bool().unwrap_or(true),
                offhand: !c["offhand"].is_null(),
                slots,
                offhand_pop: pop(&c["offhand"]).unwrap_or(0.0),
                attack: (indicator == "HOTBAR").then(|| float("attackScale", 1.0)),
            };
            draws.extend(hud_layout::hotbar(w, h, &inputs));
        }
        let vehicle_hearts = if c.get("vehicleMaxHealth").is_some() {
            hud_layout::vehicle_max_hearts(float("vehicleMaxHealth", 0.0))
        } else {
            0
        };
        if survival {
            let millis = 1_000_000_000i64;
            let mut hud = HealthHud {
                last_health: int("lastHealth", 0),
                display_health: int("displayHealth", 0),
                last_health_time: millis - i64::from(int("lastHealthAgo", 0)),
                health_blink_time: i64::from(int("healthBlinkTime", 0)),
            };
            let kind = if has("minecraft:poison") {
                HeartKind::Poisoned
            } else if has("minecraft:wither") {
                HeartKind::Withered
            } else if c["frozen"].as_bool().unwrap_or(false) {
                HeartKind::Frozen
            } else {
                HeartKind::Normal
            };
            let inputs = HealthInputs {
                tick: int("tick", 0),
                millis,
                health: float("health", 20.0),
                max_health: float("maxHealth", 20.0),
                absorption: float("absorption", 0.0),
                armor: int("armor", 0),
                food: int("food", 20),
                saturation: float("saturation", 5.0),
                kind,
                regeneration: has("minecraft:regeneration"),
                hunger: has("minecraft:hunger"),
                hardcore: c["hardcore"].as_bool().unwrap_or(false),
                damage_cooldown: int("damageCooldown", 0) > 0,
                air: int("air", 300),
                max_air: int("maxAir", 300),
                under_water: c["underWater"].as_bool().unwrap_or(false),
                vehicle_hearts,
            };
            draws.extend(hud.layout(&inputs, w, h));
        }
        if vehicle_hearts > 0 {
            draws.extend(hud_layout::vehicle_health(
                w,
                h,
                vehicle_hearts,
                float("vehicleHealth", 0.0),
            ));
        }
        let width = |s: &str| HudBatch::text_width(&atlas, s) as i32;
        let level = if survival { int("xpLevel", 0) } else { 0 };
        draws.extend(hud_layout::experience(
            w,
            h,
            float("xpProgress", 0.0),
            level,
            c["experienceBar"].as_bool().unwrap_or(false),
            &width,
        ));
        let draws: Vec<Value> = draws
            .iter()
            .map(|d| match d {
                GuiDraw::Sprite { sprite, x, y, w, h, region } => {
                    json!({"type": "sprite", "sprite": sprite, "x": x, "y": y, "w": w, "h": h, "region": region})
                }
                GuiDraw::Text { text, x, y, color, shadow } => {
                    json!({"type": "text", "text": text, "x": x, "y": y, "color": *color as i32, "shadow": shadow})
                }
                GuiDraw::Item { slot, x, y, seed, scale, pivot } => {
                    // translate(pivot) * scale * translate(-pivot) as a 3x2 matrix.
                    let m = [scale[0], 0.0, 0.0, scale[1], pivot[0] - pivot[0] * scale[0], pivot[1] - pivot[1] * scale[1]];
                    json!({"type": "item", "slot": slot, "x": x, "y": y, "seed": seed, "matrix": m})
                }
            })
            .collect();
        writeln!(out, "{}", json!({ "draws": draws }))?;
    }
    Ok(())
}
