use horizon_world::item_use_lifecycle::{self, UseLifecycleWorld, UseState};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
struct World<'a> {
    f: &'a Value,
    out: &'a mut dyn Write,
    calls: i64,
    changed: bool,
    using: bool,
    hands: [usize; 2],
}
impl World<'_> {
    fn state(&self) -> &Value {
        &self.f[if self.changed { "after" } else { "initial" }]
    }
    fn reset(&mut self, s: &mut UseState<usize>) {
        s.item = self.state()["item"].as_u64().unwrap() as usize;
        s.remaining = self.state()["remaining"].as_i64().unwrap() as i32;
        s.has_recent_kinetic_enemies = self.state()["kinetic"].as_bool().unwrap();
        self.using = self.state()["using"].as_bool().unwrap();
        self.hands = std::array::from_fn(|i| self.state()["hands"][i].as_u64().unwrap() as usize);
    }
    fn event(&mut self, s: &mut UseState<usize>, name: &str) -> io::Result<()> {
        writeln!(self.out, "{name}")?;
        let n = self.calls;
        self.calls += 1;
        if n == self.f["fail"].as_i64().unwrap() {
            return Err(io::Error::other("injected lifecycle failure"));
        }
        if n == self.f["mutate_at"].as_i64().unwrap() {
            self.changed = true;
            self.reset(s);
        }
        Ok(())
    }
    fn kind(&self, item: usize) -> usize {
        if item == 1 && self.f["shared_type"].as_bool().unwrap() {
            0
        } else {
            item
        }
    }
}
impl UseLifecycleWorld for World<'_> {
    type Error = io::Error;
    type Item = usize;
    type Hand = usize;
    type Level = usize;
    fn empty_item(&self) -> usize {
        2
    }
    fn is_empty(&self, item: usize) -> bool {
        item == 2
    }
    fn same_item_type(&self, a: usize, b: usize) -> bool {
        self.kind(a) == self.kind(b)
    }
    fn using_item(&mut self, s: &mut UseState<usize>) -> io::Result<bool> {
        self.event(s, "using")?;
        Ok(self.using)
    }
    fn used_hand(&mut self, s: &mut UseState<usize>) -> io::Result<usize> {
        self.event(s, "hand")?;
        Ok(self.state()["hand"].as_u64().unwrap() as usize)
    }
    fn hand_item(&mut self, s: &mut UseState<usize>, hand: usize) -> io::Result<usize> {
        self.event(s, &format!("get-hand {hand}"))?;
        Ok(self.hands[hand])
    }
    fn set_hand_item(
        &mut self,
        s: &mut UseState<usize>,
        hand: usize,
        item: usize,
    ) -> io::Result<()> {
        self.event(s, &format!("set-hand {hand} {item}"))?;
        self.hands[hand] = item;
        Ok(())
    }
    fn level(&mut self, s: &mut UseState<usize>) -> io::Result<usize> {
        self.event(s, "level")?;
        Ok(self.state()["level"].as_u64().unwrap() as usize)
    }
    fn is_client(&mut self, s: &mut UseState<usize>, level: usize) -> io::Result<bool> {
        self.event(s, &format!("client {level}"))?;
        Ok(self.state()["clients"][level].as_bool().unwrap())
    }
    fn remaining_ticks(&mut self, s: &mut UseState<usize>) -> io::Result<i32> {
        self.event(s, "remaining")?;
        Ok(self.state()["virtual_remaining"].as_i64().unwrap() as i32)
    }
    fn item_tick(
        &mut self,
        s: &mut UseState<usize>,
        item: usize,
        level: usize,
        remaining: i32,
    ) -> io::Result<()> {
        let kind = self.kind(item);
        if kind == 2 {
            return Ok(());
        }
        self.event(s, &format!("tick {kind} {item} {level} {remaining}"))
    }
    fn use_on_release(&mut self, s: &mut UseState<usize>, item: usize) -> io::Result<bool> {
        let kind = self.kind(item);
        if kind == 2 {
            return Ok(false);
        }
        self.event(s, &format!("release-query {kind} {item}"))?;
        Ok(self.state()["releases"][kind].as_bool().unwrap())
    }
    fn release_item(
        &mut self,
        s: &mut UseState<usize>,
        item: usize,
        level: usize,
        remaining: i32,
    ) -> io::Result<()> {
        let kind = self.kind(item);
        if kind == 2 {
            return Ok(());
        }
        self.event(s, &format!("release {kind} {item} {level} {remaining}"))
    }
    fn finish_item(
        &mut self,
        s: &mut UseState<usize>,
        item: usize,
        level: usize,
    ) -> io::Result<usize> {
        let kind = self.kind(item);
        if kind == 2 {
            return Ok(item);
        }
        self.event(s, &format!("finish {kind} {item} {level}"))?;
        Ok(self.state()["finishes"][kind].as_u64().unwrap() as usize)
    }
    fn set_using_flag(&mut self, s: &mut UseState<usize>, using: bool) -> io::Result<()> {
        self.event(s, &format!("flag {using}"))?;
        self.using = using;
        Ok(())
    }
    fn finish_vibration(&mut self, s: &mut UseState<usize>, item: usize) -> io::Result<()> {
        if item < 2 && self.f["vibrations"][item].as_bool().unwrap() {
            self.event(s, "vibration")?;
        }
        Ok(())
    }
}
fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = std::env::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let mut s = UseState {
            item: 2,
            remaining: 0,
            has_recent_kinetic_enemies: false,
        };
        let mut w = World {
            f: &f,
            out: &mut out,
            calls: 0,
            changed: false,
            using: false,
            hands: [2, 2],
        };
        w.reset(&mut s);
        let result = match f["mode"].as_str().unwrap() {
            "tick" => item_use_lifecycle::tick(&mut w, &mut s),
            "update" => {
                item_use_lifecycle::update(&mut w, &mut s, f["argument"].as_u64().unwrap() as usize)
            }
            "complete" => item_use_lifecycle::complete(&mut w, &mut s),
            "release" => item_use_lifecycle::release(&mut w, &mut s),
            "stop" => item_use_lifecycle::stop(&mut w, &mut s),
            _ => panic!("mode"),
        };
        writeln!(
            w.out,
            "result {}",
            if result.is_ok() { "ok" } else { "error" }
        )?;
        writeln!(
            w.out,
            "state {} {} {} {} {} {} {} {}",
            s.item,
            s.remaining,
            s.has_recent_kinetic_enemies,
            w.using,
            w.hands[0],
            w.hands[1],
            w.calls,
            w.changed
        )?;
    }
    Ok(())
}
