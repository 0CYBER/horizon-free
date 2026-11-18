//! `/stop`, `/save-all`, `/save-off`, `/save-on`, `/setidletimeout`, `/banlist` and `/whitelist`
//! (StopCommand, SaveAllCommand, SaveOffCommand, SaveOnCommand,
//! SetPlayerIdleTimeoutCommand, BanListCommands, WhitelistCommand): commands of a
//! dedicated server, run on its lists by an operator in game.
use super::host::{CommandHost, R};
use horizon_client::{
    command::{Arg, Context},
    server::user_lists::NameAndId,
    text::Text,
};

/// What these commands need from the game: the lists and switches of a dedicated server.
/// The host already holds the lists: the command resolved them once, as
/// `Commands.CommandSelection.DEDICATED` asks, before it called in here.
pub trait ProfilesHost: CommandHost {
    /// `MinecraftServer.halt`: the server loop ends after this tick.
    fn request_stop(&mut self);
    /// Whether an entity save may run now.
    fn entity_save_admitted(&self) -> bool;
    /// Whether a guest's command is running.
    fn guest_turn_active(&self) -> bool;
    /// Saves when the acting guest's turn ends.
    fn save_after_guest_turn(&mut self);
    /// Saves the game; the error is what the log says.
    fn save_game(&mut self) -> Result<(), String>;
    /// Sets the auto-save switch: false when it was already so.
    fn set_auto_save(&mut self, enabled: bool) -> bool;
    /// `DedicatedServer.setPlayerIdleTimeout`.
    fn set_player_idle_timeout(&mut self, minutes: i32);
    /// The bans `/banlist` shows (`banlist`, `banlist.ips` or `banlist.players`): the name or
    /// address, the source and the reason.
    fn ban_entries(&self, id: &str) -> Vec<(String, String, Option<String>)>;
    /// `BanListEntry.getReasonMessage`.
    fn reason_message(&self, reason: Option<&str>) -> Text;
    /// `PlayerList.reloadWhiteList`.
    fn reload_whitelist(&mut self);
    /// Whether the whitelist is on.
    fn using_whitelist(&self) -> bool;
    /// `DedicatedServer.setUsingWhitelist`.
    fn set_using_whitelist(&mut self, on: bool);
    /// `MinecraftServer.kickUnlistedPlayers`.
    fn kick_unlisted_players(&mut self);
    /// The names on the whitelist.
    fn whitelist_names(&self) -> Vec<String>;
    /// Adds `target` to the whitelist, or removes it: false when nothing changed.
    fn whitelist_apply(&mut self, add: bool, target: &NameAndId) -> bool;
    /// `GameProfileArgument.getGameProfiles` of the argument `name`.
    fn game_profiles(&mut self, ctx: &Context, name: &str) -> R<Vec<NameAndId>>;
}

/// StopCommand: the server loop ends after this tick.
pub fn stop(host: &mut dyn ProfilesHost) -> R<i32> {
    host.feedback(Text::translate("commands.stop.stopping", &[]));
    host.request_stop();
    Ok(1)
}

/// SaveAllCommand. Every save waits for the chunks to be written, so
/// `flush` changes nothing.
pub fn save_all(host: &mut dyn ProfilesHost) -> R<i32> {
    host.feedback(Text::translate("commands.save.saving", &[]));
    if !host.entity_save_admitted() {
        return Err(host.error("commands.save.failed", &[]));
    }
    if host.guest_turn_active() {
        // The acting guest's record holds the game's state until their
        // turn ends: saving now would write it to their file.
        // shortcut: the other operators aren't told of a guest's save, add it if anyone asks.
        host.save_after_guest_turn();
        return Ok(1);
    }
    host.save_game().map_err(|e| {
        log::error!("Horizon: /save-all: {e}");
        host.error("commands.save.failed", &[])
    })?;
    host.feedback(Text::translate("commands.save.success", &[]));
    Ok(1)
}

/// SaveOffCommand and SaveOnCommand.
pub fn auto_save(host: &mut dyn ProfilesHost, enabled: bool) -> R<i32> {
    if !host.set_auto_save(enabled) {
        let already = if enabled {
            "commands.save.alreadyOn"
        } else {
            "commands.save.alreadyOff"
        };
        return Err(host.error(already, &[]));
    }
    let done = if enabled {
        "commands.save.enabled"
    } else {
        "commands.save.disabled"
    };
    host.feedback(Text::translate(done, &[]));
    Ok(1)
}

/// `SetPlayerIdleTimeoutCommand.setIdleTimeout`: the minutes of idleness
/// after which a player is kicked (`multiplayer.disconnect.idling`), 0 for
/// never. The reply names the minutes, or says the timeout is off, and the
/// command's result is the number of minutes.
pub fn setidle_timeout(host: &mut dyn ProfilesHost, ctx: &Context) -> R<i32> {
    let Some(Arg::Int(minutes)) = ctx.get("minutes") else {
        return Ok(0);
    };
    let minutes = *minutes;
    // DedicatedServer.setPlayerIdleTimeout: the live sweep reads it, and
    // server.properties keeps it.
    host.set_player_idle_timeout(minutes);
    host.feedback(idle_timeout_reply(minutes));
    Ok(minutes)
}

/// `BanListCommands.showList`: the players' bans then the addresses'
/// (`banlist`), or one of them; told to the source only.
pub fn banlist(host: &mut dyn ProfilesHost, id: &str) -> R<i32> {
    let lit = |s: &str| Text::literal(s.to_owned());
    let entries = host.ban_entries(id);
    if entries.is_empty() {
        host.feedback_unbroadcast(Text::translate("commands.banlist.none", &[]));
        return Ok(0);
    }
    host.feedback_unbroadcast(Text::translate(
        "commands.banlist.list",
        &[lit(&entries.len().to_string())],
    ));
    for (name, source, reason) in &entries {
        host.feedback_unbroadcast(Text::translate(
            "commands.banlist.entry",
            &[
                lit(name),
                lit(source),
                host.reason_message(reason.as_deref()),
            ],
        ));
    }
    Ok(entries.len() as i32)
}

/// `WhitelistCommand`: `on`, `off`, `list`, `reload`, `add` and `remove`.
pub fn whitelist(host: &mut dyn ProfilesHost, id: &str, ctx: &Context) -> R<i32> {
    let lit = |s: &str| Text::literal(s.to_owned());
    match id {
        "whitelist.reload" => {
            host.reload_whitelist();
            host.feedback(Text::translate("commands.whitelist.reloaded", &[]));
            host.kick_unlisted_players();
            Ok(1)
        }
        "whitelist.on" => {
            if host.using_whitelist() {
                return Err(host.error("commands.whitelist.alreadyOn", &[]));
            }
            host.set_using_whitelist(true);
            host.feedback(Text::translate("commands.whitelist.enabled", &[]));
            host.kick_unlisted_players();
            Ok(1)
        }
        "whitelist.off" => {
            if !host.using_whitelist() {
                return Err(host.error("commands.whitelist.alreadyOff", &[]));
            }
            host.set_using_whitelist(false);
            host.feedback(Text::translate("commands.whitelist.disabled", &[]));
            Ok(1)
        }
        "whitelist.list" => {
            let names = host.whitelist_names();
            if names.is_empty() {
                host.feedback_unbroadcast(Text::translate("commands.whitelist.none", &[]));
            } else {
                host.feedback_unbroadcast(Text::translate(
                    "commands.whitelist.list",
                    &[lit(&names.len().to_string()), lit(&names.join(", "))],
                ));
            }
            Ok(names.len() as i32)
        }
        _ => {
            let add = id == "whitelist.add";
            let targets = host.game_profiles(ctx, "targets")?;
            let mut count = 0;
            for target in &targets {
                if host.whitelist_apply(add, target) {
                    count += 1;
                    let key = if add {
                        "commands.whitelist.add.success"
                    } else {
                        "commands.whitelist.remove.success"
                    };
                    host.feedback(Text::translate(key, &[lit(&target.name)]));
                }
            }
            if count == 0 {
                return Err(host.error(
                    if add {
                        "commands.whitelist.add.failed"
                    } else {
                        "commands.whitelist.remove.failed"
                    },
                    &[],
                ));
            }
            if !add {
                host.kick_unlisted_players();
            }
            Ok(count)
        }
    }
}

/// `SetPlayerIdleTimeoutCommand.setIdleTimeout`'s reply: the minutes, or
/// `commands.setidletimeout.success.disabled` when the timeout is off.
fn idle_timeout_reply(minutes: i32) -> Text {
    if minutes > 0 {
        Text::translate(
            "commands.setidletimeout.success",
            &[Text::literal(minutes.to_string())],
        )
    } else {
        Text::translate("commands.setidletimeout.success.disabled", &[])
    }
}

#[cfg(test)]
mod tests {
    use super::idle_timeout_reply;

    /// `SetPlayerIdleTimeoutCommand.setIdleTimeout`: minutes above zero are
    /// named in `commands.setidletimeout.success`, and zero turns the timeout
    /// off with `commands.setidletimeout.success.disabled`.
    #[test]
    fn the_idle_timeout_reply_names_the_minutes_or_says_it_is_off() {
        let reply = idle_timeout_reply(5);
        assert_eq!(
            reply.translation_key(),
            Some("commands.setidletimeout.success"),
            "{reply:?}"
        );
        assert_eq!(reply.component()["with"], serde_json::json!([5]));
        let off = idle_timeout_reply(0);
        assert_eq!(
            off.translation_key(),
            Some("commands.setidletimeout.success.disabled")
        );
        // The disabled message takes no argument.
        assert!(off.component().get("with").is_none(), "{off:?}");
    }
}
