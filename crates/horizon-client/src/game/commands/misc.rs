//! `/random` (RandomCommand) and `/teammsg` (TeamMsgCommand).
use super::{Arg, Context, Game, R, Target, err};
use crate::game::chat::PLAYER_NAME;
use horizon_client::text::{self, Text};
use horizon_core::random::RandomSource;

fn millis(nanos: u64) -> String {
    format!("{:.1}", nanos as f32 / 1_000_000.0)
}

impl Game<'_> {
    /// `/tick` (TickCommand) over the game clock.
    pub(super) fn cmd_tick(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let t = |key: &str, args: &[Text]| Text::translate(key, args);
        let lit = |s: String| Text::literal(s);
        let time = match ctx.get("time") {
            Some(Arg::Int(v)) => *v,
            _ => 1,
        };
        match id {
            "tick.rate" => {
                let rate = match ctx.get("rate") {
                    Some(Arg::Float(v)) => *v,
                    _ => 20.0,
                };
                self.clock.set_rate(rate);
                self.feedback(t(
                    "commands.tick.rate.success",
                    &[lit(format!("{rate:.1}"))],
                ));
                Ok(rate as i32)
            }
            "tick.query" => {
                let rate = self.clock.rate();
                let busy = millis(self.clock.average_nanos());
                let rate_text = format!("{rate:.1}");
                if self.clock.sprinting() {
                    self.feedback(t("commands.tick.status.sprinting", &[]));
                    self.feedback(t(
                        "commands.tick.query.rate.sprinting",
                        &[lit(rate_text), lit(busy)],
                    ));
                } else {
                    let target = self.clock.tick_duration().as_nanos() as u64;
                    let status = if self.clock.frozen() {
                        "commands.tick.status.frozen"
                    } else if target < self.clock.average_nanos() {
                        "commands.tick.status.lagging"
                    } else {
                        "commands.tick.status.running"
                    };
                    self.feedback(t(status, &[]));
                    self.feedback(t(
                        "commands.tick.query.rate.running",
                        &[lit(rate_text), lit(busy), lit(millis(target))],
                    ));
                }
                let samples = self.clock.sorted_samples();
                let at = |q: f32| {
                    samples
                        .get(
                            ((samples.len() as f32 * q) as usize)
                                .min(samples.len().saturating_sub(1)),
                        )
                        .copied()
                        .unwrap_or(0)
                };
                self.feedback(t(
                    "commands.tick.query.percentiles",
                    &[
                        lit(millis(at(0.5))),
                        lit(millis(at(0.95))),
                        lit(millis(at(0.99))),
                        lit(samples.len().to_string()),
                    ],
                ));
                Ok(rate as i32)
            }
            "tick.sprint" => {
                if self.clock.start_sprint(time as u32) {
                    self.feedback(t("commands.tick.sprint.stop.success", &[]));
                }
                self.feedback(t("commands.tick.status.sprinting", &[]));
                Ok(1)
            }
            "tick.sprint.stop" => {
                if !self.clock.stop_sprint() {
                    return Err(err("commands.tick.sprint.stop.fail", &[]));
                }
                self.feedback(t("commands.tick.sprint.stop.success", &[]));
                Ok(1)
            }
            "tick.freeze" | "tick.unfreeze" => {
                let freeze = id == "tick.freeze";
                if freeze {
                    self.clock.stop_sprint();
                    self.clock.stop_step();
                }
                self.clock.set_frozen(freeze);
                self.feedback(t(
                    if freeze {
                        "commands.tick.status.frozen"
                    } else {
                        "commands.tick.status.running"
                    },
                    &[],
                ));
                Ok(i32::from(freeze))
            }
            "tick.step" => {
                if !self.clock.start_step(time as u32) {
                    return Err(err("commands.tick.step.fail", &[]));
                }
                self.feedback(t("commands.tick.step.success", &[lit(time.to_string())]));
                Ok(1)
            }
            _ => {
                if !self.clock.stop_step() {
                    return Err(err("commands.tick.step.stop.fail", &[]));
                }
                self.feedback(t("commands.tick.step.stop.success", &[]));
                Ok(1)
            }
        }
    }
    pub(super) fn cmd_random(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let seed = self.level.seed;
        if id == "random.reset" {
            let sequence = match ctx.get("sequence") {
                Some(Arg::Resource(s)) => Some(s.clone()),
                _ => None,
            };
            let int = |name: &str| match ctx.get(name) {
                Some(Arg::Int(v)) => Some(*v),
                _ => None,
            };
            let flag = |name: &str| !matches!(ctx.get(name), Some(Arg::Bool(false)));
            let name_text = |s: &str| Text::literal(s.to_owned());
            return match (sequence, int("seed")) {
                (None, None) => {
                    let n = self.rand_sequences.clear();
                    self.feedback(Text::translate(
                        "commands.random.reset.all.success",
                        &[Text::literal(n.to_string())],
                    ));
                    Ok(n as i32)
                }
                (None, Some(salt)) => {
                    self.rand_sequences.set_defaults(
                        salt,
                        flag("includeWorldSeed"),
                        flag("includeSequenceId"),
                    );
                    let n = self.rand_sequences.clear();
                    self.feedback(Text::translate(
                        "commands.random.reset.all.success",
                        &[Text::literal(n.to_string())],
                    ));
                    Ok(n as i32)
                }
                (Some(s), None) => {
                    self.rand_sequences.reset(&s, seed);
                    self.feedback(Text::translate(
                        "commands.random.reset.success",
                        &[name_text(&s)],
                    ));
                    Ok(1)
                }
                (Some(s), Some(salt)) => {
                    self.rand_sequences.reset_with(
                        &s,
                        seed,
                        salt,
                        flag("includeWorldSeed"),
                        flag("includeSequenceId"),
                    );
                    self.feedback(Text::translate(
                        "commands.random.reset.success",
                        &[name_text(&s)],
                    ));
                    Ok(1)
                }
            };
        }
        let Some(Arg::IntRange((lo, hi))) = ctx.get("range").cloned() else {
            return Ok(0);
        };
        let (min, max) = (lo.unwrap_or(i32::MIN), hi.unwrap_or(i32::MAX));
        let span = i64::from(max) - i64::from(min);
        if span == 0 {
            return Err(err("commands.random.error.range_too_small", &[]));
        }
        if span >= i64::from(i32::MAX) {
            return Err(err("commands.random.error.range_too_large", &[]));
        }
        let value = match ctx.get("sequence") {
            Some(Arg::Resource(s)) => {
                let s = s.clone();
                self.rand_sequences
                    .get(&s, seed)
                    .next_int_between_inclusive(min, max)
            }
            _ => self.world.random.next_int_between_inclusive(min, max),
        }
        .unwrap_or(min);
        if id == "random.roll" {
            let who = self.target_name(
                self.command_source
                    .and_then(|s| s.entity)
                    .unwrap_or(Target::Player),
            );
            self.system_message(Text::translate(
                "commands.random.roll",
                &[
                    who,
                    Text::literal(value.to_string()),
                    Text::literal(min.to_string()),
                    Text::literal(max.to_string()),
                ],
            ));
        } else {
            self.feedback(Text::translate(
                "commands.random.sample.success",
                &[Text::literal(value.to_string())],
            ));
        }
        Ok(value)
    }
    /// `/teammsg`: the executor's team hears the message.
    pub(super) fn cmd_teammsg(&mut self, ctx: &Context) -> R<i32> {
        let entity = match self.command_source {
            Some(s) => s.entity,
            None => Some(Target::Player),
        };
        let Some(entity) = entity else {
            return Err(err("permissions.requires.entity", &[]));
        };
        let Some(holder) = self.holder_name(entity) else {
            return Err(err("permissions.requires.entity", &[]));
        };
        let Some(team) = self.score.board.players_team(&holder).cloned() else {
            return Err(err("commands.teammsg.failed.noteam", &[]));
        };
        // Only the local player can be on the receiving end.
        let player_name = PLAYER_NAME.to_owned();
        let player_in_team = self
            .score
            .board
            .players_team(&player_name)
            .is_some_and(|t| t.name == team.name);
        if !player_in_team {
            return Ok(0);
        }
        let raw = match ctx.get("message") {
            Some(Arg::Str(m)) => m.clone(),
            _ => String::new(),
        };
        let message = self.expand_selectors(&raw);
        let hover = text::Style {
            hover: Some(Box::new(Text::translate("chat.type.team.hover", &[]))),
            suggest: Some("/teammsg ".into()),
            ..text::Style::default()
        };
        let team_name = team.formatted_display_name().with_style(&hover);
        let sender = self.target_name(entity);
        let key = if entity == Target::Player {
            "chat.type.team.sent"
        } else {
            "chat.type.team.text"
        };
        self.system_message(Text::translate(key, &[team_name, sender, message]));
        Ok(1)
    }
}
