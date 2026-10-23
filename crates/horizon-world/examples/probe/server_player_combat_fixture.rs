use crate::support::common::f32_bits;
use horizon_world::damage_immunity::{DamageRule, ImmunityTag, ImmunityWorld};
use horizon_world::living_hurt::{HurtState, HurtTag, HurtWorld};
use horizon_world::{
    attribute_health::HealthAttributes,
    attributes::{
        Modifier, Operation,
        entity::{Catalog, EntityAttributes},
        live::LiveAttributes,
    },
    combat_rules::ArmorWorld,
    damage_reduction::Bypass,
    food::FoodData,
    living_damage::DamageStat,
    player_health::PlayerFloatIds,
    server_player_combat::{CombatEnvironment, ServerPlayerCombat},
    server_player_damage::ServerDamageEnvironment,
    synced_floats::{FloatValue, Observer, SyncedFloats},
};
use horizon_world::{
    food::Difficulty,
    player_hurt::{CausingEntity, PlayerHurtWorld, ServerPlayerHurtWorld},
};
use serde_json::Value;
use std::sync::Arc;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
// Run the identical native combat corpus through both attribute stores.
trait FixtureAttributes: HealthAttributes {
    fn create(catalog: Arc<Catalog>) -> Self;
    fn configure(&mut self, name: &str, base: f64, modifier: Option<Modifier>);
}
impl FixtureAttributes for EntityAttributes {
    fn create(catalog: Arc<Catalog>) -> Self {
        catalog.create("minecraft:player").unwrap()
    }
    fn configure(&mut self, name: &str, base: f64, modifier: Option<Modifier>) {
        let key = self.catalog().id(name).unwrap();
        self.edit(key, |a, o| {
            a.set_base_value(base, o)?;
            if let Some(m) = modifier {
                a.add_transient(m, o)?;
            }
            Ok::<_, horizon_world::attributes::AttributeError<std::convert::Infallible>>(())
        })
        .unwrap()
        .unwrap();
    }
}
type Live = LiveAttributes<Box<dyn FnMut() -> i32>>;
impl FixtureAttributes for Live {
    fn create(catalog: Arc<Catalog>) -> Self {
        let hashes: Vec<_> = catalog
            .attributes()
            .enumerate()
            .map(|(i, _)| i as i32)
            .collect();
        let mut next = 0i32;
        Self::new(
            catalog,
            "minecraft:player",
            &hashes,
            Box::new(move || {
                next = next.wrapping_add(1);
                next
            }),
        )
        .unwrap()
    }
    fn configure(&mut self, name: &str, base: f64, modifier: Option<Modifier>) {
        let handle = self.get_instance(self.key(name)).unwrap().unwrap();
        self.edit(&handle, |a, o| {
            a.set_base_value(base, o)?;
            if let Some(m) = modifier {
                a.add_transient(m, o)?;
            }
            Ok(())
        })
        .unwrap();
    }
}
fn single(v: &Value) -> f32 {
    f32::from_bits(u32::from_str_radix(v.as_str().unwrap(), 16).unwrap())
}
fn state(s: &HurtState<()>) -> String {
    format!(
        "{} {} {:08x} {} {} {} {}",
        s.no_action_time,
        s.cooldown,
        f32_bits(s.last_hurt),
        s.hurt_duration,
        s.hurt_time,
        s.last_source.is_some(),
        s.last_stamp
    )
}
struct Env<'a> {
    out: &'a mut dyn Write,
    op: &'a Value,
    calls: i64,
    hit: bool,
    recorded: f32,
    player: bool,
    difficulty_calls: usize,
    slots: &'a [Value],
    head: u64,
    dead: bool,
}
impl Env<'_> {
    fn query(&mut self, event: &str, key: &str) -> io::Result<bool> {
        self.event(event)?;
        Ok(self.flag(key))
    }

    fn flag(&self, n: &str) -> bool {
        self.op[n].as_bool().unwrap()
    }
    fn event(&mut self, s: &str) -> io::Result<()> {
        writeln!(self.out, "{s}")?;
        let n = self.calls;
        self.calls += 1;
        if n == self.op["fail"].as_i64().unwrap() {
            Err(io::Error::other("injected hurt callback failure"))
        } else {
            Ok(())
        }
    }
}
impl HurtWorld for Env<'_> {
    type Error = io::Error;
    type Source = ();
    type UseItem = bool;
    type Blocking = ();
    type Sound = ();
    fn source(&self) {}
    fn is_server_player(&self) -> bool {
        self.player
    }
    fn invulnerable(&mut self, _: &mut HurtState<()>) -> io::Result<bool> {
        self.event("invulnerable")?;
        Ok(self.flag("invulnerable"))
    }
    fn fire_resistance(&mut self, _: &mut HurtState<()>) -> io::Result<bool> {
        self.event("fire-resistance")?;
        Ok(self.flag("fire_resistance"))
    }
    fn sleeping(&mut self, _: &mut HurtState<()>) -> io::Result<bool> {
        self.event("sleeping")?;
        Ok(self.flag("sleeping"))
    }
    fn freeze_hurts_extra(&mut self, _: &mut HurtState<()>) -> io::Result<bool> {
        self.event("freeze-extra")?;
        Ok(self.flag("freeze_extra"))
    }
    fn stop_sleeping(&mut self, _: &mut HurtState<()>) -> io::Result<()> {
        self.event("wake")
    }
    fn resolve_mob(&mut self, _: &mut HurtState<()>) -> io::Result<()> {
        self.event("resolve-mob")
    }
    fn resolve_player(&mut self, _: &mut HurtState<()>) -> io::Result<()> {
        self.event("resolve-player")
    }
    fn broadcast_damage(&mut self, _: &mut HurtState<()>) -> io::Result<()> {
        self.event("broadcast")
    }
    fn mark_hurt(&mut self, _: &mut HurtState<()>) -> io::Result<()> {
        self.event("mark")
    }
    fn secondary_sound(&mut self, _: &mut HurtState<()>) -> io::Result<()> {
        self.event("thorns")
    }
    fn die(&mut self, _: &mut HurtState<()>) -> io::Result<()> {
        self.event("die")?;
        self.dead = true;
        Ok(())
    }
    fn hurt_sound(&mut self, _: &mut HurtState<()>) -> io::Result<()> {
        self.event("hurt-sound")
    }

    fn dead_or_dying(&mut self, _: &mut HurtState<()>) -> io::Result<bool> {
        panic!("combat binding supplies stored health/death predicate")
    }
    fn tag(&mut self, _: &mut HurtState<()>, tag: HurtTag) -> io::Result<bool> {
        let n = match tag {
            HurtTag::Fire => "fire",
            HurtTag::Freezing => "freezing",
            HurtTag::DamagesHelmet => "damages_helmet",
            HurtTag::BypassesCooldown => "bypasses_cooldown",
            HurtTag::NoImpact => "no_impact",
            HurtTag::NoKnockback => "no_knockback",
        };
        self.event(&format!("tag {n}"))?;
        Ok(self.flag(n))
    }
    fn use_item(&mut self, _: &mut HurtState<()>) -> io::Result<bool> {
        self.event("use-item")?;
        Ok(self.flag("component"))
    }
    fn apply_blocking(&mut self, _: &mut HurtState<()>, v: f32) -> io::Result<f32> {
        self.event(&format!("blocking {:08x}", f32_bits(v)))?;
        Ok(single(&self.op["blocked"]))
    }
    fn helmet_empty(&mut self, _: &mut HurtState<()>) -> io::Result<bool> {
        self.event(&format!("slot {}", self.head))?;
        Ok(!self.flag("helmet"))
    }
    fn hurt_helmet(&mut self, _: &mut HurtState<()>, v: f32) -> io::Result<()> {
        self.event(&format!("hurt-helmet {:08x}", f32_bits(v)))
    }
    fn actually_hurt(&mut self, _s: &mut HurtState<()>, _v: f32) -> io::Result<()> {
        panic!("combat binding supplies actual inner damage")
    }
    fn blocking_component(&mut self, v: bool) -> Option<()> {
        v.then_some(())
    }
    fn on_blocked(&mut self, _: &mut HurtState<()>, _: ()) -> io::Result<()> {
        Ok(())
    }
    fn knockback(&mut self, _: &mut HurtState<()>, v: f32, b: bool) -> io::Result<()> {
        self.event(&format!("knockback {:08x} {b}", f32_bits(v)))
    }
    fn death_protection(&mut self, _: &mut HurtState<()>) -> io::Result<bool> {
        self.event("tag bypasses_invulnerability")?;
        if !self.flag("bypasses_invulnerability") {
            self.event("hand 0")?;
            self.event("hand 1")?;
        }
        Ok(false)
    }
    fn death_sound(&mut self, _: &mut HurtState<()>) -> io::Result<()> {
        self.event("death-sound")
    }
    fn make_sound(&mut self, _: &mut HurtState<()>, _: ()) -> io::Result<()> {
        self.event("make-sound")
    }
    fn game_time(&mut self, _: &mut HurtState<()>) -> io::Result<i64> {
        self.event("level")?;
        self.event("game-time")?;
        Ok(self.op["game_time"].as_i64().unwrap())
    }
    fn effects_on_hurt(&mut self, _: &mut HurtState<()>, v: f32) -> io::Result<()> {
        self.event("effects")?;
        for i in 0..self.op["effects"].as_u64().unwrap() {
            self.event(&format!("effect {i} {:08x}", f32_bits(v)))?;
        }
        Ok(())
    }
    fn player_hurt_criterion(
        &mut self,
        _: &mut HurtState<()>,
        _: f32,
        _: f32,
        _: bool,
    ) -> io::Result<()> {
        self.event("criterion")
    }
    fn award_blocked(&mut self, _: &mut HurtState<()>, v: i32) -> io::Result<()> {
        self.event(&format!("blocked-stat {v}"))
    }
    fn attacker_criterion(
        &mut self,
        _: &mut HurtState<()>,
        _: f32,
        _: f32,
        _: bool,
    ) -> io::Result<()> {
        self.event("attacker")
    }
}
impl PlayerHurtWorld for Env<'_> {
    fn ability_invulnerable(&self) -> bool {
        self.flag("ability_invulnerable")
    }
    fn bypasses_invulnerability(&mut self, _: &mut HurtState<()>) -> io::Result<bool> {
        self.event("tag bypasses_invulnerability")?;
        Ok(self.flag("bypasses_invulnerability"))
    }
    fn remove_shoulders(&mut self, _: &mut HurtState<()>) -> io::Result<()> {
        self.event("shoulders")
    }
    fn scales_with_difficulty(&mut self, _: &mut HurtState<()>) -> io::Result<bool> {
        self.event("scales")?;
        Ok(self.flag("scales"))
    }
    fn difficulty(&mut self, _: &mut HurtState<()>) -> io::Result<Difficulty> {
        self.event("difficulty")?;
        let v = self.op["difficulties"][self.difficulty_calls]
            .as_u64()
            .unwrap();
        self.difficulty_calls += 1;
        Ok([
            Difficulty::Peaceful,
            Difficulty::Easy,
            Difficulty::Normal,
            Difficulty::Hard,
        ][v as usize])
    }
}
impl ServerPlayerHurtWorld for Env<'_> {
    type Player = ();
    type Arrow = ();
    fn causing_entity(&mut self, _: &mut HurtState<()>) -> io::Result<CausingEntity<(), ()>> {
        self.event("attacker")?;
        Ok(match self.op["attacker"].as_u64().unwrap() {
            0 | 1 => CausingEntity::Other,
            2 => CausingEntity::Player(()),
            3 => CausingEntity::Arrow(()),
            _ => panic!("attacker"),
        })
    }
    fn arrow_player_owner(&mut self, _: &mut HurtState<()>, _: ()) -> io::Result<Option<()>> {
        self.event("owner")?;
        Ok((self.op["owner"] == 1).then_some(()))
    }
    fn can_harm_player(&mut self, _: &mut HurtState<()>, _: ()) -> io::Result<bool> {
        self.event("can-harm")?;
        Ok(self.flag("can_harm"))
    }
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
impl<A: FixtureAttributes> ServerDamageEnvironment<A> for Env<'_> {
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
        self.event(&format!("tag bypass_{n}"))?;
        Ok(self.flag(&format!("bypass_{n}")))
    }
    fn hurt_armor(&mut self, a: &mut A, v: f32) -> io::Result<()> {
        self.event(&format!("hurt-armor {:08x}", f32_bits(v)))?;
        if self.flag("break_armor") {
            a.configure("minecraft:armor", number(&self.op["armor_after"]), None);
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
        self.hit = true;
        Ok(())
    }
    fn damage_event(&mut self) -> io::Result<()> {
        self.event("damage-event")
    }
}
impl<A: FixtureAttributes> CombatEnvironment<A> for Env<'_> {
    fn dead_flag(&self) -> bool {
        self.dead
    }
}
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    if crate::args().nth(2).as_deref() == Some("--live") {
        run::<Live>(dir, "rust-live.txt")
    } else {
        run::<EntityAttributes>(dir, "rust.txt")
    }
}
fn run<A: FixtureAttributes>(dir: &Path, output: &str) -> Result<(), Box<dyn std::error::Error>> {
    let ids: Value = serde_json::from_str(&fs::read_to_string(dir.join("ids.json"))?)?;
    let ids = PlayerFloatIds {
        health: ids["health"].as_u64().unwrap() as u8,
        absorption: ids["absorption"].as_u64().unwrap() as u8,
    };
    let slots: Value = serde_json::from_str(&fs::read_to_string(dir.join("slots.json"))?)?;
    let head: u64 = serde_json::from_str(&fs::read_to_string(dir.join("head.json"))?)?;
    let catalog = Arc::new(Catalog::from_json(&serde_json::from_str(
        &fs::read_to_string(dir.join("catalog.json"))?,
    )?)?);
    let keys = [
        "minecraft:armor",
        "minecraft:armor_toughness",
        "minecraft:max_health",
        "minecraft:max_absorption",
    ];
    let mut out = io::BufWriter::new(fs::File::create(dir.join(output))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let mut attributes = A::create(catalog.clone());
        for (i, key) in keys.iter().enumerate() {
            let a = &f["attributes"][i];
            let m = Modifier::new(
                "horizon:test",
                number(&a["amount"]),
                Operation::ALL[a["operation"].as_u64().unwrap() as usize],
            )?;
            attributes.configure(key, number(&f["attributes"][i]["base"]), Some(m));
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
        let mut dead = f["dead"].as_bool().unwrap();
        let mut s = HurtState {
            no_action_time: f["no_action"].as_i64().unwrap() as i32,
            cooldown: f["cooldown"].as_i64().unwrap() as i32,
            last_hurt: single(&f["last_hurt"]),
            hurt_duration: f["duration"].as_i64().unwrap() as i32,
            hurt_time: f["time"].as_i64().unwrap() as i32,
            last_source: f["source"].as_bool().unwrap().then_some(()),
            last_stamp: f["stamp"].as_i64().unwrap(),
        };
        for op in f["ops"].as_array().unwrap() {
            if op["reset_cooldown"].as_bool().unwrap_or(false) {
                s.cooldown = 0;
            }
            let mut e = Env {
                out: &mut out,
                op,
                calls: 0,
                hit: false,
                recorded,
                player: true,
                difficulty_calls: 0,
                slots: slots.as_array().unwrap(),
                head,
                dead,
            };
            let result =
                ServerPlayerCombat::new(&mut attributes, &mut data, &mut food, ids, &mut e)?
                    .hurt(&mut s, single(&op["damage"]));
            match result {
                Ok(v) => writeln!(e.out, "bool {v}\nresult ok")?,
                Err(_) => writeln!(e.out, "result error")?,
            };
            recorded = e.recorded;
            dead = e.dead;
            writeln!(
                e.out,
                "state {} {:08x} {} {}",
                state(&s),
                f32_bits(recorded),
                e.hit,
                e.calls
            )?;
            writeln!(
                e.out,
                "storage {:08x} {:08x} {:08x} {} {} {} {}",
                f32_bits(data.get(ids.health).unwrap()),
                f32_bits(data.get(ids.absorption).unwrap()),
                f32_bits(food.exhaustion),
                data.is_dirty(),
                data.item_dirty(ids.health).unwrap(),
                data.item_dirty(ids.absorption).unwrap(),
                dead
            )?;
            if e.flag("pack") {
                writeln!(e.out, "packed {}", values(data.take_dirty().as_deref()))?;
            }
            writeln!(e.out, "defaults {}", values(data.non_default().as_deref()))?;
        }
    }
    Ok(())
}
impl ImmunityWorld for Env<'_> {
    type Error = io::Error;
    fn removed(&self) -> bool {
        self.flag("removed")
    }
    fn permanently_invulnerable(&mut self) -> io::Result<bool> {
        self.query("permanent", "permanent")
    }
    fn temporarily_invulnerable(&mut self) -> io::Result<bool> {
        self.query("temporary", "temporary")
    }
    fn tag(&mut self, t: ImmunityTag) -> io::Result<bool> {
        let n = match t {
            ImmunityTag::BypassesInvulnerability => "bypasses_invulnerability",
            ImmunityTag::Fire => "fire",
            ImmunityTag::Fall => "fall",
            ImmunityTag::Drowning => "drowning",
            ImmunityTag::Freezing => "freezing",
        };
        self.query(&format!("tag {n}"), n)
    }
    fn creative_source(&mut self) -> io::Result<bool> {
        self.query("creative", "creative")
    }
    fn fire_immune(&mut self) -> io::Result<bool> {
        self.query("fire-immune", "fire_immune")
    }
    fn fall_immune(&mut self) -> io::Result<bool> {
        self.query("fall-immune", "fall_immune")
    }
    fn enchantment_immunity(&mut self) -> io::Result<bool> {
        for s in self.slots {
            self.event(&format!("slot {}", s.as_u64().unwrap()))?;
        }
        Ok(false)
    }
    fn damage_rule(&mut self, r: DamageRule) -> io::Result<bool> {
        let n = match r {
            DamageRule::Drowning => "drowning",
            DamageRule::Fall => "fall",
            DamageRule::Fire => "fire",
            DamageRule::Freeze => "freeze",
        };
        self.event("rules")?;
        self.query(&format!("rule {n}"), &format!("rule_{n}"))
    }
    fn changing_dimension(&mut self) -> io::Result<bool> {
        self.query("changing", "changing")
    }
    fn ender_pearl(&mut self) -> io::Result<bool> {
        self.query("pearl", "pearl")
    }
    fn client_loaded(&mut self) -> io::Result<bool> {
        self.query("loaded", "loaded")
    }
}
