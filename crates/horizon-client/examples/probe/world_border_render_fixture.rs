//! Headless transport for the world border parity gate: replays each case's
//! border changes, ticks and frames through `horizon_client::worldborder`,
//! `border_render` and `hud_overlays::vignette`, and writes what the
//! vanilla renderer computes (bounds, fade, wall buffer, transforms, walls
//! drawn, vignette colour) with floats as raw bits.
#![forbid(unsafe_code)]
use horizon_client::{
    border_render::{self, BorderRenderState, BorderRenderer},
    hud_layout::GuiDraw,
    hud_overlays,
    worldborder::{BorderStatus, WorldBorder},
};
use serde_json::{Value, json};
use std::{
    fs,
    io::{BufWriter, Write},
};
type Result<T> = std::result::Result<T, Box<dyn std::error::Error>>;

fn floats(v: &[f32]) -> Value {
    v.iter().map(|f| f.to_bits() as i32).collect()
}
fn doubles(v: &[f64]) -> Value {
    v.iter().map(|f| f.to_bits() as i64).collect()
}

pub fn main() -> Result<()> {
    let args: Vec<String> = crate::args().skip(1).collect();
    let [inputs, output] = args.as_slice() else {
        return Err("usage: world_border_render_fixture INPUTS OUTPUT".into());
    };
    let mut out = BufWriter::new(fs::File::create(output)?);
    for line in fs::read_to_string(inputs)?
        .lines()
        .filter(|l| !l.is_empty())
    {
        let c: Value = serde_json::from_str(line)?;
        let f = |v: &Value| v.as_f64().unwrap_or(0.0);
        let mut border = WorldBorder::default();
        border.warning_distance = c["warningBlocks"].as_i64().unwrap_or(0) as i32;
        border.warning_time = c["warningTime"].as_i64().unwrap_or(0) as i32;
        border.set_absolute_max_size(c["absoluteMaxSize"].as_i64().unwrap_or(0) as i32);
        border.set_center([f(&c["centerX"]), f(&c["centerZ"])]);
        border.set_size(f(&c["size"]));
        let mut renderer = BorderRenderer::default();
        let mut state = BorderRenderState::default();
        let mut frames = Vec::new();
        for op in c["ops"].as_array().into_iter().flatten() {
            match op["op"].as_str().unwrap_or("") {
                "tick" => (0..op["n"].as_i64().unwrap_or(0)).for_each(|_| border.tick()),
                "lerp" => border.lerp_between(
                    f(&op["from"]),
                    f(&op["to"]),
                    op["ticks"].as_i64().unwrap_or(0),
                ),
                "size" => border.set_size(f(&op["size"])),
                "center" => border.set_center([f(&op["x"]), f(&op["z"])]),
                "warning" => {
                    border.warning_distance = op["blocks"].as_i64().unwrap_or(0) as i32;
                    border.warning_time = op["time"].as_i64().unwrap_or(0) as i32;
                }
                "invalidate" => renderer.invalidate(),
                "frame" => {
                    let partial = f(&op["partial"]) as f32;
                    let p = &op["camera"];
                    let camera = [f(&p[0]), f(&p[1]), f(&p[2])];
                    let distance = f(&op["renderDistance"]);
                    let millis = op["millis"].as_i64().unwrap_or(0);
                    let status = match border.status() {
                        BorderStatus::Growing => "GROWING",
                        BorderStatus::Shrinking => "SHRINKING",
                        BorderStatus::Stationary => "STATIONARY",
                    };
                    let mut frame = json!({
                        "bounds": doubles(&border.bounds_at(partial)),
                        "sizes": doubles(&[border.size(), border.target_size(), border.lerp_speed(), border.distance(camera[0], camera[2])]),
                        "lerpTime": border.remaining_ticks(),
                        "status": status,
                    });
                    border_render::extract(&border, partial, camera, distance, &mut state);
                    frame["alpha"] = doubles(&[state.alpha]);
                    frame["tint"] = json!(state.tint as i32);
                    let drawn =
                        renderer.prepare(&state, camera, distance, f(&op["depthFar"]) as f32);
                    frame["drawn"] = json!(drawn);
                    frame["rebuilt"] = json!(renderer.take_rebuilt());
                    if drawn {
                        frame["vertices"] = floats(&renderer.vertices.concat());
                        let t = renderer.transforms(&state, camera, millis);
                        frame["color"] = floats(&t.color);
                        frame["offset"] = floats(&t.model_offset);
                        frame["texture"] = floats(&[t.texture_offset, t.texture_offset]);
                        frame["sides"] = json!(BorderRenderer::sides(&state, camera, distance));
                    }
                    let warning = border.warning_strength(camera[0], camera[2]);
                    let vignette =
                        hud_overlays::vignette(427, 240, f(&op["vignette"]) as f32, warning);
                    let color = match vignette {
                        GuiDraw::Blit { color, .. } => color,
                        _ => 0,
                    };
                    frame["vignetteColor"] = json!(color as i32);
                    frames.push(frame);
                }
                other => return Err(format!("unknown op {other}").into()),
            }
        }
        writeln!(out, "{}", json!({ "frames": frames }))?;
    }
    Ok(())
}
