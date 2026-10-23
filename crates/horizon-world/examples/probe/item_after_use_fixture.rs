use horizon_world::{
    component_patch::{Patch, Prototype},
    identifier::Identifier,
    item_after_use::{self, AfterUseWorld},
    item_cooldowns::{self, CooldownUser, ItemCooldowns, Observer, UseCooldown},
    item_stack_refs::{ItemStacks, StackRef},
    item_stack_storage::ItemStackData,
    item_template::{ItemTemplate, ValidationRules},
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
#[derive(Clone, Debug, PartialEq)]
enum Component {
    Integer(i32),
    Remainder(ItemTemplate<usize, u8, Component>),
    Cooldown(UseCooldown<Identifier>),
}
type Stack = StackRef<usize, u8, Component>;
type StackData = ItemStackData<usize, u8, Component>;
type Template = ItemTemplate<usize, u8, Component>;
type Book = ItemCooldowns<Identifier>;
fn n(v: &Value, k: &str) -> i32 {
    v[k].as_i64().unwrap() as i32
}
fn integer(v: Option<&Component>) -> Option<i32> {
    match v {
        Some(Component::Integer(n)) => Some(*n),
        _ => None,
    }
}
fn bits(f: f32) -> u32 {
    if f.is_nan() { 0x7fc00000 } else { f.to_bits() }
}
fn cooldown(v: &Value) -> Option<UseCooldown<Identifier>> {
    v["cooldown"].as_u64().map(|b| UseCooldown {
        seconds: f32::from_bits(b as u32),
        group: usize::try_from(n(v, "group"))
            .ok()
            .map(|i| Identifier::parse(&format!("test:group{i}")).unwrap()),
    })
}
fn template(v: &Value) -> Option<Template> {
    v["remainder"].as_i64().map(|count| {
        let mut p = Vec::new();
        if let Some(m) = v["maximum"].as_i64() {
            p.push((0, Some(Component::Integer(m as i32))));
        }
        if v["damage"].as_bool().unwrap() {
            p.push((1, Some(Component::Integer(0))));
        }
        Template::new(2, 0, count as i32, Patch::new(p)).unwrap()
    })
}
fn components(stack: &Stack, v: &Value) {
    let mut s = stack.write();
    s.components
        .set(2, template(v).map(Component::Remainder))
        .unwrap();
    s.components
        .set(3, cooldown(v).map(Component::Cooldown))
        .unwrap();
}
struct Data {
    f: Value,
    out: io::BufWriter<fs::File>,
    refs: [Stack; 3],
    extra: Option<Stack>,
    original: Option<Stack>,
    result: Option<Stack>,
    changed: bool,
    calls: i32,
    prototypes: [Prototype<u8, Component>; 3],
    store: ItemStacks<usize, u8, Component>,
}
impl Data {
    fn config(&self) -> &Value {
        &self.f[if self.changed { "after" } else { "initial" }]
    }
    fn snapshot(&self, s: Option<&Stack>) -> String {
        let Some(s) = s else {
            return "none".into();
        };
        let d = s.read();
        let remainder = match d.component(&2) {
            Some(Component::Remainder(t)) => format!(
                "{}:{}",
                t.count(),
                integer(t.component(&self.prototypes[t.item()], &0)).unwrap_or(1)
            ),
            _ => "none".into(),
        };
        let cooldown = match d.component(&3) {
            Some(Component::Cooldown(c)) => format!(
                "{}:{}",
                bits(c.seconds),
                c.group.as_ref().map_or("none".into(), ToString::to_string)
            ),
            _ => "none".into(),
        };
        format!(
            "{} {} {} {} {} {} {}",
            s.same_reference(&self.store.empty()),
            d.item(),
            d.count,
            d.count(),
            d.pop_time,
            remainder,
            cooldown
        )
    }
    fn all(&mut self, book: &Book) {
        writeln!(
            self.out,
            "state {} {} {}",
            self.calls, self.changed, book.tick_count
        )
        .unwrap();
        let slots = [
            Some(self.refs[0].clone()),
            Some(self.refs[1].clone()),
            Some(self.refs[2].clone()),
            self.extra.clone(),
            self.original.clone(),
            self.result.clone(),
        ];
        for (i, s) in slots.iter().enumerate() {
            writeln!(self.out, "ref {i} {}", self.snapshot(s.as_ref())).unwrap();
        }
        let mut same = Vec::new();
        for a in 0..slots.len() {
            for b in a + 1..slots.len() {
                same.push((slots[a] == slots[b]).to_string());
            }
        }
        writeln!(self.out, "same {}", same.join(",")).unwrap();
        let entries = book
            .entries()
            .map(|(k, v)| format!("{k}={}:{}", v.start, v.end))
            .collect::<Vec<_>>()
            .join(",");
        writeln!(self.out, "cooldowns {entries}").unwrap();
    }
    fn reset(&mut self, book: &mut Book) {
        let c = self.config().clone();
        self.prototypes[2] = Prototype::new([(0, Component::Integer(n(&c, "prototype_max")))]);
        for i in 0..2 {
            let v = &c["items"][i];
            self.refs[i].write().count = n(v, "count");
            components(&self.refs[i], v);
        }
        book.tick_count = n(&c, "clock");
        if let Some(extra) = &self.extra {
            extra.write().count = n(&c, "extra_count");
        }
        if let Some(original) = &self.original {
            original.write().count = n(&c, "original_count");
            components(original, &c["items"][1]);
        }
    }
    fn event(&mut self, book: &mut Book, name: String) -> Result<(), &'static str> {
        writeln!(self.out, "event {name}").unwrap();
        let index = self.calls;
        self.calls += 1;
        self.all(book);
        if index == n(&self.f, "fail") {
            return Err("callback");
        }
        if index == n(&self.f, "mutate") {
            self.changed = true;
            self.reset(book);
        }
        Ok(())
    }
}
impl Observer<Identifier> for Data {
    type Error = &'static str;
    fn started(&mut self, b: &mut Book, g: Identifier, d: i32) -> Result<(), Self::Error> {
        self.event(b, format!("started {g} {d}"))
    }
    fn ended(&mut self, _: &mut Book, _: Identifier) -> Result<(), Self::Error> {
        unreachable!()
    }
}
struct World {
    data: Data,
    book: Book,
}
impl ValidationRules<usize, u8, Component> for World {
    type ContentsError = std::convert::Infallible;
    fn has_max_damage(&self, s: &StackData) -> bool {
        s.component(&1).is_some()
    }
    fn max_stack_size(&self, s: &StackData) -> i32 {
        integer(s.component(&0)).unwrap_or(1)
    }
    fn validate_contents(&self, _: &StackData) -> Result<(), Self::ContentsError> {
        Ok(())
    }
}
impl CooldownUser for World {
    type Error = &'static str;
    type Stack = Stack;
    type Manager = ();
    fn is_player(&self) -> bool {
        self.data.f["player"].as_bool().unwrap()
    }
    fn cooldowns(&mut self) -> Result<(), Self::Error> {
        self.data.event(&mut self.book, "get-cooldowns".into())
    }
    fn add_cooldown(&mut self, _: (), stack: Stack, ticks: i32) -> Result<(), Self::Error> {
        self.data.original = Some(stack.clone());
        self.data
            .event(&mut self.book, format!("add-cooldown {ticks}"))?;
        let group = {
            let s = stack.read();
            let c = match s.component(&3) {
                Some(Component::Cooldown(c)) => Some(c),
                _ => None,
            };
            item_cooldowns::group(c, || {
                Identifier::parse(if s.item() == 2 {
                    "minecraft:stick"
                } else {
                    "minecraft:air"
                })
                .unwrap()
            })
        };
        self.book.add(group, ticks, &mut self.data)
    }
}
impl AfterUseWorld<usize, u8, Component> for World {
    type Error = &'static str;
    type Level = ();
    type Group = Identifier;
    fn remainder(&self, s: &StackData) -> Option<Template> {
        match s.component(&2) {
            Some(Component::Remainder(t)) => Some(t.clone()),
            _ => None,
        }
    }
    fn cooldown(&self, s: &StackData) -> Option<UseCooldown<Identifier>> {
        match s.component(&3) {
            Some(Component::Cooldown(c)) => Some(c.clone()),
            _ => None,
        }
    }
    fn prototype(&self, item: usize) -> Prototype<u8, Component> {
        self.data.prototypes[item].clone()
    }
    fn finish_item(&mut self, s: Stack, _: ()) -> Result<Stack, Self::Error> {
        let kind = s.read().item();
        if kind == 1 {
            self.data.event(&mut self.book, "finish".into())?;
            Ok(self.data.refs[n(self.data.config(), "finish") as usize].clone())
        } else {
            Ok(s)
        }
    }
    fn infinite_materials(&mut self) -> Result<bool, Self::Error> {
        self.data.event(&mut self.book, "infinite".into())?;
        Ok(self.data.config()["infinite"].as_bool().unwrap())
    }
    fn extra_item(&mut self, s: Stack) -> Result<(), Self::Error> {
        self.data.extra = Some(s);
        self.data.event(&mut self.book, "extra".into())
    }
    fn apply_cooldown(
        &mut self,
        c: UseCooldown<Identifier>,
        original: Stack,
    ) -> Result<(), Self::Error> {
        c.apply(self, original)
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let store = ItemStacks::new(0);
        let prototypes = [
            Prototype::new([]),
            Prototype::new([]),
            Prototype::new([(0, Component::Integer(n(&f["initial"], "prototype_max")))]),
        ];
        let refs = [
            store.create(1, 1, prototypes[1].clone()),
            store.create(2, 1, prototypes[2].clone()),
            store.empty(),
        ];
        refs[0].write().pop_time = 5;
        refs[1].write().pop_time = 7;
        let mut w = World {
            data: Data {
                f,
                out,
                refs,
                extra: None,
                original: None,
                result: None,
                changed: false,
                calls: 0,
                prototypes,
                store: store.clone(),
            },
            book: Book::default(),
        };
        w.data.reset(&mut w.book);
        w.data.all(&w.book);
        let source = w.data.refs[n(&w.data.f, "source") as usize].clone();
        let result = item_after_use::finish(&store, &mut w, source, ());
        match result {
            Ok(s) => {
                w.data.result = Some(s);
                writeln!(w.data.out, "result ok")?;
            }
            Err(e) => writeln!(w.data.out, "result {e}")?,
        }
        w.data.all(&w.book);
        out = w.data.out;
    }
    Ok(())
}
