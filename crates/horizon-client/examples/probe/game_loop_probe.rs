#![forbid(unsafe_code)]
//! Runs the JSON-lines scenarios of tools/test_game_loop_parity.py through the
//! Rust timer, tick-rate manager, window, mouse and key-mapping code.
use horizon_client::{
    Result,
    clock::{ServerTickRateManager, Timer},
    keymapping::KeyState,
    mouse::{LookOptions, MouseLook, Rotation},
    options_file::{OptionsFile, V},
    window::{self, FrameContext, InactivityFpsLimit},
};
use serde_json::Value;
use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
};

fn fl(v: &Value) -> f32 {
    f32::from_bits(u32::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn db(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn fb(v: f32) -> String {
    format!("{:08x}", if v.is_nan() { 0x7fc0_0000 } else { v.to_bits() })
}
fn dbs(v: f64) -> String {
    format!(
        "{:016x}",
        if v.is_nan() {
            0x7ff8_0000_0000_0000
        } else {
            v.to_bits()
        }
    )
}
fn b(v: &Value) -> bool {
    v.as_bool().unwrap()
}

fn timer(c: &Value, out: &mut impl Write) -> Result<()> {
    let mut timer = Timer::new(fl(&c["tps"]), c["start"].as_i64().unwrap());
    writeln!(out, "timer")?;
    for op in c["ops"].as_array().unwrap() {
        let name = op[0].as_str().unwrap();
        let mut result = String::new();
        match name {
            "adv" => {
                let mspt = if op[2].is_null() { 0.0 } else { fl(&op[2]) };
                let ticks = timer.advance_game_time(op[1].as_i64().unwrap(), |d| {
                    if mspt == 0.0 { d } else { java_max(d, mspt) }
                });
                result = format!(" {ticks}");
            }
            "real" => timer.advance_real_time(op[1].as_i64().unwrap()),
            "pause" => timer.update_pause_state(b(&op[1])),
            "frozen" => timer.update_frozen_state(b(&op[1])),
            _ => panic!("{name}"),
        }
        writeln!(
            out,
            "{name}{result} {} {} {} {}",
            fb(timer.game_time_delta_ticks()),
            fb(timer.game_time_delta_partial_tick(false)),
            fb(timer.game_time_delta_partial_tick(true)),
            fb(timer.realtime_delta_ticks())
        )?;
    }
    Ok(())
}

/// Math.max(float, float).
fn java_max(a: f32, b: f32) -> f32 {
    if a.is_nan() || b.is_nan() {
        f32::NAN
    } else if a == 0.0 && b == 0.0 {
        f32::from_bits(a.to_bits() & b.to_bits())
    } else if a >= b {
        a
    } else {
        b
    }
}

fn rate(c: &Value, out: &mut impl Write) -> Result<()> {
    let mut m = ServerTickRateManager::default();
    writeln!(out, "rate")?;
    for op in c["ops"].as_array().unwrap() {
        let name = op[0].as_str().unwrap();
        let mut result = String::new();
        let mut report = None;
        match name {
            "rate" => m.set_tick_rate(fl(&op[1])),
            "freeze" => m.base.set_frozen(b(&op[1])),
            "step" => {
                result = format!(" {}", m.step_game_if_paused(op[1].as_i64().unwrap() as i32))
            }
            "stopstep" => result = format!(" {}", m.stop_stepping()),
            "sprint" => {
                result = format!(
                    " {}",
                    m.request_game_to_sprint(op[1].as_i64().unwrap() as i32)
                )
            }
            "check" => {
                let (run, r) = m.check_should_sprint_this_tick(op[1].as_i64().unwrap());
                result = format!(" {run}");
                report = r;
            }
            "end" => m.end_tick_work(op[1].as_i64().unwrap()),
            "stopsprint" => {
                report = m.stop_sprinting();
                result = format!(" {}", report.is_some());
            }
            "tick" => m.base.tick(),
            _ => panic!("{name}"),
        }
        let tail = report.map_or(String::new(), |r| {
            format!(" report {} {}", r.ticks_per_second, r.milliseconds_per_tick)
        });
        writeln!(
            out,
            "{name}{result} | {} {} {} {} {} {} {} {}{tail}",
            fb(m.base.tickrate()),
            m.base.nanoseconds_per_tick(),
            fb(m.base.milliseconds_per_tick()),
            m.base.runs_normally(),
            m.base.is_stepping_forward(),
            m.base.frozen_ticks_to_run(),
            m.base.is_frozen(),
            m.is_sprinting()
        )?;
    }
    Ok(())
}

fn scale(c: &Value, out: &mut impl Write) -> Result<()> {
    let i = |k: &str| c[k].as_i64().unwrap() as i32;
    let fbsize = [i("w"), i("h")];
    let scale = window::calculate_scale(fbsize, i("max"), b(&c["uni"]));
    let [gw, gh] = window::gui_scaled_size(fbsize, scale);
    writeln!(
        out,
        "scale {scale} {gw} {gh} {} {}",
        dbs(window::scaled_pos(db(&c["x"]), gw, i("sw"))),
        dbs(window::scaled_pos(db(&c["y"]), gh, i("sh")))
    )?;
    Ok(())
}

fn limit(c: &Value, out: &mut impl Write) -> Result<()> {
    let ctx = FrameContext {
        inactivity: if b(&c["afk"]) {
            InactivityFpsLimit::Afk
        } else {
            InactivityFpsLimit::Minimized
        },
        iconified: b(&c["iconified"]),
        focused: b(&c["focused"]),
        exclusive_fullscreen: false,
        now_ms: c["ago"].as_i64().unwrap(),
        latest_input_ms: 0,
        has_level: b(&c["level"]),
        has_screen: b(&c["screen"]),
        has_overlay: b(&c["overlay"]),
    };
    let reason = window::throttle_reason(ctx);
    let name = match reason {
        window::ThrottleReason::None => "NONE",
        window::ThrottleReason::WindowIconified => "WINDOW_ICONIFIED",
        window::ThrottleReason::LongAfk => "LONG_AFK",
        window::ThrottleReason::ShortAfk => "SHORT_AFK",
        window::ThrottleReason::OutOfLevelMenu => "OUT_OF_LEVEL_MENU",
    };
    writeln!(
        out,
        "limit {name} {} {}",
        window::framerate_limit(c["opt"].as_i64().unwrap() as i32, ctx),
        window::heavily_throttled(reason)
    )?;
    Ok(())
}

fn mouse(c: &Value, out: &mut impl Write) -> Result<()> {
    let look = LookOptions {
        sensitivity: db(&c["sens"]),
        smooth_camera: b(&c["smooth"]),
        scoping: b(&c["scoping"]),
        invert_x: b(&c["ix"]),
        invert_y: b(&c["iy"]),
    };
    let (active, grabbed) = (b(&c["active"]), b(&c["grabbed"]));
    let mut handler = MouseLook::default();
    let mut rotation = Rotation {
        x_rot: 0.0,
        y_rot: 0.0,
        x_rot_o: 0.0,
        y_rot_o: 0.0,
    };
    writeln!(out, "mouse")?;
    for op in c["ops"].as_array().unwrap() {
        match op[0].as_str().unwrap() {
            "move" => handler.on_move(
                db(&op[1]),
                db(&op[2]),
                db(&op[3]),
                db(&op[4]),
                active,
                grabbed,
            ),
            "ignore" => handler.set_ignore_first_move(),
            "frame" => {
                if let Some((xo, yo)) =
                    handler.handle_accumulated_movement(db(&op[1]), active, grabbed, look)
                {
                    rotation.turn(xo, yo);
                }
                writeln!(
                    out,
                    "frame {} {} {} {}",
                    fb(rotation.x_rot),
                    fb(rotation.y_rot),
                    fb(rotation.x_rot_o),
                    fb(rotation.y_rot_o)
                )?;
            }
            other => panic!("{other}"),
        }
    }
    Ok(())
}

fn scroll(c: &Value, out: &mut impl Write) -> Result<()> {
    let mut handler = MouseLook::default();
    let mut slot = c["slot"].as_i64().unwrap() as i32;
    let spectator = c["spectator"].as_bool().unwrap_or(false);
    let mut speed = 0.05;
    // OptionInstance.set quantises the wheel sensitivity.
    let mut file = OptionsFile::default();
    file.set("mouseWheelSensitivity", V::Double(db(&c["sens"])));
    let sensitivity = file.double("mouseWheelSensitivity");
    writeln!(out, "scroll")?;
    for op in c["ops"].as_array().unwrap() {
        if let Some((wheel, (_, y))) =
            handler.on_scroll(db(&op[0]), db(&op[1]), b(&c["discrete"]), sensitivity)
        {
            if spectator {
                speed = horizon_client::mouse::spectator_flying_speed(speed, y);
            } else {
                slot = horizon_client::mouse::ScrollWheelHandler::next_selection(
                    f64::from(wheel),
                    slot,
                    9,
                );
            }
        }
        if spectator {
            writeln!(out, "{slot} {}", fb(speed))?;
        } else {
            writeln!(out, "{slot}")?;
        }
    }
    Ok(())
}

fn key(c: &Value, out: &mut impl Write) -> Result<()> {
    let mut state = if b(&c["toggle"]) {
        KeyState::toggle(b(&c["restore"]), b(&c["keyboard"]))
    } else {
        KeyState::plain()
    };
    let mut option = false;
    writeln!(out, "key")?;
    for op in c["ops"].as_array().unwrap() {
        let name = op[0].as_str().unwrap();
        let mut result = String::new();
        match name {
            "option" => option = b(&op[1]),
            "down" => state.set_down(b(&op[1]), option),
            "release" => state.release(option),
            "restore" => {
                result = format!(" {}", state.should_restore_state_on_screen_closed(option))
            }
            "click" => state.click(),
            "consume" => result = format!(" {}", state.consume_click()),
            "ingame" => result = format!(" {}", state.should_set_on_ingame_focus(option)),
            _ => panic!("{name}"),
        }
        writeln!(out, "{name}{result} {}", state.is_down)?;
    }
    Ok(())
}

pub fn main() -> Result<()> {
    let args: Vec<_> = crate::args().skip(1).collect();
    let mut out = BufWriter::new(File::create(&args[1])?);
    for line in BufReader::new(File::open(&args[0])?).lines() {
        let c: Value = serde_json::from_str(&line?)?;
        match c["t"].as_str().unwrap() {
            "timer" => timer(&c, &mut out)?,
            "rate" => rate(&c, &mut out)?,
            "scale" => scale(&c, &mut out)?,
            "limit" => limit(&c, &mut out)?,
            "mouse" => mouse(&c, &mut out)?,
            "scroll" => scroll(&c, &mut out)?,
            "key" => key(&c, &mut out)?,
            other => panic!("{other}"),
        }
    }
    Ok(())
}
