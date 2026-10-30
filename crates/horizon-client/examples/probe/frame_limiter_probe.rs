#![forbid(unsafe_code)]
//! Runs the JSON-lines scenarios of tools/test_frame_limiter_parity.py through
//! the Rust `FramerateLimiter` with the same scripted virtual clock.
use horizon_client::{
    Result,
    window::{FramerateLimiter, LimiterClock, end_frame},
};
use serde_json::Value;
use std::{
    fs::File,
    io::{BufRead, BufReader, BufWriter, Write},
};

/// The scripted clock of the case: every query costs `call` ns, parking
/// sleeps the request plus the next overshoot (at least 1 ns), spinning
/// costs `spin` ns and interruptions follow their script.
struct Script {
    now: i64,
    call: i64,
    spin: i64,
    overshoot: Vec<i64>,
    next_overshoot: usize,
    interrupts: Vec<bool>,
    next_interrupt: usize,
    parks: Vec<i64>,
    spins: u64,
    queries: u64,
}
impl LimiterClock for Script {
    fn nanos(&mut self) -> i64 {
        self.queries += 1;
        let t = self.now;
        self.now = self.now.wrapping_add(self.call);
        t
    }
    fn park(&mut self, nanos: i64) {
        self.parks.push(nanos);
        let o = self.overshoot[self.next_overshoot % self.overshoot.len()];
        self.next_overshoot += 1;
        self.now = self.now.wrapping_add(nanos.max(0).wrapping_add(o).max(1));
    }
    fn spin(&mut self) {
        self.spins += 1;
        self.now = self.now.wrapping_add(self.spin);
    }
    fn interrupted(&mut self) -> bool {
        let i = self.interrupts[self.next_interrupt % self.interrupts.len()];
        self.next_interrupt += 1;
        i
    }
}

pub fn main() -> Result<()> {
    let args: Vec<String> = crate::args().collect();
    let input = BufReader::new(File::open(&args[1])?);
    let mut out = BufWriter::new(File::create(&args[2])?);
    for (n, line) in input.lines().enumerate() {
        let c: Value = serde_json::from_str(&line?)?;
        writeln!(out, "== {n}")?;
        let i = |k: &str| c[k].as_i64().unwrap();
        let mut clock = Script {
            now: i("now"),
            call: i("call"),
            spin: i("spin"),
            overshoot: c["overshoot"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_i64().unwrap())
                .collect(),
            next_overshoot: 0,
            interrupts: c["interrupts"]
                .as_array()
                .unwrap()
                .iter()
                .map(|v| v.as_bool().unwrap())
                .collect(),
            next_interrupt: 0,
            parks: Vec::new(),
            spins: 0,
            queries: 0,
        };
        let mut limiter =
            FramerateLimiter::with_state(i("last"), i("average"), i("last_limit") as i32);
        for frame in c["frames"].as_array().unwrap() {
            clock.now = clock.now.wrapping_add(frame[0].as_i64().unwrap());
            clock.parks.clear();
            clock.spins = 0;
            clock.queries = 0;
            end_frame(&mut limiter, frame[1].as_i64().unwrap() as i32, &mut clock);
            let parks: Vec<String> = clock.parks.iter().map(i64::to_string).collect();
            writeln!(
                out,
                "frame {} {} {} {} queries {} spins {} parks [{}]",
                clock.now,
                limiter.last_frame_time(),
                limiter.average_overshoot_ns(),
                limiter.last_framerate_limit(),
                clock.queries,
                clock.spins,
                parks.join(",")
            )?;
        }
    }
    out.flush()?;
    Ok(())
}
