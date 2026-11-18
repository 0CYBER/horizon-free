//! `horizon-report [SECONDS]` and `horizon-trace [SECONDS]` on the dedicated server's console: the words they take.

/// The longest window a report records.
pub const MAX_SECONDS: u32 = 600;

/// What `horizon-report` was asked.
#[derive(Debug, PartialEq)]
pub enum Command {
    Record {
        seconds: u32,
        samples: bool,
        timeline: bool,
    },
    Stop,
    Status,
}

/// The longest `horizon-trace` records when no seconds are given.
const TRACE_DEFAULT_SECONDS: u32 = 30;

/// The words after `horizon-trace`: `[SECONDS | stop]`, a timeline-only capture.
#[expect(
    clippy::result_unit_err,
    reason = "a bad word only prints the usage line"
)]
pub fn parse_trace(words: &[&str]) -> Result<Command, ()> {
    match words {
        [] => Ok(Command::Record {
            seconds: TRACE_DEFAULT_SECONDS,
            samples: false,
            timeline: true,
        }),
        ["stop"] => Ok(Command::Stop),
        [seconds] => match seconds.parse::<u32>() {
            Ok(seconds) if (1..=MAX_SECONDS).contains(&seconds) => Ok(Command::Record {
                seconds,
                samples: false,
                timeline: true,
            }),
            _ => Err(()),
        },
        _ => Err(()),
    }
}

/// The words after `horizon-report`: `[SECONDS | stop | status] [--no-samples] [--timeline] [--profile]`, flags anywhere.
#[expect(
    clippy::result_unit_err,
    reason = "a bad word only prints the usage line"
)]
pub fn parse(words: &[&str]) -> Result<Command, ()> {
    let mut samples = true;
    let mut timeline = false;
    let mut positional = Vec::new();
    for &word in words {
        match word {
            "--no-samples" => samples = false,
            "--timeline" => timeline = true,
            // Samples are on by default now; the old switch is accepted and does nothing.
            "--profile" => {}
            flag if flag.starts_with("--") => return Err(()),
            word => positional.push(word),
        }
    }
    match positional[..] {
        [] => Ok(Command::Record {
            seconds: 60,
            samples,
            timeline,
        }),
        ["stop"] => Ok(Command::Stop),
        ["status"] => Ok(Command::Status),
        [seconds] => match seconds.parse::<u32>() {
            Ok(seconds) if (1..=MAX_SECONDS).contains(&seconds) => Ok(Command::Record {
                seconds,
                samples,
                timeline,
            }),
            _ => Err(()),
        },
        _ => Err(()),
    }
}

/// Registry ids for the loose entities a level counts apart from its mobs: items, orbs, projectiles, minecarts, boats.
pub const LOOSE_ENTITIES: [&str; 5] = [
    "minecraft:item",
    "minecraft:experience_orb",
    "horizon:projectile",
    "minecraft:minecart",
    "minecraft:boat",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn report_words_pick_the_window_and_the_samples() {
        let record = |seconds, samples| {
            Ok(Command::Record {
                seconds,
                samples,
                timeline: false,
            })
        };
        assert_eq!(parse(&[]), record(60, true));
        assert_eq!(parse(&["120"]), record(120, true));
        assert_eq!(parse(&["--no-samples", "30"]), record(30, false));
        assert_eq!(parse(&["30", "--no-samples"]), record(30, false));
        assert_eq!(parse(&["--profile"]), record(60, true));
        assert_eq!(
            parse(&["--timeline", "10"]),
            Ok(Command::Record {
                seconds: 10,
                samples: true,
                timeline: true
            })
        );
        assert_eq!(parse(&["stop"]), Ok(Command::Stop));
        assert_eq!(parse(&["status", "--no-samples"]), Ok(Command::Status));
        for bad in [
            &["0"][..],
            &["601"],
            &["abc"],
            &["--fast"],
            &["30", "60"],
            &["stop", "5"],
        ] {
            assert_eq!(parse(bad), Err(()), "{bad:?}");
        }
    }

    #[test]
    fn trace_words_pick_a_timeline_only_window() {
        let trace = |seconds| {
            Ok(Command::Record {
                seconds,
                samples: false,
                timeline: true,
            })
        };
        assert_eq!(parse_trace(&[]), trace(30));
        assert_eq!(parse_trace(&["45"]), trace(45));
        assert_eq!(parse_trace(&["600"]), trace(600));
        assert_eq!(parse_trace(&["stop"]), Ok(Command::Stop));
        for bad in [&["0"][..], &["601"], &["x"], &["10", "20"], &["stop", "1"]] {
            assert_eq!(parse_trace(bad), Err(()), "{bad:?}");
        }
    }
}
