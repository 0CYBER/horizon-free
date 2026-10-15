//! Example Horizon mod (`docs/mods.md`): coordinates, facing and the time of
//! day in the top-left corner, `/where` to put them in chat and `/coords`
//! to hide or show the panel.
use horizon_mod_sdk::{
    Mod, Registration, chat,
    game::{self, Clock, Player},
    hud::{self, Corner, Line},
};

struct CoordsHud {
    shown: bool,
    /// The panel last sent, so an unchanged one costs no call.
    last: Vec<String>,
}

/// Direction.fromYRot: yaw 0 faces south, 90 west.
fn facing(yaw: f32) -> &'static str {
    let i = (yaw / 90.0 + 0.5).floor() as i32 & 3;
    ["South (+Z)", "West (-X)", "North (-Z)", "East (+X)"][i as usize]
}

/// `Day 3, 14:05`: day time 0 is 6:00 on day 1.
fn time_of_day(clock: &Clock) -> String {
    let day = clock.day_time / 24000 + 1;
    let ticks = clock.day_time % 24000;
    let hours = (ticks / 1000 + 6) % 24;
    let minutes = ticks % 1000 * 60 / 1000;
    format!("Day {day}, {hours:02}:{minutes:02}")
}

fn lines(p: &Player, clock: Option<&Clock>) -> Vec<String> {
    let mut lines = vec![
        format!("XYZ {:.1} / {:.1} / {:.1}", p.x, p.y, p.z),
        format!("Facing {}", facing(p.yaw)),
    ];
    lines.extend(clock.map(time_of_day));
    lines
}

impl Mod for CoordsHud {
    fn init() -> Result<(Self, Registration), String> {
        let hooks = Registration::new()
            .command("where")
            .command("coords")
            .on_tick();
        let state = CoordsHud {
            shown: true,
            last: Vec::new(),
        };
        Ok((state, hooks))
    }

    fn tick(&mut self, _tick: u64) {
        let Some(p) = game::get_player().filter(|_| self.shown) else {
            if !self.last.is_empty() {
                hud::clear();
                self.last.clear();
            }
            return;
        };
        let now = lines(&p, game::get_clock().as_ref());
        if now != self.last {
            let panel: Vec<Line> = now
                .iter()
                .map(|text| Line {
                    text: text.clone(),
                    color: 0xFFFFFF,
                })
                .collect();
            hud::set_panel(Corner::TopLeft, &panel);
            self.last = now;
        }
    }

    fn command(&mut self, name: &str, _args: &str) -> Result<Option<String>, String> {
        match name {
            "coords" => {
                self.shown = !self.shown;
                let state = if self.shown { "shown" } else { "hidden" };
                Ok(Some(format!("Coordinates panel {state}")))
            }
            _ => {
                let p = game::get_player().ok_or("Not in a world")?;
                chat::show(&format!(
                    "{:.0}, {:.0}, {:.0} in {}",
                    p.x.floor(),
                    p.y.floor(),
                    p.z.floor(),
                    p.dimension
                ));
                Ok(None)
            }
        }
    }
}

horizon_mod_sdk::export_mod!(CoordsHud);

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn facing_follows_vanilla_yaw() {
        assert_eq!(facing(0.0), "South (+Z)");
        assert_eq!(facing(90.0), "West (-X)");
        assert_eq!(facing(-180.0), "North (-Z)");
        assert_eq!(facing(-90.0), "East (+X)");
        assert_eq!(facing(314.0), "East (+X)");
    }

    #[test]
    fn day_time_reads_as_a_clock() {
        let at = |day_time| {
            time_of_day(&Clock {
                game_time: 0,
                day_time,
            })
        };
        assert_eq!(at(0), "Day 1, 06:00");
        assert_eq!(at(18000), "Day 1, 00:00");
        assert_eq!(at(24000 * 2 + 8083), "Day 3, 14:04");
    }
}
