//! The server's console for native mods: every logged line (`console_line`)
//! and a console command whose feedback comes back (`server_run_command_captured`).
use super::{Dim, Game, commands::execute};
use horizon_client::logging;
use mod_api::ConsoleRefusal;

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

    /// `Commands::run_command` with its feedback kept: whether it ran without
    /// failing, and the console's lines for it as plain text.
    pub(super) fn server_run_command_captured(
        &mut self,
        line: &str,
    ) -> Result<(bool, Vec<String>), ConsoleRefusal> {
        if self.lan.is_none() {
            return Err(ConsoleRefusal::NotOpen);
        }
        let line = line.trim();
        let line = line.strip_prefix('/').unwrap_or(line);
        if line.trim().is_empty() {
            return Err(ConsoleRefusal::Blank);
        }
        let console = execute::Source {
            dimension: Dim::Overworld,
            silent: false,
            ..self.server_source()
        };
        let outer = self.command_source.replace(console);
        let filter = std::mem::replace(&mut self.native_command_filter, true);
        let outer_exec = std::mem::take(&mut self.exec);
        let outer_capture = self.chat_capture.replace(Vec::new());
        let ok = self.as_console(|game| game.run_command(line));
        let lines = std::mem::replace(&mut self.chat_capture, outer_capture).unwrap_or_default();
        self.exec = outer_exec;
        self.native_command_filter = filter;
        self.command_source = outer;
        Ok((ok, lines.iter().map(|text| text.plain()).collect()))
    }
}
