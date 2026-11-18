//! The server's run/stop boundary, shared by integrated and dedicated servers.

use super::Game;
use horizon_client::Result;
use std::panic::{AssertUnwindSafe, catch_unwind};

impl Game<'_> {
    /// `MinecraftServer.runServer`'s finally: once startup begins, every exit
    /// stops and saves the server, including a failed init or tick.
    pub(super) fn run_server_lifecycle(
        &mut self,
        dedicated: bool,
        run: impl FnOnce(&mut Self, &mut bool) -> Result<()>,
    ) -> Result<()> {
        let mut startup_began = false;
        let run = catch_unwind(AssertUnwindSafe(|| run(self, &mut startup_began)));
        let crashed = run.is_err();
        let result = match run {
            Ok(result) => result,
            Err(payload) => {
                let message = super::dedicated::panic_message(payload.as_ref());
                log::error!("The server thread panicked: {message}");
                self.prepare_server_crash();
                Err(format!("the server thread panicked: {message}").into())
            }
        };
        if !startup_began {
            return result;
        }
        let run_error = result.as_ref().err().map(ToString::to_string);
        let shutdown = self.shutdown_started_server(dedicated, crashed, run_error.as_deref());
        result.and(shutdown)
    }

    /// Vanilla's nested `stopServer` finally: a listener panic still reaches
    /// cleanup, storage release and `server_stopped`, and the error is kept.
    fn shutdown_started_server(
        &mut self,
        dedicated: bool,
        crashed: bool,
        run_error: Option<&str>,
    ) -> Result<()> {
        let mut failure: Option<String> = None;
        if let Err(payload) =
            catch_unwind(AssertUnwindSafe(|| self.native_server_stopping(dedicated)))
        {
            remember_shutdown(&mut failure, shutdown_panic(payload.as_ref()));
            self.keep_players_after_shutdown_panic();
        }
        if let Some(error) = run_error {
            let error = error.to_owned();
            if let Err(payload) = catch_unwind(AssertUnwindSafe(|| {
                self.native_lifecycle_world_failed(&error);
                self.native_lifecycle_close_failed(&error);
            })) {
                remember_shutdown(&mut failure, shutdown_panic(payload.as_ref()));
                self.keep_players_after_shutdown_panic();
            }
        }
        match catch_unwind(AssertUnwindSafe(|| {
            if dedicated {
                self.stop_dedicated(crashed)
            } else {
                self.stop_integrated(crashed)
            }
        })) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                let text = error.to_string();
                let _ = catch_unwind(AssertUnwindSafe(|| {
                    self.native_lifecycle_close_failed(&text);
                }));
                log::error!("Exception stopping the server: {text}");
                if failure.is_none() {
                    failure = Some(text);
                }
            }
            Err(payload) => {
                let message = shutdown_panic(payload.as_ref());
                remember_shutdown(&mut failure, message.clone());
                let _ = catch_unwind(AssertUnwindSafe(|| {
                    self.native_lifecycle_close_failed(&message);
                }));
                self.keep_players_after_shutdown_panic();
                // A config/player callback may unwind before the stop path
                // reaches its final save. Complete that same tail, not just
                // the writes already queued, while retaining the first error.
                if let Err(error) = self.finish_server_save(crashed) {
                    remember_shutdown(&mut failure, error.to_string());
                }
            }
        }
        if let Err(payload) =
            catch_unwind(AssertUnwindSafe(|| self.native_server_stopped(dedicated)))
        {
            remember_shutdown(&mut failure, shutdown_panic(payload.as_ref()));
        }
        match failure {
            Some(message) => Err(message.into()),
            None => Ok(()),
        }
    }

    /// A shutdown callback can panic inside a player's turn too: the players saved after it
    /// leave that turn out, as a crash's save does.
    fn keep_players_after_shutdown_panic(&mut self) {
        if let Some(warning) = self.keep_interrupted_players() {
            log::warn!("{warning}");
        }
    }

    fn prepare_server_crash(&mut self) {
        if let Some(warning) = self.keep_interrupted_players() {
            log::warn!("{warning}");
        }
    }

    /// Before a crash's save: a player whose turn the panic interrupted keeps their last
    /// saved file, not the half-done turn. That is the player still swapped in
    /// (`lan_acting`), one whose record a swap still displaces (`lan_guest_swaps`: an outer
    /// turn, or a turn the panic hit while it was being entered), or one whose turn wrote its
    /// state back on the way out (`lan_interrupted_turns`). They are unplaced, so `save_guest`
    /// skips them. If the network tick lost the hub, only world state can be saved. Returns
    /// the warning to log.
    pub(in crate::game) fn keep_interrupted_players(&mut self) -> Option<&'static str> {
        let interrupted: Vec<u32> = self
            .lan_acting
            .into_iter()
            .chain(self.lan_guest_swaps.iter().copied())
            .chain(self.lan_interrupted_turns.drain(..))
            .collect();
        let Some(host) = self.lan.as_mut() else {
            self.keep_owner_files = true;
            return Some(
                "The panic interrupted the network tick: players are not saved, only the world",
            );
        };
        let owner = host.owner.as_ref().map(|o| o.uuid);
        if host
            .hub
            .guests
            .iter()
            .any(|g| Some(g.uuid) == owner && interrupted.contains(&g.conn))
        {
            self.keep_owner_files = true;
        }
        for &conn in &interrupted {
            let _ = host
                .hub
                .guests
                .update_connection(conn, |guest| guest.placed = false);
        }
        (!interrupted.is_empty()).then_some(
            "The panic interrupted a player's turn: that player keeps the last saved file",
        )
    }

    /// Complete teardown even if freeing beds or the final save fails. A failed
    /// open must never overwrite a world the server did not successfully adopt.
    /// The session lock is not that marker: it is installed before the prepared
    /// level, so tag reads work, and a save in that gap would write the placeholder.
    pub(super) fn finish_server_save(&mut self, crashed: bool) -> Result<()> {
        // IntegratedServer.teardownPublishedState must happen even if an
        // earlier callback panicked; otherwise lan_close only unpublishes a
        // still-serving server instead of disconnecting and dropping its hub.
        self.stop_serving();
        let mut result = match catch_unwind(AssertUnwindSafe(|| {
            if crashed {
                Ok(())
            } else {
                self.free_player_beds()
            }
        })) {
            Ok(result) => result,
            Err(payload) => {
                // A save listener can panic inside a turn: lan_close saves the leaving players.
                self.keep_players_after_shutdown_panic();
                Err(shutdown_panic(payload.as_ref()).into())
            }
        };
        let saved = match catch_unwind(AssertUnwindSafe(|| {
            if self.world_adopted && self.session_lock.is_some() {
                self.save_game()
            } else {
                Ok(())
            }
        })) {
            Ok(result) => result,
            Err(payload) => {
                // A save listener can panic inside a turn: lan_close saves the leaving players.
                self.keep_players_after_shutdown_panic();
                Err(shutdown_panic(payload.as_ref()).into())
            }
        };
        result = result.and(saved);
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| self.lan_close(false))) {
            let message = shutdown_panic(payload.as_ref());
            if result.is_ok() {
                result = Err(message.into());
            }
        }
        // A player callback/save in lan_close can also unwind before taking
        // the host. Reuse its transport teardown independently of callbacks.
        if let Err(payload) = catch_unwind(AssertUnwindSafe(|| self.lan_disconnect(false)))
            && result.is_ok()
        {
            result = Err(shutdown_panic(payload.as_ref()).into());
        }
        match catch_unwind(AssertUnwindSafe(|| self.save.flush())) {
            Ok(Ok(())) => {}
            Ok(Err(error)) => {
                if result.is_ok() {
                    result = Err(error);
                }
            }
            Err(payload) => {
                let message = shutdown_panic(payload.as_ref());
                if result.is_ok() {
                    result = Err(message.into());
                }
            }
        }
        if result.is_ok()
            && let Err(payload) =
                catch_unwind(AssertUnwindSafe(|| self.native_lifecycle_world_closed()))
        {
            result = Err(shutdown_panic(payload.as_ref()).into());
        }
        self.session_lock = None;
        super::cauldron_interactions::clear();
        self.clear_world_parts();
        self.world_adopted = false;
        result
    }
}

fn shutdown_panic(payload: &(dyn std::any::Any + Send)) -> String {
    format!(
        "the server shutdown panicked: {}",
        super::dedicated::panic_message(payload)
    )
}

fn remember_shutdown(failure: &mut Option<String>, message: String) {
    log::error!("{message}");
    if failure.is_none() {
        *failure = Some(message);
    }
}

#[cfg(test)]
#[path = "server_lifecycle_tests.rs"]
mod tests;
