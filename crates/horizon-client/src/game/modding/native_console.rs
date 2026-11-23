//! The server's console for native mods: every logged line (`console_line`)
//! and a console command whose feedback comes back (`run_command_with_output`).
use super::Game;
use horizon_client::logging;
use mod_api::{CommandResult, ConsoleLevel, ConsoleLogRefusal};

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

    /// `Messages::console_log`: `line` logged at `level` with its mod's prefix, through the same
    /// log as the server's own lines.
    pub(super) fn native_console_log(
        &self,
        mod_id: &str,
        level: ConsoleLevel,
        line: &str,
    ) -> Result<(), ConsoleLogRefusal> {
        let text = console_log_text(mod_id, line)?;
        match level {
            ConsoleLevel::Warn => log::warn!("{text}"),
            ConsoleLevel::Error => log::error!("{text}"),
            // `Info`, and any level a later API adds: the line is never lost.
            _ => log::info!("{text}"),
        }
        Ok(())
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

/// The most bytes of a mod's console line `Messages::console_log` keeps.
pub(super) const MAX_CONSOLE_LOG_BYTES: usize = 4096;
/// The most bytes of a mod id `Messages::console_log` accepts.
pub(super) const MAX_MOD_ID_BYTES: usize = 64;

/// The text `Messages::console_log` logs: `[mod_id] line`, the line cut to
/// `MAX_CONSOLE_LOG_BYTES` at a character boundary. Control characters in the line become
/// spaces, so the text is one log line. Refuses an empty mod id, one over `MAX_MOD_ID_BYTES` or
/// one with a control character, before anything is logged.
pub(super) fn console_log_text(mod_id: &str, line: &str) -> Result<String, ConsoleLogRefusal> {
    if mod_id.is_empty() {
        return Err(ConsoleLogRefusal::EmptyModId);
    }
    if mod_id.len() > MAX_MOD_ID_BYTES {
        return Err(ConsoleLogRefusal::ModIdTooLong);
    }
    if mod_id.chars().any(char::is_control) {
        return Err(ConsoleLogRefusal::InvalidModId);
    }
    let line: String = line
        .chars()
        .map(|c| if c.is_control() { ' ' } else { c })
        .collect();
    let mut end = line.len().min(MAX_CONSOLE_LOG_BYTES);
    while !line.is_char_boundary(end) {
        end -= 1;
    }
    Ok(format!("[{mod_id}] {}", &line[..end]))
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
