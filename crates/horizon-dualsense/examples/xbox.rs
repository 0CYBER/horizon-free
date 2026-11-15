//! Plays the four motors on a real Xbox One pad, so a change to the reports can be checked without
//! the game:
//!
//!     cargo run -p horizon-dualsense --example xbox -- [handles|triggers|link|all]
//!
//! On Windows a pad on a cable goes through Windows.Gaming.Input, whose four motors are the only
//! way to its triggers there; a Bluetooth pad uses its HID report, as on Linux and macOS.
use horizon_dualsense::xbox::{
    Motors, Target,
    device::{Link, Xbox},
};
use std::time::Duration;

fn main() {
    let what = std::env::args().nth(1).unwrap_or_else(|| "all".into());
    if what == "link" {
        link();
        return;
    }
    if what == "all" {
        link();
    }
    let api = hidapi::HidApi::new().expect("hidapi");
    let mut pad = match Xbox::open(&api, Target::ANY) {
        Some(Ok(pad)) => pad,
        Some(Err(e)) => panic!("the Xbox pad is there but can't be opened: {e}"),
        None => panic!("no Xbox One pad found that takes output on this system"),
    };
    println!("Xbox One pad over {:?}", pad.transport());
    let handles = [
        ("left handle (heavy) 50%", 128, 0),
        ("left handle (heavy) 100%", 255, 0),
        ("right handle (light) 50%", 0, 128),
        ("right handle (light) 100%", 0, 255),
        ("both handles 100%", 255, 255),
    ];
    let triggers = [
        ("left trigger 50%", 128, 0),
        ("left trigger 100%", 255, 0),
        ("right trigger 50%", 0, 128),
        ("right trigger 100%", 0, 255),
        ("both triggers 100%", 255, 255),
    ];
    let groups = [
        ("handles", &handles[..], false),
        ("triggers", &triggers[..], true),
    ];
    for (group, steps, triggers_on) in groups {
        if what != "all" && what != group {
            continue;
        }
        println!("  {group}:");
        for &(name, left, right) in steps {
            println!("    {name}");
            let motors = if triggers_on {
                Motors {
                    left_trigger: left,
                    right_trigger: right,
                    ..Motors::STOP
                }
            } else {
                Motors {
                    strong: left,
                    weak: right,
                    ..Motors::STOP
                }
            };
            pad.write(motors).expect("write");
            std::thread::sleep(Duration::from_millis(700));
        }
    }
    pad.write(Motors::STOP).expect("stop");
    println!("  stopped");
}

/// The writer thread: a two-step vibration, and the stop when it drops.
fn link() {
    println!("link:");
    let link = Link::start(Target::ANY);
    let start = std::time::Instant::now();
    while link.open().is_none() && start.elapsed() < Duration::from_secs(3) {
        std::thread::sleep(Duration::from_millis(20));
    }
    println!("  open over {:?} after {:?}", link.open(), start.elapsed());
    link.rumble(vec![
        (
            Motors::levels(0.6, 0.0, [0.0, 0.0]),
            Duration::from_millis(150),
        ),
        (
            Motors::levels(0.0, 0.0, [0.9, 0.9]),
            Duration::from_millis(250),
        ),
    ]);
    std::thread::sleep(Duration::from_millis(600));
    drop(link);
    println!("  dropped: motors stopped");
}
