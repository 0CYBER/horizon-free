use crate::support::common::f32_bits;
use horizon_world::living_hurt::{HurtState, HurtTag, HurtWorld};
use horizon_world::{
    food::Difficulty,
    player_hurt::{
        CausingEntity, PlayerHurtWorld, ServerPlayerHurtWorld, player_hurt_server,
        server_player_hurt_server,
    },
};
use serde_json::Value;
use std::{
    fs,
    io::{self, Write},
    path::Path,
};
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
}
impl Env<'_> {
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
        self.event("die")
    }
    fn hurt_sound(&mut self, _: &mut HurtState<()>) -> io::Result<()> {
        self.event("hurt-sound")
    }

    fn dead_or_dying(&mut self, _: &mut HurtState<()>) -> io::Result<bool> {
        self.event("dead")?;
        Ok(self.flag(if self.hit {
            "dead_after"
        } else {
            "dead_before"
        }))
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
        self.event("helmet")?;
        Ok(!self.flag("helmet"))
    }
    fn hurt_helmet(&mut self, _: &mut HurtState<()>, v: f32) -> io::Result<()> {
        self.event(&format!("hurt-helmet {:08x}", f32_bits(v)))
    }
    fn actually_hurt(&mut self, s: &mut HurtState<()>, v: f32) -> io::Result<()> {
        self.event(&format!("hurt {:08x} {}", f32_bits(v), state(s)))?;
        self.hit = true;
        self.recorded = v;
        if self.flag("mutate_hit") {
            s.cooldown = self.op["hit_cooldown"].as_i64().unwrap() as i32;
            s.last_hurt = single(&self.op["hit_last"]);
            s.hurt_duration = self.op["hit_duration"].as_i64().unwrap() as i32;
            s.hurt_time = self.op["hit_time"].as_i64().unwrap() as i32;
        }
        Ok(())
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
pub fn main() -> Result<(), Box<dyn std::error::Error>> {
    let arg = crate::args().nth(1).unwrap();
    let dir = Path::new(&arg);
    let mut out = io::BufWriter::new(fs::File::create(dir.join("rust.txt"))?);
    for line in fs::read_to_string(dir.join("inputs.txt"))?.lines() {
        let f: Value = serde_json::from_str(line)?;
        let mut s = HurtState {
            no_action_time: f["no_action"].as_i64().unwrap() as i32,
            cooldown: f["cooldown"].as_i64().unwrap() as i32,
            last_hurt: single(&f["last_hurt"]),
            hurt_duration: f["duration"].as_i64().unwrap() as i32,
            hurt_time: f["time"].as_i64().unwrap() as i32,
            last_source: f["source"].as_bool().unwrap().then_some(()),
            last_stamp: f["stamp"].as_i64().unwrap(),
        };
        let mut recorded = 0.0;
        for op in f["ops"].as_array().unwrap() {
            let mut e = Env {
                out: &mut out,
                op,
                calls: 0,
                hit: false,
                recorded,
                player: true,
                difficulty_calls: 0,
            };
            let result = if f["server"].as_bool().unwrap() {
                server_player_hurt_server(&mut s, &mut e, single(&op["damage"]))
            } else {
                player_hurt_server(&mut s, &mut e, single(&op["damage"]))
            };
            match result {
                Ok(v) => writeln!(e.out, "bool {v}\nresult ok")?,
                Err(_) => writeln!(e.out, "result error")?,
            };
            recorded = e.recorded;
            writeln!(
                e.out,
                "state {} {:08x} {} {}",
                state(&s),
                f32_bits(recorded),
                e.hit,
                e.calls
            )?;
        }
    }
    Ok(())
}
