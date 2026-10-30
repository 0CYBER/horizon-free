//! The scoreboard model for differential tests against vanilla's `Scoreboard`
//! (tools/test_scoreboard_parity.py): runs the same scripted operations as
//! VanillaScoreboardOracle on `horizon_client::scoreboard::Scoreboard` and
//! prints the same JSON lines (results, dumps, the packed save data before
//! and after reloading it). With a vanilla output file it also reloads each
//! case's vanilla NBT and packs it again (`vreload`).
//!
//! usage: scoreboard_fixture REGISTRIES.json INPUTS OUTPUT [VANILLA_OUTPUT]
use horizon_client::{
    command::Registries,
    scoreboard::{
        self as sb, CollisionRule, Comp, NumberFormat, RenderType, Scoreboard, Visibility,
        canonical_component, criteria_by_name, criteria_read_only, criteria_render_type,
    },
};
use horizon_world::nbt::{self, NamedTag, Tag};
use serde_json::{Map, Value, json};
use std::{collections::BTreeMap, fs, io::Write};

fn canon(tag: &Tag) -> Value {
    match tag {
        Tag::Compound(c) => {
            let sorted: BTreeMap<String, &Tag> =
                c.iter().map(|(k, v)| (k.to_string_lossy(), v)).collect();
            Value::Object(sorted.into_iter().map(|(k, v)| (k, canon(v))).collect())
        }
        Tag::List(l) => Value::Array(l.iter().map(canon).collect()),
        Tag::String(s) => Value::String(format!("'{}", s.to_string_lossy())),
        Tag::Byte(v) => Value::String(format!("{v}b")),
        Tag::Short(v) => Value::String(format!("{v}s")),
        Tag::Int(v) => Value::String(format!("{v}i")),
        Tag::Long(v) => Value::String(format!("{v}L")),
        Tag::Float(v) => Value::String(format!("f:{v:x}")),
        Tag::Double(v) => Value::String(format!("d:{v:x}")),
        Tag::IntArray(v) => json!({ "I;": v }),
        other => Value::String(format!("?{other:?}")),
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn unhex(text: &str) -> Vec<u8> {
    (0..text.len())
        .step_by(2)
        .map(|i| u8::from_str_radix(&text[i..i + 2], 16).unwrap_or(0))
        .collect()
}
fn encode(tag: &Tag) -> Vec<u8> {
    nbt::write_named(
        &NamedTag {
            name: "".into(),
            tag: tag.clone(),
        },
        Default::default(),
    )
    .expect("nbt")
}
fn decode(bytes: &[u8]) -> Tag {
    nbt::read_named(bytes, Default::default())
        .expect("nbt")
        .0
        .tag
}

fn comp(v: Option<&Value>) -> Option<Comp> {
    v.filter(|v| !v.is_null()).cloned()
}
fn comp_json(c: Option<&Comp>) -> Value {
    c.map_or(Value::Null, canonical_component)
}
fn fmt_json(f: Option<&NumberFormat>) -> Value {
    f.map_or(Value::Null, NumberFormat::to_json)
}
fn fmt(v: Option<&Value>) -> Option<NumberFormat> {
    v.filter(|v| !v.is_null()).and_then(NumberFormat::from_json)
}
fn s<'a>(op: &'a Value, k: &str) -> &'a str {
    op[k].as_str().unwrap_or_default()
}

struct Fixture {
    reg: Registries,
}
impl Fixture {
    fn criteria(&self, name: &str) -> Value {
        match criteria_by_name(name, &self.reg) {
            Ok(Some(c)) => Value::String(c),
            Ok(None) => Value::Null,
            Err(_) => Value::String("!invalid".into()),
        }
    }
    fn dump(&self, b: &Scoreboard, probes: &[Value]) -> Value {
        let objectives: Vec<Value> = b
            .objectives
            .iter()
            .map(|o| {
                json!({
                    "name": o.name, "criteria": o.criteria, "display": comp_json(Some(&o.display_name)),
                    "render": o.render_type.id(), "auto": o.display_auto_update,
                    "format": fmt_json(o.number_format.as_ref()),
                })
            })
            .collect();
        let stream: Vec<Value> = b
            .objectives_streamed()
            .iter()
            .map(|o| Value::String(o.name.clone()))
            .collect();
        let tracked = b.tracked();
        let mut lists = Map::new();
        for o in &b.objectives {
            let rows: Vec<Value> = b
                .list_scores(&o.name)
                .iter()
                .map(|e| {
                    json!([
                        e.owner,
                        e.value,
                        comp_json(e.display.as_ref()),
                        fmt_json(e.number_format.as_ref())
                    ])
                })
                .collect();
            lists.insert(o.name.clone(), Value::Array(rows));
        }
        let mut holders = Map::new();
        for h in &tracked {
            let mut per = Map::new();
            for o in &b.objectives {
                if let Some(sc) = b.score(h, &o.name) {
                    per.insert(o.name.clone(), score_json(sc));
                }
            }
            holders.insert(h.clone(), Value::Object(per));
        }
        let teams: Vec<Value> = b
            .teams
            .iter()
            .map(|t| {
                json!({
                    "name": t.name, "display": comp_json(Some(&t.display_name)), "color": t.color,
                    "ff": t.allow_friendly_fire, "si": t.see_friendly_invisibles,
                    "prefix": comp_json(Some(&t.prefix)), "suffix": comp_json(Some(&t.suffix)),
                    "nametag": t.nametag_visibility.name(), "death": t.death_message_visibility.name(),
                    "collision": t.collision_rule.name(),
                    "players": t.players.iter().collect::<Vec<_>>(),
                })
            })
            .collect();
        let teams_stream: Vec<Value> = b
            .teams_streamed()
            .iter()
            .map(|t| Value::String(t.name.clone()))
            .collect();
        let mut slots = Map::new();
        for slot in sb::display_slots() {
            if let Some(o) = b.display_objective(&slot) {
                slots.insert(slot, Value::String(o.name.clone()));
            }
        }
        let mut team_of = Map::new();
        for p in probes {
            let p = p.as_str().unwrap_or_default();
            team_of.insert(
                p.to_owned(),
                b.players_team(p)
                    .map_or(Value::Null, |t| Value::String(t.name.clone())),
            );
        }
        json!({
            "objectives": objectives, "objectives_stream": stream, "tracked": tracked,
            "lists": lists, "holders": holders, "teams": teams, "teams_stream": teams_stream,
            "slots": slots, "team_of": team_of,
        })
    }
    fn apply(&self, b: &mut Scoreboard, op: &Value, probes: &[Value]) -> Value {
        let kind = s(op, "op");
        let holder = s(op, "holder");
        let dname = comp(op.get("dname"));
        match kind {
            "crit" => {
                let name = self.criteria(s(op, "name"));
                let mut o = json!({ "name": name });
                if let Some(n) = name.as_str().filter(|n| !n.starts_with('!')) {
                    o["readonly"] = Value::Bool(criteria_read_only(n));
                    o["render"] = Value::String(criteria_render_type(n).id().into());
                }
                o
            }
            "obj_add" => {
                let Some(criteria) = self
                    .criteria(s(op, "criteria"))
                    .as_str()
                    .filter(|c| !c.starts_with('!'))
                    .map(str::to_owned)
                else {
                    return json!("bad");
                };
                if b.objective(s(op, "name")).is_some() {
                    return json!("exists");
                }
                b.add_objective(
                    s(op, "name"),
                    &criteria,
                    comp(op.get("display")).unwrap_or(Value::Null),
                    RenderType::by_id(s(op, "render")),
                    op["auto"].as_bool().unwrap_or(false),
                    fmt(op.get("fmt")),
                );
                json!("ok")
            }
            "obj_rm" => {
                if b.objective(s(op, "name")).is_none() {
                    return json!("missing");
                }
                b.remove_objective(s(op, "name"));
                json!("ok")
            }
            "obj_mod" => {
                let Some(o) = b.objective_mut(s(op, "name")) else {
                    return json!("missing");
                };
                let v = &op["value"];
                match s(op, "field") {
                    "display" => o.display_name = v.clone(),
                    "render" => o.render_type = RenderType::by_id(v.as_str().unwrap_or_default()),
                    "auto" => o.display_auto_update = v.as_bool().unwrap_or(false),
                    "fmt" => o.number_format = fmt(Some(v)),
                    other => panic!("field {other}"),
                }
                json!("ok")
            }
            "slot" => {
                let name = op["name"].as_str();
                if let Some(n) = name
                    && b.objective(n).is_none()
                {
                    return json!("missing");
                }
                b.set_display_objective(s(op, "slot"), name);
                json!("ok")
            }
            "set" | "add" | "get" | "lock" | "unlock" | "sdisplay" | "sfmt" => {
                let name = s(op, "name");
                let Some(criteria) = b.objective(name).map(|o| o.criteria.clone()) else {
                    return json!("missing");
                };
                let force = op["force"].as_bool().unwrap_or(false);
                let writable = force || !criteria_read_only(&criteria);
                let current = b.get(holder, name);
                match kind {
                    "set" | "add" => {
                        if !writable {
                            return json!("readonly");
                        }
                        let value = op["value"].as_i64().unwrap_or(0) as i32;
                        let new = if kind == "add" {
                            current.wrapping_add(value)
                        } else {
                            value
                        };
                        b.set_with(holder, name, new, dname.as_ref());
                        if kind == "add" {
                            return json!(new);
                        }
                    }
                    "get" => return json!(current),
                    "lock" => b.score_mut(holder, name).locked = true,
                    "unlock" => b.score_mut(holder, name).locked = false,
                    "sdisplay" => b.score_mut(holder, name).display = comp(op.get("value")),
                    "sfmt" => b.score_mut(holder, name).number_format = fmt(op.get("value")),
                    _ => unreachable!(),
                }
                json!("ok")
            }
            "info" => {
                let name = s(op, "name");
                if b.objective(name).is_none() {
                    return json!("missing");
                }
                b.score(holder, name).map_or(Value::Null, score_json)
            }
            "reset" => {
                match op["name"].as_str() {
                    None => b.reset_all(holder),
                    Some(name) => {
                        if b.objective(name).is_none() {
                            return json!("missing");
                        }
                        b.reset_one(holder, name);
                    }
                }
                json!("ok")
            }
            "all" => {
                let criteria = self.criteria(s(op, "criteria"));
                let criteria = criteria.as_str().unwrap_or_default();
                let value = op["value"].as_i64().unwrap_or(0) as i32;
                let k = s(op, "kind");
                b.for_all_objectives_with(criteria, holder, dname.as_ref(), |v| match k {
                    "inc" => v.wrapping_add(1),
                    "add" => v.wrapping_add(value),
                    "set" => value,
                    _ => 0,
                });
                json!("ok")
            }
            "team_add" => {
                if b.team(s(op, "name")).is_some() {
                    return json!("exists");
                }
                b.add_team(s(op, "name"));
                json!("ok")
            }
            "team_rm" => {
                if b.team(s(op, "name")).is_none() {
                    return json!("missing");
                }
                b.remove_team(s(op, "name"));
                json!("ok")
            }
            "join" => {
                if b.team(s(op, "team")).is_none() {
                    return json!("missing");
                }
                json!(b.add_to_team(holder, s(op, "team")))
            }
            "leave" => json!(b.remove_from_team(holder)),
            "team_set" => {
                let Some(t) = b.team_mut(s(op, "name")) else {
                    return json!("missing");
                };
                let v = &op["value"];
                let name = |v: &Value| v.as_str().unwrap_or_default().to_owned();
                match s(op, "field") {
                    "display" => t.display_name = v.clone(),
                    "color" => t.color = v.as_str().map(str::to_owned),
                    "ff" => t.allow_friendly_fire = v.as_bool().unwrap_or(true),
                    "si" => t.see_friendly_invisibles = v.as_bool().unwrap_or(true),
                    "prefix" => t.prefix = comp(Some(v)).unwrap_or_else(|| sb::literal("")),
                    "suffix" => t.suffix = comp(Some(v)).unwrap_or_else(|| sb::literal("")),
                    "nametag" => {
                        t.nametag_visibility = Visibility::by_name(&name(v)).expect("visibility")
                    }
                    "death" => {
                        t.death_message_visibility =
                            Visibility::by_name(&name(v)).expect("visibility")
                    }
                    "collision" => {
                        t.collision_rule = CollisionRule::by_name(&name(v)).expect("collision")
                    }
                    other => panic!("field {other}"),
                }
                json!("ok")
            }
            "removed" => {
                b.entity_removed(holder);
                json!("ok")
            }
            "allied" => json!(b.allied(s(op, "a"), s(op, "b"))),
            "dump" => self.dump(b, probes),
            other => panic!("op {other}"),
        }
    }
}
fn score_json(sc: &sb::Score) -> Value {
    json!({
        "value": sc.value, "locked": sc.locked, "display": comp_json(sc.display.as_ref()),
        "format": fmt_json(sc.number_format.as_ref()),
    })
}

pub fn main() {
    let args: Vec<String> = crate::args().collect();
    let registries: Value =
        serde_json::from_str(&fs::read_to_string(&args[1]).expect("registries")).expect("json");
    let list = |k: &str| -> Vec<String> {
        registries[k]
            .as_array()
            .map(|a| {
                a.iter()
                    .filter_map(|v| v.as_str().map(str::to_owned))
                    .collect()
            })
            .unwrap_or_default()
    };
    let reg = Registries {
        items: list("items"),
        blocks: list("blocks")
            .into_iter()
            .map(|b| (b, Vec::new()))
            .collect(),
        entity_types: list("entity_types"),
        ..Registries::default()
    };
    let fixture = Fixture { reg };
    let vanilla: BTreeMap<i64, String> = args
        .get(4)
        .and_then(|p| fs::read_to_string(p).ok())
        .map(|text| {
            text.lines()
                .filter_map(|l| serde_json::from_str::<Value>(l).ok())
                .filter_map(|v| Some((v["case"].as_i64()?, v["nbt"].as_str()?.to_owned())))
                .collect()
        })
        .unwrap_or_default();
    let mut out = std::io::BufWriter::new(fs::File::create(&args[3]).expect("output"));
    for line in fs::read_to_string(&args[2]).expect("inputs").lines() {
        let case: Value = serde_json::from_str(line).expect("case");
        let probes = case["probes"].as_array().cloned().unwrap_or_default();
        let mut board = Scoreboard::default();
        let outs: Vec<Value> = case["ops"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|op| fixture.apply(&mut board, op, &probes))
            .collect();
        let packed = board.pack();
        let bytes = encode(&packed);
        let reloaded = Scoreboard::unpack(&decode(&bytes));
        let mut result = json!({
            "case": case["case"], "out": outs, "pack": canon(&packed), "nbt": hex(&bytes),
            "reload": canon(&reloaded.pack()), "reload_dump": fixture.dump(&reloaded, &probes),
        });
        if let Some(v) = case["case"].as_i64().and_then(|c| vanilla.get(&c)) {
            result["vreload"] = canon(&Scoreboard::unpack(&decode(&unhex(v))).pack());
        }
        writeln!(out, "{result}").expect("write");
    }
}
