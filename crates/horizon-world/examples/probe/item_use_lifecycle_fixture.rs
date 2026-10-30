use horizon_world::{
    component_patch::Prototype,
    item_stack_refs::{ItemStacks, StackRef},
    item_use_lifecycle::{self, UseLifecycleWorld, UseState},
};
type Stack = StackRef<usize, u8, i32>;
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
    items: [Stack; 3],
}
impl World<'_> {
    fn state(&self) -> &Value {
        &self.f[if self.changed { "after" } else { "initial" }]
    }
    fn reset(&mut self, s: &mut UseState<Stack>) {
        s.item = self.items[self.state()["item"].as_u64().unwrap() as usize].clone();
        s.remaining = self.state()["remaining"].as_i64().unwrap() as i32;
        s.has_recent_kinetic_enemies = self.state()["kinetic"].as_bool().unwrap();
        self.using = self.state()["using"].as_bool().unwrap();
        self.hands = std::array::from_fn(|i| self.state()["hands"][i].as_u64().unwrap() as usize);
    }
    fn event(&mut self, s: &mut UseState<Stack>, name: &str) -> io::Result<()> {
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
    fn id(&self, item: &Stack) -> usize {
        self.items
            .iter()
            .position(|s| s.same_reference(item))
            .unwrap()
    }
    fn kind(&self, item: &Stack) -> usize {
        item.read().item()
    }
}
impl UseLifecycleWorld for World<'_> {
    type Error = io::Error;
    type Item = Stack;
    type Hand = usize;
    type Level = usize;
    fn empty_item(&self) -> Stack {
        self.items[2].clone()
    }
    fn is_empty(&self, item: Stack) -> bool {
        item.read().is_empty()
    }
    fn same_item_type(&self, a: Stack, b: Stack) -> bool {
        a.read().same_item(&b.read())
    }
    fn using_item(&mut self, s: &mut UseState<Stack>) -> io::Result<bool> {
        self.event(s, "using")?;
        Ok(self.using)
    }
    fn used_hand(&mut self, s: &mut UseState<Stack>) -> io::Result<usize> {
        self.event(s, "hand")?;
        Ok(self.state()["hand"].as_u64().unwrap() as usize)
    }
    fn hand_item(&mut self, s: &mut UseState<Stack>, hand: usize) -> io::Result<Stack> {
        self.event(s, &format!("get-hand {hand}"))?;
        Ok(self.items[self.hands[hand]].clone())
    }
    fn set_hand_item(
        &mut self,
        s: &mut UseState<Stack>,
        hand: usize,
        item: Stack,
    ) -> io::Result<()> {
        self.event(s, &format!("set-hand {hand} {}", self.id(&item)))?;
        self.hands[hand] = self.id(&item);
        Ok(())
    }
    fn level(&mut self, s: &mut UseState<Stack>) -> io::Result<usize> {
        self.event(s, "level")?;
        Ok(self.state()["level"].as_u64().unwrap() as usize)
    }
    fn is_client(&mut self, s: &mut UseState<Stack>, level: usize) -> io::Result<bool> {
        self.event(s, &format!("client {level}"))?;
        Ok(self.state()["clients"][level].as_bool().unwrap())
    }
    fn remaining_ticks(&mut self, s: &mut UseState<Stack>) -> io::Result<i32> {
        self.event(s, "remaining")?;
        Ok(self.state()["virtual_remaining"].as_i64().unwrap() as i32)
    }
    fn item_tick(
        &mut self,
        s: &mut UseState<Stack>,
        item: Stack,
        level: usize,
        remaining: i32,
    ) -> io::Result<()> {
        let kind = self.kind(&item);
        if kind == 2 {
            return Ok(());
        }
        self.event(
            s,
            &format!("tick {kind} {} {level} {remaining}", self.id(&item)),
        )
    }
    fn use_on_release(&mut self, s: &mut UseState<Stack>, item: Stack) -> io::Result<bool> {
        let kind = self.kind(&item);
        if kind == 2 {
            return Ok(false);
        }
        self.event(s, &format!("release-query {kind} {}", self.id(&item)))?;
        Ok(self.state()["releases"][kind].as_bool().unwrap())
    }
    fn release_item(
        &mut self,
        s: &mut UseState<Stack>,
        item: Stack,
        level: usize,
        remaining: i32,
    ) -> io::Result<()> {
        let kind = self.kind(&item);
        if kind == 2 {
            return Ok(());
        }
        self.event(
            s,
            &format!("release {kind} {} {level} {remaining}", self.id(&item)),
        )
    }
    fn finish_item(
        &mut self,
        s: &mut UseState<Stack>,
        item: Stack,
        level: usize,
    ) -> io::Result<Stack> {
        let kind = self.kind(&item);
        if kind == 2 {
            return Ok(item);
        }
        self.event(s, &format!("finish {kind} {} {level}", self.id(&item)))?;
        Ok(self.items[self.state()["finishes"][kind].as_u64().unwrap() as usize].clone())
    }
    fn set_using_flag(&mut self, s: &mut UseState<Stack>, using: bool) -> io::Result<()> {
        self.event(s, &format!("flag {using}"))?;
        self.using = using;
        Ok(())
    }
    fn finish_vibration(&mut self, s: &mut UseState<Stack>, item: Stack) -> io::Result<()> {
        if self.id(&item) < 2 && self.f["vibrations"][self.id(&item)].as_bool().unwrap() {
            self.event(s, "vibration")?;
        }
        Ok(())
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let stacks = ItemStacks::new(2);
        let items = [
            stacks.create(0, 1, Prototype::new([])),
            stacks.create(
                if f["shared_type"].as_bool().unwrap() {
                    0
                } else {
                    1
                },
                1,
                Prototype::new([]),
            ),
            stacks.empty(),
        ];
        let mut s = UseState {
            item: items[2].clone(),
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
            items,
        };
        w.reset(&mut s);
        let result = match f["mode"].as_str().unwrap() {
            "tick" => item_use_lifecycle::tick(&mut w, &mut s),
            "update" => {
                let item = w.items[f["argument"].as_u64().unwrap() as usize].clone();
                item_use_lifecycle::update(&mut w, &mut s, item)
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
            w.id(&s.item),
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
