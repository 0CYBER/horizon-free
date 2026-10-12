//! The overlay case kinds of the HUD parity fixture.
use horizon_client::{
    brewing, component,
    hud::HudAtlas,
    hud_layout::{self, GuiDraw},
    hud_overlays::{self, EffectIcon, HeldName, Titles, ToolHighlight},
    text::{self, Text},
};
use serde_json::Value;

fn int(c: &Value, k: &str, d: i64) -> i32 {
    c[k].as_i64().unwrap_or(d) as i32
}
fn float(c: &Value, k: &str, d: f64) -> f32 {
    c[k].as_f64().unwrap_or(d) as f32
}
/// A text component in its JSON form.
fn comp(v: &Value) -> Text {
    component::to_text(v, &mut |s| Text::literal(s))
}
/// `Options.getBackgroundColor(0.0F)` from the case's text background options.
fn background(c: &Value) -> u32 {
    hud_layout::background_color(
        c["chatOnly"].as_bool().unwrap_or(true),
        c["textOpacity"].as_f64().unwrap_or(0.5),
        0.0,
    )
}

pub fn run(kind: &str, c: &Value, atlas: &HudAtlas) -> Vec<GuiDraw> {
    let (w, h) = (int(c, "width", 0), int(c, "height", 0));
    match kind {
        "effects" => effects(c, w),
        "title" => title(c, w, h, atlas),
        "selected" => selected(c, w, h, atlas),
        _ => Vec::new(),
    }
}

fn effects(c: &Value, w: i32) -> Vec<GuiDraw> {
    let icons: Vec<EffectIcon> = c["active"]
        .as_array()
        .into_iter()
        .flatten()
        .map(|e| {
            let id = e["id"]
                .as_str()
                .unwrap_or("")
                .trim_start_matches("minecraft:");
            let def = brewing::effect(id).expect("effect");
            EffectIcon {
                id: id.to_owned(),
                beneficial: def.category == brewing::Category::Beneficial,
                color: def.color as i32,
                ambient: e["ambient"].as_bool().unwrap_or(false),
                duration: e["duration"].as_i64().unwrap_or(0) as i32,
                show_icon: e["icon"].as_bool().unwrap_or(true),
            }
        })
        .collect();
    hud_overlays::effects(w, c["demo"].as_bool().unwrap_or(false), &icons)
}

/// Replays `Hud.setTitle` / `setSubtitle` / `setTimes` / `clearTitles` /
/// `resetTitleTimes` / `setOverlayMessage` and ticks, then extracts the
/// action bar and the title.
fn title(c: &Value, w: i32, h: i32, atlas: &HudAtlas) -> Vec<GuiDraw> {
    let mut t = Titles::default();
    for op in c["ops"].as_array().into_iter().flatten() {
        let n = |i: usize| op[i].as_i64().unwrap_or(0) as i32;
        match op[0].as_str().unwrap_or("") {
            "title" => t.set_title(comp(&op[1])),
            "subtitle" => t.set_subtitle(comp(&op[1])),
            "times" => t.set_times(n(1), n(2), n(3)),
            "clear" => t.clear(),
            "reset" => t.reset_times(),
            "overlay" => t.set_overlay(comp(&op[1]), op[2].as_bool().unwrap_or(false)),
            "tick" => {
                for _ in 0..n(1) {
                    t.tick();
                }
            }
            other => panic!("unknown title op {other}"),
        }
    }
    let partial = float(c, "partial", 0.0);
    let bg = background(c);
    let mut out = t.overlay_message(w, h, partial, &atlas.font, bg);
    out.extend(t.title(w, h, partial, &atlas.font, bg));
    out
}

/// Ticks `toolHighlightTimer` through the selected stacks, then extracts the name.
fn selected(c: &Value, w: i32, h: i32, atlas: &HudAtlas) -> Vec<GuiDraw> {
    let mut highlight = ToolHighlight::default();
    let time = c["displayTime"].as_f64().unwrap_or(1.0);
    for step in c["steps"].as_array().into_iter().flatten() {
        let held = (!step.is_null()).then(|| {
            let custom = &step["custom"];
            HeldName {
                item: step["item"].as_str().unwrap_or("").to_owned(),
                hover_name: if custom.is_null() {
                    Text::translate(step["key"].as_str().unwrap_or(""), &[])
                } else {
                    comp(custom)
                },
                rarity_color: text::named_color(step["rarity"].as_str().unwrap_or("white"))
                    .unwrap_or(0xFFFFFF),
                custom_name: !custom.is_null(),
            }
        });
        highlight.tick(held, time);
    }
    let mode = c["gameMode"].as_str().unwrap_or("survival");
    let can_hurt = mode == "survival" || mode == "adventure";
    highlight.draw(w, h, can_hurt, &atlas.font, background(c))
}
