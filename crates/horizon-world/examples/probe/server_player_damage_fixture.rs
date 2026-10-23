use crate::support::common::f32_bits;
use horizon_world::{
    attributes::{
        Modifier, Operation,
        entity::{Catalog, EntityAttributes},
    },
    combat_rules::ArmorWorld,
    damage_reduction::Bypass,
    food::FoodData,
    living_damage::DamageStat,
    player_health::PlayerFloatIds,
    server_player_damage::{ServerDamageEnvironment, ServerPlayerDamage},
    synced_floats::{FloatValue, Observer, SyncedFloats},
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
    sync::Arc,
};
fn single(v: &Value) -> f32 {
    f32::from_bits(u32::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn number(v: &Value) -> f64 {
    f64::from_bits(u64::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn values(v: Option<&[FloatValue]>) -> String {
    match v {
        None => "none".into(),
        Some([]) => "empty".into(),
        Some(v) => v
            .iter()
            .map(|v| format!("{}:{:08x}", v.id, f32_bits(v.value)))
            .collect::<Vec<_>>()
            .join(","),
    }
}
struct Env<'a> {
    out: &'a mut dyn Write,
    op: &'a Value,
    slots: &'a [Value],
    calls: i64,
    recorded: f32,
}
impl Env<'_> {
    fn event(&mut self, s: &str) -> io::Result<()> {
        writeln!(self.out, "{s}")?;
        let n = self.calls;
        self.calls += 1;
        if n == self.op["fail"].as_i64().unwrap() {
            Err(io::Error::other("injected damage callback failure"))
        } else {
            Ok(())
        }
    }
    fn flag(&self, s: &str) -> bool {
        self.op[s].as_bool().unwrap()
    }
}
impl Observer for Env<'_> {
    type Error = io::Error;
    fn updated(&mut self, data: &mut SyncedFloats, id: u8) -> io::Result<()> {
        self.event(&format!(
            "updated {id} {:08x} {}",
            f32_bits(data.get(id).unwrap()),
            data.is_dirty()
        ))?;
        if self.flag("pack_callback") {
            writeln!(
                self.out,
                "callback-pack {}",
                values(data.take_dirty().as_deref())
            )?;
        }
        Ok(())
    }
}
impl ArmorWorld for Env<'_> {
    type Error = io::Error;
    type Weapon = ();
    type ServerLevel = ();
    fn weapon(&mut self) -> io::Result<Option<()>> {
        self.event("weapon")?;
        Ok(self.flag("weapon").then_some(()))
    }
    fn server_level(&mut self) -> io::Result<Option<()>> {
        self.event("level")?;
        Ok(Some(()))
    }
    fn modify_armor_effectiveness(&mut self, _: (), _: (), v: f32) -> io::Result<f32> {
        Ok(v)
    }
}
impl ServerDamageEnvironment for Env<'_> {
    type Attacker = ();
    fn invulnerable(&mut self) -> io::Result<bool> {
        self.event("invulnerable")?;
        Ok(self.flag("invulnerable"))
    }
    fn bypasses(&mut self, b: Bypass) -> io::Result<bool> {
        let n = match b {
            Bypass::Armor => "armor",
            Bypass::Effects => "effects",
            Bypass::Resistance => "resistance",
            Bypass::Enchantments => "enchantments",
        };
        self.event(&format!("bypass {n}"))?;
        Ok(self.flag(&format!("bypass_{n}")))
    }
    fn hurt_armor(&mut self, a: &mut EntityAttributes, v: f32) -> io::Result<()> {
        self.event(&format!("hurt-armor {:08x}", f32_bits(v)))?;
        if self.flag("break_armor") {
            let id = a.catalog().id("minecraft:armor").unwrap();
            a.edit(id, |a, o| {
                a.set_base_value(number(&self.op["armor_after"]), o)
            })
            .unwrap()
            .unwrap();
        }
        Ok(())
    }
    fn has_resistance(&mut self) -> io::Result<bool> {
        self.event("has-resistance")?;
        Ok(self.flag("resistance"))
    }
    fn resistance_amplifier(&mut self) -> io::Result<i32> {
        self.event("effect")?;
        Ok(self.op["amplifier"].as_i64().unwrap() as i32)
    }
    fn award_resisted(&mut self, v: i32) -> io::Result<()> {
        self.event(&format!("stat resisted {v}"))
    }
    fn attacker(&mut self) -> io::Result<Option<()>> {
        panic!("player damage must not query attacker")
    }
    fn award_dealt_resisted(&mut self, _: i32) -> io::Result<()> {
        panic!("player damage must award own resisted stat")
    }
    fn damage_protection(&mut self, _: ()) -> io::Result<f32> {
        for slot in self.slots {
            self.event(&format!("slot {}", slot.as_u64().unwrap()))?;
        }
        Ok(0.0)
    }
    fn award_self(&mut self, s: DamageStat, v: i32) -> io::Result<()> {
        let n = match s {
            DamageStat::Absorbed => "absorbed",
            DamageStat::Taken => "taken",
            DamageStat::DealtAbsorbed => panic!("attacker stat"),
        };
        self.event(&format!("stat {n} {v}"))
    }
    fn award_attacker(&mut self, _: (), _: DamageStat, _: i32) -> io::Result<()> {
        panic!("attacker stat")
    }
    fn source_food_exhaustion(&mut self) -> io::Result<f32> {
        self.event("food-exhaustion")?;
        Ok(single(&self.op["food_exhaustion"]))
    }
    fn ability_invulnerable(&self) -> bool {
        self.flag("ability_invulnerable")
    }
    fn level_is_client(&mut self) -> io::Result<bool> {
        self.event("level")?;
        self.event("client")?;
        Ok(self.flag("client"))
    }
    fn record_damage(&mut self, v: f32) -> io::Result<()> {
        self.event("tracker")?;
        self.event(&format!("record {:08x}", f32_bits(v)))?;
        self.recorded = v;
        Ok(())
    }
    fn damage_event(&mut self) -> io::Result<()> {
        self.event("damage-event")
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    let ids: Value = serde_json::from_str(&fs::read_to_string(dir.join("ids.json"))?)?;
    let ids = PlayerFloatIds {
        health: ids["health"].as_u64().unwrap() as u8,
        absorption: ids["absorption"].as_u64().unwrap() as u8,
    };
    let slots: Value = serde_json::from_str(&fs::read_to_string(dir.join("slots.json"))?)?;
    let catalog = Arc::new(Catalog::from_json(&serde_json::from_str(
        &fs::read_to_string(dir.join("catalog.json"))?,
    )?)?);
    let keys = [
        "minecraft:armor",
        "minecraft:armor_toughness",
        "minecraft:max_health",
        "minecraft:max_absorption",
    ]
    .map(|k| catalog.id(k).unwrap());
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let mut attributes = catalog.create("minecraft:player").unwrap();
        for (i, key) in keys.iter().enumerate() {
            let a = &f["attributes"][i];
            let m = Modifier::new(
                "horizon:test",
                number(&a["amount"]),
                Operation::ALL[a["operation"].as_u64().unwrap() as usize],
            )?;
            attributes
                .edit(*key, |a, o| {
                    a.set_base_value(number(&f["attributes"][i]["base"]), o)?;
                    a.add_transient(m, o)
                })
                .unwrap()
                .unwrap();
        }
        let mut data = SyncedFloats::new(&[
            FloatValue {
                id: ids.health,
                value: single(&f["health"]),
            },
            FloatValue {
                id: ids.absorption,
                value: single(&f["absorption"]),
            },
        ])?;
        let mut food = FoodData {
            exhaustion: single(&f["exhaustion"]),
            ..FoodData::default()
        };
        let mut recorded = 0.0;
        for op in f["ops"].as_array().unwrap() {
            let mut env = Env {
                out: &mut out,
                op,
                slots: slots.as_array().unwrap(),
                calls: 0,
                recorded,
            };
            let result =
                ServerPlayerDamage::new(&mut attributes, &mut data, &mut food, ids, &mut env)?
                    .apply(single(&op["damage"]));
            writeln!(
                env.out,
                "result {}",
                if result.is_ok() { "ok" } else { "error" }
            )?;
            recorded = env.recorded;
            writeln!(
                env.out,
                "state {:08x} {:08x} {:08x} {:08x} {} {} {} {}",
                f32_bits(data.get(ids.health).unwrap()),
                f32_bits(data.get(ids.absorption).unwrap()),
                f32_bits(food.exhaustion),
                f32_bits(recorded),
                data.is_dirty(),
                data.item_dirty(ids.health).unwrap(),
                data.item_dirty(ids.absorption).unwrap(),
                env.calls
            )?;
            if env.flag("pack") {
                writeln!(env.out, "packed {}", values(data.take_dirty().as_deref()))?;
            }
            writeln!(
                env.out,
                "defaults {}",
                values(data.non_default().as_deref())
            )?;
        }
    }
    Ok(())
}
