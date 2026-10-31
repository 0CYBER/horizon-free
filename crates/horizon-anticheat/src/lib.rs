//! The dedicated server's anticheat (docs/anticheat.md), without the game: the `[anticheat]` section of
//! `server.toml`, each player's violation levels, and the rules of the checks. The server's hub
//! (`horizon_client::lan`) feeds it what players send and carries out what it decides.
//!
//! It is off unless the owner turns it on. A check that fails *flags* the player: its violation level
//! (VL) rises by the flag's weight and falls back by `decay` a second. From `setback` on, the move or
//! action is cancelled (the player is sent back where the server last had them); from `alert` on,
//! online operators and the console are told; from `kick` on, the player is disconnected. Nothing bans.
//! Any threshold at 0 turns that action off.
//!
//! The checks are written from vanilla 26.3's own rules and from what cheats are publicly described to
//! do, never from another anticheat's code.

use serde::{Deserialize, Serialize};

pub mod prediction;
pub mod timer;

/// What one check's violation level leads to.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Actions {
    pub setback: f32,
    pub alert: f32,
    pub kick: f32,
    /// Violation level lost each second.
    pub decay: f32,
}

macro_rules! checks {
    ($(
        $(#[$doc:meta])*
        $field:ident: $Check:ident => $Config:ident [$setback:expr, $alert:expr, $kick:expr, $decay:expr] {
            $( $(#[$xdoc:meta])* $extra:ident: $ty:ty = $default:expr, )*
        }
    )*) => {
        /// Every check, in the order of the players' violation levels.
        #[derive(Clone, Copy, Debug, PartialEq, Eq)]
        pub enum Check { $($Check,)* }

        impl Check {
            pub const ALL: &[Check] = &[$(Check::$Check,)*];
            /// Its name in `server.toml` and in alerts.
            pub fn name(self) -> &'static str {
                match self { $(Check::$Check => stringify!($field),)* }
            }
        }

        $(
            $(#[$doc])*
            #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
            #[serde(default, deny_unknown_fields)]
            pub struct $Config {
                pub enabled: bool,
                pub setback: f32,
                pub alert: f32,
                pub kick: f32,
                pub decay: f32,
                $( $(#[$xdoc])* pub $extra: $ty, )*
            }

            impl Default for $Config {
                fn default() -> Self {
                    Self {
                        enabled: true,
                        setback: $setback,
                        alert: $alert,
                        kick: $kick,
                        decay: $decay,
                        $($extra: $default,)*
                    }
                }
            }
        )*

        /// `server.toml`'s `[anticheat]`: off by default.
        #[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
        #[serde(default, deny_unknown_fields)]
        pub struct Config {
            pub enabled: bool,
            /// Tell online operators of an alert in their chat (the console and the log always are).
            pub alerts: bool,
            /// The permission level a player needs to see alerts.
            pub alert_level: u8,
            /// Every flag, one line each, relative to the server's folder; empty: no file.
            pub log: String,
            $(pub $field: $Config,)*
        }

        impl Default for Config {
            fn default() -> Self {
                Self {
                    enabled: false,
                    alerts: true,
                    alert_level: 2,
                    log: "logs/anticheat.log".into(),
                    $($field: $Config::default(),)*
                }
            }
        }

        impl Config {
            /// Whether `check` runs: the anticheat and the check both on.
            pub fn runs(&self, check: Check) -> bool {
                self.enabled && match check { $(Check::$Check => self.$field.enabled,)* }
            }

            pub fn actions(&self, check: Check) -> Actions {
                match check {
                    $(Check::$Check => {
                        let c = &self.$field;
                        Actions { setback: c.setback, alert: c.alert, kick: c.kick, decay: c.decay }
                    })*
                }
            }
        }
    };
}

checks! {
    /// Timer: more client ticks than the server's clock allows (a sped-up game). Vanilla's client ends
    /// every tick with ServerboundClientTickEndPacket, at the server's tick rate; each one spends a tick's
    /// time from a balance that wall-clock time fills, capped so a pause can't be saved up for a burst.
    timer: Timer => TimerConfig [2.0, 5.0, 20.0, 0.02] {
        /// How far ahead of the clock, in milliseconds, a client may get before it is flagged.
        tolerance_ms: f64 = 300.0,
        /// The most time, in milliseconds, a quiet connection banks for the ticks that follow (a lag
        /// spike's backlog arrives at once).
        max_lag_ms: f64 = 1000.0,
    }

    /// Ground: a move that says it ends on the ground with nothing there to stand on: no-fall (claiming
    /// the ground to skip fall damage) and walking on water. The claim is always dropped, so vanilla's
    /// fall damage follows; from `setback` on the player is also sent back.
    ground: Ground => GroundConfig [5.0, 15.0, 60.0, 2.0] {}

    /// Prediction: each move played again from the client's own inputs with vanilla's movement code,
    /// and compared (speed, step, fly, jesus...). Each player's client ticks are recorded
    /// (`prediction::Record`), with a ping a tick to place what the server changes, and played again
    /// (horizon-client's `lan::prediction`). A move that lands farther than `tolerance` from the
    /// replay flags, by how far it lands (one `tolerance` off weighs 1, at most 5 a tick), and the
    /// setback puts the player back where its moves last agreed. A tick near what the replay can't
    /// play yet (another entity, a block change or abilities the client hadn't seen, a moving piston,
    /// a chunk the client doesn't have, a riptide launch) is excused, and so are the ticks after it
    /// until the moves agree again.
    prediction: Prediction => PredictionConfig [5.0, 15.0, 60.0, 2.0] {
        /// How far, in blocks, a move may land from where the replay puts it.
        tolerance: f64 = 0.03,
    }
}

/// How many checks there are.
pub const CHECKS: usize = Check::ALL.len();

impl Config {
    /// The `[anticheat]` section of a whole `server.toml`; the default (off) when it has none.
    pub fn from_server_toml(text: &str) -> Result<Self, String> {
        #[derive(Deserialize)]
        struct File {
            #[serde(default)]
            anticheat: Config,
        }
        // Other sections are the supervisor's to check: only this one is read here.
        let mut table: toml::Table = toml::from_str(text).map_err(|e| e.to_string())?;
        let mut only = toml::Table::new();
        if let Some(section) = table.remove("anticheat") {
            only.insert("anticheat".into(), section);
        }
        let file: File = only
            .try_into()
            .map_err(|e: toml::de::Error| format!("[anticheat]: {e}"))?;
        file.anticheat.check()?;
        Ok(file.anticheat)
    }

    /// Thresholds can't be negative.
    pub fn check(&self) -> Result<(), String> {
        for &c in Check::ALL {
            let a = self.actions(c);
            if [a.setback, a.alert, a.kick, a.decay]
                .iter()
                .any(|v| !v.is_finite() || *v < 0.0)
            {
                return Err(format!(
                    "anticheat.{}: setback, alert, kick and decay are numbers from 0 up",
                    c.name()
                ));
            }
        }
        let tolerance = self.prediction.tolerance;
        if !tolerance.is_finite() || tolerance <= 0.0 {
            return Err("anticheat.prediction: tolerance is a number of blocks above 0".into());
        }
        Ok(())
    }
}

/// What a flag leads to.
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Verdict {
    /// The check's violation level after the flag.
    pub vl: f32,
    /// Cancel the move or action.
    pub setback: bool,
    /// Tell operators and the console (at most once a second per check).
    pub alert: bool,
    pub kick: bool,
}

/// One player's anticheat state. Clean players cost nothing: levels only change when a check flags.
#[derive(Clone, Debug, Default)]
pub struct Player {
    vl: [f32; CHECKS],
    /// The server tick of each check's last flag (its decay runs from there).
    flagged: [u64; CHECKS],
    /// The server tick of each check's last alert.
    alerted: [Option<u64>; CHECKS],
    pub timer: timer::Balance,
    /// The server tick and height where the ground last held the player.
    pub ground: Option<(u64, f64)>,
    /// The client ticks the prediction plays again (only while it runs).
    pub record: Option<Box<prediction::Record>>,
}

impl Player {
    /// The check's violation level at `tick`, with its decay since the last flag.
    pub fn vl(&self, config: &Config, check: Check, tick: u64) -> f32 {
        let i = check as usize;
        let seconds = tick.saturating_sub(self.flagged[i]) as f32 / 20.0;
        (self.vl[i] - config.actions(check).decay * seconds).max(0.0)
    }

    /// `check` failed with this weight at server tick `tick`.
    pub fn flag(&mut self, config: &Config, check: Check, weight: f32, tick: u64) -> Verdict {
        let i = check as usize;
        let a = config.actions(check);
        let vl = self.vl(config, check, tick) + weight;
        self.vl[i] = vl;
        self.flagged[i] = tick;
        let reached = |threshold: f32| threshold > 0.0 && vl >= threshold;
        let alert = reached(a.alert) && self.alerted[i].is_none_or(|t| tick >= t + 20);
        if alert {
            self.alerted[i] = Some(tick);
        }
        Verdict {
            vl,
            setback: reached(a.setback),
            alert,
            kick: reached(a.kick),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn on() -> Config {
        Config {
            enabled: true,
            ..Config::default()
        }
    }

    #[test]
    fn off_by_default_and_without_a_section() {
        assert!(!Config::default().enabled);
        let c = Config::from_server_toml("[server]\nport = 25565\n").unwrap();
        assert_eq!(c, Config::default());
        assert!(!c.runs(Check::Timer));
        assert!(!c.runs(Check::Ground));
    }

    #[test]
    fn reads_its_section() {
        let text = "[server]\nport = 1\n[anticheat]\nenabled = true\n[anticheat.timer]\nkick = 0\ntolerance_ms = 500\n";
        let c = Config::from_server_toml(text).unwrap();
        assert!(c.runs(Check::Timer));
        assert_eq!(c.timer.kick, 0.0);
        assert_eq!(c.timer.tolerance_ms, 500.0);
        assert_eq!(c.timer.alert, TimerConfig::default().alert);
        let zero = "[anticheat]\n[anticheat.prediction]\ntolerance = 0\n";
        assert!(Config::from_server_toml(zero).is_err());
        let off = "[anticheat]\nenabled = true\n[anticheat.timer]\nenabled = false\n";
        assert!(!Config::from_server_toml(off).unwrap().runs(Check::Timer));
    }

    #[test]
    fn refuses_unknown_keys_and_negative_levels() {
        assert!(Config::from_server_toml("[anticheat]\nenable = true\n").is_err());
        assert!(Config::from_server_toml("[anticheat.timer]\nkick = -1\n").is_err());
    }

    #[test]
    fn levels_rise_decay_and_act_in_order() {
        let mut c = on();
        c.timer = TimerConfig {
            setback: 2.0,
            alert: 3.0,
            kick: 5.0,
            decay: 1.0,
            ..TimerConfig::default()
        };
        let mut p = Player::default();
        let v = p.flag(&c, Check::Timer, 1.0, 100);
        assert_eq!((v.setback, v.alert, v.kick), (false, false, false));
        let v = p.flag(&c, Check::Timer, 1.0, 100);
        assert_eq!((v.vl, v.setback, v.alert), (2.0, true, false));
        let v = p.flag(&c, Check::Timer, 1.0, 100);
        assert!(v.alert && !v.kick);
        // At most one alert a second.
        assert!(!p.flag(&c, Check::Timer, 0.5, 110).alert);
        // 3 at tick 110 (half a level gone, half added); two seconds later two more are gone.
        assert_eq!(p.vl(&c, Check::Timer, 150), 1.0);
        assert_eq!(p.vl(&c, Check::Timer, 10_000), 0.0);
        let v = p.flag(&c, Check::Timer, 5.0, 10_000);
        assert!(v.kick && v.alert);
    }

    #[test]
    fn zero_turns_an_action_off() {
        let mut c = on();
        c.timer.kick = 0.0;
        let mut p = Player::default();
        assert!(!p.flag(&c, Check::Timer, 1000.0, 1).kick);
    }
}
