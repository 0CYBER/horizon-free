use horizon_world::{
    component_patch::Prototype,
    identifier::Identifier,
    item_cooldowns::{self, CooldownUser, ItemCooldowns, Observer, TickError, UseCooldown},
    item_stack_refs::{ItemStacks, StackRef},
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
type Book = ItemCooldowns<Identifier>;
type Stack = StackRef<usize, u8, UseCooldown<Identifier>>;
fn n(v: &Value, k: &str) -> i32 {
    v[k].as_i64().unwrap() as i32
}
fn fl(v: &Value, k: &str) -> f32 {
    f32::from_bits(v[k].as_u64().unwrap() as u32)
}
fn bits(f: f32) -> u32 {
    if f.is_nan() { 0x7fc00000 } else { f.to_bits() }
}
fn group(stack: &Stack) -> Identifier {
    let s = stack.read();
    item_cooldowns::group(s.component(&0), || {
        Identifier::parse(["minecraft:air", "minecraft:stick", "minecraft:shield"][s.item()])
            .unwrap()
    })
}
fn tick(book: &mut Book, events: &mut Events) -> Result<(), &'static str> {
    book.tick(events).map_err(|e| match e {
        TickError::Iterator(_) => "modified",
        TickError::Observer(e) => e,
    })
}
struct Events {
    out: io::BufWriter<fs::File>,
    groups: Vec<Identifier>,
    stack: Stack,
    op: Value,
    calls: i32,
    quiet: bool,
}
impl Events {
    fn snapshot(&mut self, book: &Book) {
        let entries = book
            .entries()
            .map(|(k, v)| format!("{k}={}:{}", v.start, v.end))
            .collect::<Vec<_>>()
            .join(",");
        writeln!(
            self.out,
            "state {} {} | {}",
            book.tick_count, self.calls, entries
        )
        .unwrap();
        writeln!(
            self.out,
            "stack {} {}",
            self.stack.read().count(),
            group(&self.stack)
        )
        .unwrap();
    }
    fn event(&mut self, book: &mut Book, name: String) -> Result<(), &'static str> {
        if self.quiet {
            return Ok(());
        }
        writeln!(self.out, "event {name}").unwrap();
        let index = self.calls;
        self.calls += 1;
        self.snapshot(book);
        if index == n(&self.op, "fail") {
            return Err("callback");
        }
        if index == n(&self.op, "mutate") {
            let other = self.groups[n(&self.op, "other") as usize].clone();
            let after = n(&self.op, "after");
            match self.op["action"].as_str().unwrap() {
                "add" => book.add(other, after, self)?,
                "remove" => book.remove(other, self)?,
                "clock" => book.tick_count = after,
                "tick" => tick(book, self)?,
                "stack" => {
                    let mut s = self.stack.write();
                    s.count = after;
                    s.components
                        .set(
                            0,
                            Some(UseCooldown {
                                seconds: 1.0,
                                group: Some(other),
                            }),
                        )
                        .unwrap();
                }
                "none" => {}
                _ => panic!(),
            }
        }
        Ok(())
    }
}
impl Observer<Identifier> for Events {
    type Error = &'static str;
    fn started(&mut self, b: &mut Book, g: Identifier, d: i32) -> Result<(), Self::Error> {
        self.event(b, format!("started {g} {d}"))
    }
    fn ended(&mut self, b: &mut Book, g: Identifier) -> Result<(), Self::Error> {
        self.event(b, format!("ended {g}"))
    }
}
struct User<'a> {
    book: &'a mut Book,
    events: &'a mut Events,
    player: bool,
}
impl CooldownUser for User<'_> {
    type Error = &'static str;
    type Stack = Stack;
    type Manager = ();
    fn is_player(&self) -> bool {
        self.player
    }
    fn cooldowns(&mut self) -> Result<(), Self::Error> {
        self.events.event(self.book, "get-cooldowns".into())
    }
    fn add_cooldown(&mut self, _: (), stack: Stack, ticks: i32) -> Result<(), Self::Error> {
        self.book.add(group(&stack), ticks, self.events)
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let groups: Vec<_> = f["groups"]
            .as_array()
            .unwrap()
            .iter()
            .map(|v| Identifier::parse(v.as_str().unwrap()).unwrap())
            .collect();
        let store = ItemStacks::new(0);
        let stack = store.create(n(&f, "item") as usize, n(&f, "count"), Prototype::new([]));
        if f["component"].as_bool().unwrap() {
            stack.write().components.set(
                0,
                Some(UseCooldown {
                    seconds: fl(&f, "seconds"),
                    group: usize::try_from(n(&f, "group"))
                        .ok()
                        .map(|i| groups[i].clone()),
                }),
            )?;
        }
        let mut events = Events {
            out,
            groups,
            stack,
            op: Value::Null,
            calls: 0,
            quiet: true,
        };
        let mut book = Book::default();
        book.tick_count = n(&f, "clock");
        for pair in f["initial"].as_array().unwrap() {
            let g = events.groups[pair[0].as_u64().unwrap() as usize].clone();
            book.add(g, pair[1].as_i64().unwrap() as i32, &mut events)
                .unwrap();
        }
        events.quiet = false;
        events.snapshot(&book);
        for op in f["ops"].as_array().unwrap() {
            events.op = op.clone();
            events.calls = 0;
            let result = match op["kind"].as_str().unwrap() {
                "add" => book.add(
                    events.groups[n(op, "group") as usize].clone(),
                    n(op, "duration"),
                    &mut events,
                ),
                "add-stack" => book.add(group(&events.stack), n(op, "duration"), &mut events),
                "remove" => {
                    book.remove(events.groups[n(op, "group") as usize].clone(), &mut events)
                }
                "tick" => tick(&mut book, &mut events),
                "clock" => {
                    book.tick_count = n(op, "duration");
                    Ok(())
                }
                "percent" => {
                    let group = group(&events.stack);
                    writeln!(
                        events.out,
                        "percent {} {}",
                        bits(book.percent(&group, fl(op, "partial"))),
                        book.is_on_cooldown(&group)
                    )?;
                    Ok(())
                }
                "duration" => {
                    writeln!(
                        events.out,
                        "duration {}",
                        UseCooldown::<Identifier> {
                            seconds: fl(op, "seconds"),
                            group: None
                        }
                        .ticks()
                    )?;
                    Ok(())
                }
                "apply" => {
                    let component = UseCooldown {
                        seconds: fl(op, "seconds"),
                        group: Some(events.groups[n(op, "group") as usize].clone()),
                    };
                    let stack = events.stack.clone();
                    component.apply(
                        &mut User {
                            book: &mut book,
                            events: &mut events,
                            player: op["player"].as_bool().unwrap(),
                        },
                        stack,
                    )
                }
                _ => panic!(),
            };
            writeln!(events.out, "result {}", result.err().unwrap_or("ok"))?;
            events.snapshot(&book);
        }
        out = events.out;
    }
    Ok(())
}
