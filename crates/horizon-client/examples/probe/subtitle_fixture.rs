//! The subtitle overlay for differential tests against vanilla's
//! `SubtitleOverlay` (tools/test_subtitle_parity.py): replays each case's
//! sound plays, clock changes and frames through
//! `horizon_client::subtitles` and prints every frame's fills and texts with
//! their pose, in VanillaSubtitleOracle's JSON lines.
//!
//! usage: subtitle_fixture RESOURCES INPUTS OUTPUT
use horizon_client::{
    hud::HudAtlas,
    hud_layout::{GuiDraw, IDENTITY, background_color, pose_apply},
    hud_overlays::{SubtitleFrame, SubtitleOverlay},
    lang,
};
use serde_json::{Value, json};
use std::{fs, io::Write, path::PathBuf};

fn vec3(v: &Value) -> [f64; 3] {
    std::array::from_fn(|i| v[i].as_f64().unwrap_or(0.0))
}
fn pose(m: [f32; 6]) -> Value {
    json!(m)
}

pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<PathBuf> = crate::args_os().skip(1).map(PathBuf::from).collect();
    let [root, inputs, output] = args.as_slice() else {
        return Err("usage: subtitle_fixture RESOURCES INPUTS OUTPUT".into());
    };
    lang::set(lang::Language::load(&[root], lang::DEFAULT));
    let atlas = HudAtlas::build(root)?;
    let mut out = std::io::BufWriter::new(fs::File::create(output)?);
    for line in fs::read_to_string(inputs)?
        .lines()
        .filter(|l| !l.is_empty())
    {
        let c: Value = serde_json::from_str(line)?;
        let mut overlay = SubtitleOverlay::default();
        let mut now = 0i64;
        let mut frames = Vec::new();
        for op in c["ops"].as_array().into_iter().flatten() {
            match op["op"].as_str().unwrap_or_default() {
                // Util.getMillis: nanoseconds / 1_000_000.
                "time" => now = op["ms"].as_i64().unwrap_or(0),
                "play" => {
                    if overlay.listening() {
                        let range = op["range"].as_f64().map_or(f32::INFINITY, |r| r as f32);
                        overlay.on_play(
                            op["key"].as_str(),
                            vec3(&op["at"]),
                            range,
                            op["looping"].as_bool().unwrap_or(false),
                            now,
                        );
                    }
                }
                "frame" => {
                    let frame = SubtitleFrame {
                        show: op["show"].as_bool().unwrap_or(false),
                        position: vec3(&op["pos"]),
                        forward: vec3(&op["forward"]),
                        up: vec3(&op["up"]),
                        multiplier: op["mult"].as_f64().unwrap_or(1.0),
                        background: background_color(
                            op["chat_only"].as_bool().unwrap_or(true),
                            op["opacity"].as_f64().unwrap_or(0.5),
                            0.8,
                        ),
                        width: op["w"].as_i64().unwrap_or(0) as i32,
                        height: op["h"].as_i64().unwrap_or(0) as i32,
                    };
                    let mut stack = vec![IDENTITY];
                    let mut draws = Vec::new();
                    let calls = overlay.extract(&frame, &atlas.font, now);
                    if !calls.is_empty() {
                        draws.push(json!({"type": "stratum"}));
                    }
                    for d in calls {
                        let m = *stack.last().unwrap_or(&IDENTITY);
                        match d {
                            GuiDraw::Push(ops) => {
                                let mut top = m;
                                for op in ops {
                                    pose_apply(&mut top, op);
                                }
                                stack.push(top);
                            }
                            GuiDraw::Pop => {
                                stack.pop();
                            }
                            GuiDraw::Fill {
                                x0,
                                y0,
                                x1,
                                y1,
                                color,
                            } => draws.push(json!({
                                "type": "fill", "x0": x0, "y0": y0, "x1": x1, "y1": y1,
                                "color": format!("{color:x}"), "pose": pose(m),
                            })),
                            GuiDraw::Text {
                                text,
                                x,
                                y,
                                color,
                                shadow,
                            } => draws.push(json!({
                                "type": "text", "text": text.plain(), "x": x, "y": y,
                                "color": format!("{color:x}"), "shadow": shadow, "pose": pose(m),
                            })),
                            other => return Err(format!("unexpected draw {other:?}").into()),
                        }
                    }
                    frames.push(Value::Array(draws));
                }
                other => return Err(format!("op {other}").into()),
            }
        }
        writeln!(out, "{}", json!({"case": c["case"], "frames": frames}))?;
    }
    Ok(())
}
