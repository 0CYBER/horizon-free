//! Example Horizon mod (`docs/mods.md`): `/missile` turns you into a guided
//! missile 60 blocks up. You fly along your own look, so the mouse steers;
//! on a block or a mob a stack of primed TNT goes off, and you are put back
//! where you launched. Built from vanilla commands (capability `command`),
//! so it needs cheats.
use horizon_mod_sdk::{
    Mod, Registration,
    command::run,
    game::{self, Player},
    hud::{self, Corner, Line},
};

/// Blocks per tick (a fired arrow does about 3).
const SPEED: f64 = 1.5;
const ALTITUDE: i32 = 60;
/// Ticks of flight before the missile detonates on its own.
const FUEL: u32 = 30 * 20;
/// Ticks to watch the blast before going back.
const AFTERMATH: u32 = 40;
/// Primed TNT per blast: the stack's explosions add up.
const WARHEAD: usize = 4;
/// What the impact checks print; the mod hears it through the chat hook.
const IMPACT: &str = "Predator: impact";

enum State {
    Idle,
    Flying { from: Player, ticks: u32 },
    Aftermath { from: Player, ticks: u32 },
}

struct Predator {
    state: State,
}

fn launch() {
    run("gamerule send_command_feedback false");
    run("execute if entity @s[gamemode=creative] run tag @s add pm_creative");
    run("gamemode spectator");
    run(&format!("tp @s ~ ~{ALTITUDE} ~ ~ 60"));
    run("effect give @s night_vision 60 0 true");
    // The thermal camera: every mob outlined.
    run("effect give @e[type=!player,distance=..256] glowing 60 0 true");
}

/// One tick of flight: stop at the surface (the next step would enter a
/// block or a mob), else move one step along the look.
fn fly() {
    run(&format!(
        "execute at @s unless block ^ ^ ^{SPEED} #minecraft:air run tellraw @s \"{IMPACT}\""
    ));
    run(&format!(
        "execute at @s positioned ^ ^ ^{SPEED} if entity @e[type=!player,distance=..2] run tellraw @s \"{IMPACT}\""
    ));
    run(&format!(
        "execute at @s if block ^ ^ ^{SPEED} #minecraft:air run tp @s ^ ^ ^{SPEED}"
    ));
}

fn detonate() {
    for _ in 0..WARHEAD {
        run("execute at @s run summon tnt ^ ^ ^1 {fuse:0s}");
    }
}

fn land(from: &Player) {
    run(&format!(
        "tp @s {} {} {} {} {}",
        from.x, from.y, from.z, from.yaw, from.pitch
    ));
    run("execute if entity @s[tag=pm_creative] run gamemode creative");
    run("execute unless entity @s[tag=pm_creative] run gamemode survival");
    run("execute if entity @s[tag=pm_creative] run tag @s remove pm_creative");
    run("effect clear @s night_vision");
    // shortcut: assumes feedback was on before launch, as in a new world.
    run("gamerule send_command_feedback true");
}

fn panel(p: &Player, ticks: u32) -> Vec<Line> {
    let green = 0x7CFF7C;
    let left = (FUEL - ticks.min(FUEL)) / 20;
    [
        "PREDATOR".to_string(),
        format!("ALT {:.0}", p.y),
        format!("FUEL {left}s"),
        "Mouse to steer, /missile to detonate".to_string(),
    ]
    .into_iter()
    .map(|text| Line { text, color: green })
    .collect()
}

impl Predator {
    fn impact(&mut self) {
        if let State::Flying { from, .. } = &self.state {
            detonate();
            self.state = State::Aftermath {
                from: from.clone(),
                ticks: 0,
            };
        }
    }
}

impl Mod for Predator {
    fn init() -> Result<(Self, Registration), String> {
        let hooks = Registration::new().command("missile").on_tick().on_chat();
        Ok((Predator { state: State::Idle }, hooks))
    }

    fn tick(&mut self, _tick: u64) {
        match &mut self.state {
            State::Idle => {}
            State::Flying { ticks, .. } => {
                *ticks += 1;
                let ticks = *ticks;
                if ticks >= FUEL {
                    return self.impact();
                }
                // The first tick waits for the launch teleport.
                if ticks > 1 {
                    fly();
                }
                if let Some(p) = game::get_player() {
                    hud::set_panel(Corner::TopLeft, &panel(&p, ticks));
                }
            }
            State::Aftermath { from, ticks } => {
                *ticks += 1;
                if *ticks >= AFTERMATH {
                    land(from);
                    hud::clear();
                    self.state = State::Idle;
                }
            }
        }
    }

    fn chat(&mut self, text: &str) {
        if text.contains(IMPACT) {
            self.impact();
        }
    }

    fn command(&mut self, _name: &str, _args: &str) -> Result<Option<String>, String> {
        match self.state {
            State::Idle => {
                let from = game::get_player().ok_or("Not in a world")?;
                launch();
                self.state = State::Flying { from, ticks: 0 };
                Ok(Some("Missile away".into()))
            }
            State::Flying { .. } => {
                self.impact();
                Ok(None)
            }
            State::Aftermath { .. } => Err("Wait for the blast to clear".into()),
        }
    }
}

horizon_mod_sdk::export_mod!(Predator);
