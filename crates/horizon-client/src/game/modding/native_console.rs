//! The server's console for native mods: every logged line (`console_line`)
//! and a console command whose feedback comes back (`run_command_with_output`).
use super::Game;
use horizon_client::logging;
use mod_api::CommandResult;

impl Game<'_> {
    /// `console_line` for each line logged since the last server tick, while
    /// a mod listens; the lines its listeners log are not kept.
    pub(super) fn native_console_lines(&mut self) {
        let listened = mod_api::__private::listened(mod_api::HookId::ConsoleLine);
        logging::set_tap(listened);
        if !listened {
            return;
        }
        for line in logging::take_tapped() {
            let level = match line.level {
                log::Level::Error => "ERROR",
                log::Level::Warn => "WARN",
                _ => "INFO",
            };
            logging::untapped(|| self.native_console_line(level, &line.thread, &line.message));
        }
    }

    /// `Commands::run_command_with_output`: `native_run_command` with its feedback kept as
    /// plain text, at most `MAX_FEEDBACK_LINES` lines of `MAX_FEEDBACK_LINE_BYTES` bytes each.
    pub(super) fn native_run_command_with_output(&mut self, line: &str) -> CommandResult {
        let line = line.trim();
        let line = line.strip_prefix('/').unwrap_or(line);
        if line.trim().is_empty() {
            return CommandResult::default();
        }
        let (ran, lines) = self.run_console_command(line, true);
        let lines = capped_lines(&lines);
        CommandResult::new(ran, lines)
    }
}

/// The most feedback lines `Commands::run_command_with_output` keeps.
pub(super) const MAX_FEEDBACK_LINES: usize = 256;
/// The most bytes it keeps of one feedback line.
pub(super) const MAX_FEEDBACK_LINE_BYTES: usize = 4096;

/// `lines` as plain text, at most `MAX_FEEDBACK_LINES` of them, each cut to at most
/// `MAX_FEEDBACK_LINE_BYTES` bytes at a character boundary.
pub(super) fn capped_lines(lines: &[horizon_client::text::Text]) -> Vec<String> {
    lines
        .iter()
        .take(MAX_FEEDBACK_LINES)
        .map(|text| {
            let mut plain = text.plain();
            let mut end = plain.len().min(MAX_FEEDBACK_LINE_BYTES);
            while !plain.is_char_boundary(end) {
                end -= 1;
            }
            plain.truncate(end);
            plain
        })
        .collect()
}
