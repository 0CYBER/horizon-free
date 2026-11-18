//! `/random` (RandomCommand) and `/teammsg` (TeamMsgCommand).
use super::{Arg, Context, Game, R, Target, err};
use crate::game::chat::PLAYER_NAME;
use horizon_client::{
    clock::SprintReport,
    text::{self, Text},
};
use horizon_core::java_number::format_fixed;
use horizon_core::random::RandomSource;

pub(super) fn millis(nanos: u64) -> String {
    format_fixed(f64::from(nanos as f32 / 1_000_000.0), 1)
}

impl Game<'_> {
    /// `/stopwatch` (StopwatchCommand).
    pub(super) fn cmd_stopwatch(&mut self, id: &str, ctx: &Context) -> R<i32> {
        horizon_gameplay::commands::misc::stopwatch(self, id, ctx)
    }
    /// `/tick` (TickCommand) over the game clock.
    pub(super) fn cmd_tick(&mut self, id: &str, ctx: &Context) -> R<i32> {
        horizon_gameplay::commands::misc::tick(self, id, ctx)
    }
    /// The report `/tick sprint` prints when a sprint ends. Each caller then
    /// runs `on_tick_rate_changed`, as `ServerTickRateManager.finishTickSprint` does.
    pub(in crate::game) fn sprint_report(&mut self, report: &SprintReport) {
        self.feedback(Text::translate(
            "commands.tick.sprint.report",
            &[
                Text::literal(report.ticks_per_second.to_string()),
                Text::literal(report.milliseconds_per_tick.clone()),
            ],
        ));
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
            let line = Text::translate(
                "commands.random.roll",
                &[
                    self.source_display_name(),
                    Text::literal(value.to_string()),
                    Text::literal(min.to_string()),
                    Text::literal(max.to_string()),
                ],
            );
            // PlayerList.broadcastSystemMessage: every player hears the roll,
            // the source's own copy with the captured output it has (`/say`'s).
            let captured = self.chat_capture.as_ref().and(self.lan_acting);
            if let Some(host) = &self.lan {
                let content = horizon_client::lan::data::component_nbt(&line.component());
                match captured {
                    Some(conn) => host.hub.system_message_except(content, conn),
                    None => host.hub.system_message(content),
                }
            }
            // MinecraftServer.sendSystemMessage: the server's log, unless the
            // console's captured reply below is what logs it (`console_reply`).
            if self.options.dedicated.is_some()
                && (self.lan_acting.is_some() || self.chat_capture.is_none())
            {
                super::super::dedicated::log("INFO", &format!("System chat: {}", line.plain()));
            }
            self.system_message(line);
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
            hover: Some(std::sync::Arc::new(Text::translate(
                "chat.type.team.hover",
                &[],
            ))),
            click: Some(text::ClickEvent::SuggestCommand("/teammsg ".into())),
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

#[cfg(test)]
#[path = "tick_status_broadcast_tests.rs"]
mod tick_status_broadcast_tests;

#[cfg(test)]
mod random_command_tests;
