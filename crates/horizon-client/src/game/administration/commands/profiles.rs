//! `/op`, `/deop`, `/ban`, `/pardon`, `/ban-ip`, `/pardon-ip`, `/banlist`,
//! `/setidletimeout`, `/transfer` and `/whitelist` (OpCommand, DeOpCommands,
//! BanPlayerCommands, PardonCommand, BanIpCommands, PardonIpCommand,
//! BanListCommands, SetPlayerIdleTimeoutCommand, TransferCommand,
//! WhitelistCommand): commands of a dedicated server, run on its lists by an
//! operator in game (the console has its own handlers in game/interface/dedicated/mod.rs).
//! Also `/stop`, `/save-all`, `/save-off` and `/save-on` (StopCommand,
//! SaveAllCommand, SaveOffCommand, SaveOnCommand), on the switches the lists
//! share.
use super::{Arg, Context, DeferredSaveAll, Game, R, Target, command_host::profiles::ListsTarget, err};
use horizon_client::{
    server::user_lists::{BanEntry, NameAndId, OpEntry},
    text::Text,
};
use horizon_protocol::text::translatable;

impl Game<'_> {
    /// Commands.CommandSelection.DEDICATED: on an integrated server these do
    /// not exist.
    pub(super) fn dedicated_lists(
        &self,
    ) -> R<std::sync::Arc<std::sync::Mutex<super::super::dedicated::Lists>>> {
        match (&self.server_lists, self.is_dedicated_server()) {
            (Some(lists), true) => Ok(lists.clone()),
            _ => Err(err("command.unknown.command", &[])),
        }
    }

    /// StopCommand: the server loop (game/interface/dedicated/mod.rs) ends after this tick.
    pub(super) fn cmd_stop(&mut self) -> R<i32> {
        let lists = self.dedicated_lists()?;
        horizon_gameplay::commands::profiles::stop(&mut ListsTarget { game: self, lists })
    }

    pub(super) fn cmd_restart(&mut self) -> R<i32> {
        let lists = self.dedicated_lists()?;
        let reply = super::super::dedicated::request_restart(
            super::super::dedicated::supervised(),
            &mut lists.lock().unwrap().stop,
        );
        self.feedback(reply);
        Ok(1)
    }

    /// SaveAllCommand.saveAll: `commands.save.saving` to the source, then
    /// `commands.save.success` to it and the other operators, or the
    /// `ERROR_FAILED` it throws (result 0) when the save did not go through.
    /// `flush` changes nothing: every save waits for the chunks to be written.
    pub(super) fn cmd_save_all(&mut self) -> R<i32> {
        let lists = self.dedicated_lists()?;
        horizon_gameplay::commands::profiles::save_all(&mut ListsTarget { game: self, lists })
    }

    /// Save everything; `commands.save.success`, or `commands.save.failed` in red.
    pub(in crate::game) fn save_all_reply(&mut self) -> Text {
        match self.save_game() {
            Ok(()) => Text::translate("commands.save.success", &[]),
            Err(e) => {
                log::error!("Horizon: /save-all: {e}");
                Text::translate("commands.save.failed", &[]).with_color(horizon_client::text::RED)
            }
        }
    }

    /// `cmd_save_all` for a guest, once their turn ended and their own state
    /// is back in their record (`run_guest_command`): the success line to each
    /// source and to the other operators, or the `ERROR_FAILED` only to them.
    /// The turn's one save answers every `/save-all` that ran in it, each with
    /// the feedback it decided when it ran. Returns the command's result.
    pub(in crate::game) fn finish_deferred_save_all(
        &mut self,
        deferred: Vec<DeferredSaveAll>,
        lines: &mut Vec<Text>,
    ) -> bool {
        let reply = self.save_all_reply();
        let saved = reply.translation_key() == Some("commands.save.success");
        for run in deferred {
            // sendSuccess(line, true): a silent source sends nothing; else the
            // source hears it when it takes success feedback
            // (`send_command_feedback`), the operators always, and a dedicated
            // server's console logs it (`log_admin_commands`).
            if run.silent {
                continue;
            }
            if saved {
                if run.to_source {
                    lines.push(reply.clone());
                }
                // The guest's source is a player's, not the server's.
                self.broadcast_to_admins_read(
                    run.who,
                    run.to_operators,
                    run.to_console,
                    reply.clone(),
                );
            } else {
                // sendFailure: the source alone, always (`acceptsFailure`).
                lines.push(reply.clone());
            }
        }
        saved
    }

    /// SaveOffCommand and SaveOnCommand.
    pub(super) fn cmd_auto_save(&mut self, enabled: bool) -> R<i32> {
        let lists = self.dedicated_lists()?;
        horizon_gameplay::commands::profiles::auto_save(
            &mut ListsTarget { game: self, lists },
            enabled,
        )
    }

    /// `NameAndId` of a player target.
    fn target_profile(&self, t: Target) -> Option<NameAndId> {
        match t {
            Target::Player => Some(NameAndId {
                id: self.lan_acting_uuid?,
                name: self.acting_player_name().to_owned(),
            }),
            Target::Guest(conn) => self.command_guest(conn).map(|g| NameAndId {
                id: g.uuid,
                name: g.name.clone(),
            }),
            _ => None,
        }
    }

    /// `GameProfileArgument.getGameProfiles`: a selector's players
    /// (`SelectorResult.getNames`, no players is an error), else the name in
    /// the user cache.
    pub(super) fn game_profiles(&mut self, ctx: &Context, name: &str) -> R<Vec<NameAndId>> {
        match ctx.get(name).cloned() {
            Some(Arg::Entity(s)) => {
                let players: Vec<Target> = self
                    .select(&s)
                    .into_iter()
                    .filter(|t| t.is_player())
                    .collect();
                if players.is_empty() {
                    return Err(err("argument.entity.notfound.player", &[]));
                }
                Ok(players
                    .into_iter()
                    .filter_map(|t| self.target_profile(t))
                    .collect())
            }
            Some(Arg::Str(text)) => self
                .resolve_name(&text)
                .map(|p| vec![p])
                .ok_or_else(|| err("argument.player.unknown", &[])),
            _ => Err(err("argument.player.unknown", &[])),
        }
    }

    /// `CommandSourceStack.getTextName`.
    fn source_text_name(&self) -> String {
        if self.console_broadcast.is_some() {
            "Server".to_owned()
        } else {
            self.acting_player_name().to_owned()
        }
    }

    /// `OpCommand.opPlayers`, `DeOpCommands.deopPlayers`,
    /// `BanPlayerCommands.banPlayers` and `PardonCommand.pardonPlayers`.
    pub(super) fn cmd_profiles(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let lists = self.dedicated_lists()?;
        let players = self.game_profiles(ctx, "targets")?;
        let lit = |s: &str| Text::literal(s.to_owned());
        let mut count = 0;
        match id {
            "op" => {
                for player in &players {
                    let (known, level) = {
                        let l = lists.lock().unwrap();
                        (l.ops.contains(player), l.op_level)
                    };
                    if !known && self.permission_change(&lists, player, level) {
                        continue;
                    }
                    let added = {
                        let mut l = lists.lock().unwrap();
                        let level = l.op_level;
                        !l.ops.contains(player)
                            && l.ops.add(OpEntry {
                                user: player.clone(),
                                level,
                                bypasses_player_limit: false,
                            })
                    };
                    if added {
                        self.sync_ops(&lists);
                        count += 1;
                        self.feedback(Text::translate("commands.op.success", &[lit(&player.name)]));
                    }
                }
                if count == 0 {
                    return Err(err("commands.op.failed", &[]));
                }
            }
            "deop" => {
                for player in &players {
                    let known = lists.lock().unwrap().ops.contains(player);
                    if known && self.permission_change(&lists, player, 0) {
                        continue;
                    }
                    let removed = {
                        let mut l = lists.lock().unwrap();
                        l.ops.contains(player) && l.ops.remove(player)
                    };
                    if removed {
                        self.sync_ops(&lists);
                        count += 1;
                        self.feedback(Text::translate(
                            "commands.deop.success",
                            &[lit(&player.name)],
                        ));
                    }
                }
                if count == 0 {
                    return Err(err("commands.deop.failed", &[]));
                }
                self.kick_unlisted_players(&lists);
            }
            "ban" => {
                let reason = match ctx.get("reason") {
                    Some(Arg::Str(raw)) => {
                        let raw = raw.clone();
                        Some(self.expand_selectors(&raw).plain().to_string())
                    }
                    _ => None,
                };
                let source = self.source_text_name();
                for player in &players {
                    let banned = lists.lock().unwrap().bans.contains(player);
                    if banned {
                        continue;
                    }
                    lists.lock().unwrap().bans.add(BanEntry::new(
                        player.clone(),
                        Some(source.clone()),
                        None,
                        reason.clone(),
                    ));
                    count += 1;
                    // BanListEntry.getReasonMessage.
                    let shown = reason.as_deref().map_or_else(
                        || Text::translate("multiplayer.disconnect.banned.reason.default", &[]),
                        lit,
                    );
                    self.feedback(Text::translate(
                        "commands.ban.success",
                        &[lit(&player.name), shown],
                    ));
                    self.kick_uuid(
                        player.id,
                        translatable("multiplayer.disconnect.banned", &[]),
                    );
                }
                if count == 0 {
                    return Err(err("commands.ban.failed", &[]));
                }
            }
            _ => {
                for player in &players {
                    let removed = {
                        let mut l = lists.lock().unwrap();
                        l.bans.contains(player) && l.bans.remove(player)
                    };
                    if removed {
                        count += 1;
                        self.feedback(Text::translate(
                            "commands.pardon.success",
                            &[lit(&player.name)],
                        ));
                    }
                }
                if count == 0 {
                    return Err(err("commands.pardon.failed", &[]));
                }
            }
        }
        Ok(count)
    }

    /// `BanIpCommands.banIpOrName` and `PardonIpCommand.unban`: an address,
    /// or for `/ban-ip` an online player's.
    pub(super) fn cmd_ip_bans(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let lists = self.dedicated_lists()?;
        let lit = |s: &str| Text::literal(s.to_owned());
        let target = match ctx.get("target") {
            Some(Arg::Str(t)) => t.clone(),
            _ => return Err(err("command.unknown.argument", &[])),
        };
        let is_ip = target.parse::<std::net::IpAddr>().is_ok();
        if id == "pardon-ip" {
            if !is_ip {
                return Err(err("commands.pardonip.invalid", &[]));
            }
            let removed = {
                let mut l = lists.lock().unwrap();
                l.ip_bans.get(&target).is_some() && l.ip_bans.remove(&target)
            };
            if !removed {
                return Err(err("commands.pardonip.failed", &[]));
            }
            self.feedback(Text::translate(
                "commands.pardonip.success",
                &[lit(&target)],
            ));
            return Ok(1);
        }
        // PlayerList.getPlayerByName, then ServerPlayer.getIpAddress.
        let address = |name: &str| {
            let hub = &self.lan.as_ref()?.hub;
            hub.online_players()
                .find(|g| g.name.eq_ignore_ascii_case(name))
                .map(|g| g.address.ip().to_string())
        };
        let ip = if is_ip {
            Some(target)
        } else {
            address(&target)
        };
        let Some(ip) = ip else {
            return Err(err("commands.banip.invalid", &[]));
        };
        if lists.lock().unwrap().ip_bans.get(&ip).is_some() {
            return Err(err("commands.banip.failed", &[]));
        }
        let players: Vec<(u128, String)> = self
            .lan
            .as_ref()
            .map(|h| {
                h.hub
                    .online_players()
                    .filter(|g| g.address.ip().to_string() == ip)
                    .map(|g| (g.uuid, g.name.clone()))
                    .collect()
            })
            .unwrap_or_default();
        let reason = match ctx.get("reason") {
            Some(Arg::Str(raw)) => {
                let raw = raw.clone();
                Some(self.expand_selectors(&raw).plain().to_string())
            }
            _ => None,
        };
        let source = self.source_text_name();
        lists.lock().unwrap().ip_bans.add(BanEntry::new(
            ip.clone(),
            Some(source),
            None,
            reason.clone(),
        ));
        self.feedback(Text::translate(
            "commands.banip.success",
            &[lit(&ip), Self::reason_message(reason.as_deref())],
        ));
        if !players.is_empty() {
            let names: Vec<&str> = players.iter().map(|(_, n)| n.as_str()).collect();
            self.feedback(Text::translate(
                "commands.banip.info",
                &[lit(&players.len().to_string()), lit(&names.join(", "))],
            ));
        }
        for (uuid, _) in &players {
            self.kick_uuid(*uuid, translatable("multiplayer.disconnect.ip_banned", &[]));
        }
        Ok(players.len() as i32)
    }

    /// `SetPlayerIdleTimeoutCommand.setIdleTimeout`: the minutes of idleness
    /// after which a player is kicked (`multiplayer.disconnect.idling`), 0 for
    /// never. The reply names the minutes, or says the timeout is off, and the
    /// command's result is the number of minutes.
    pub(super) fn cmd_setidle_timeout(&mut self, ctx: &Context) -> R<i32> {
        let lists = self.dedicated_lists()?;
        horizon_gameplay::commands::profiles::setidle_timeout(
            &mut ListsTarget { game: self, lists },
            ctx,
        )
    }

    /// `TransferCommand.register` and `TransferCommand.transfer`: each target's
    /// client is handed to `hostname:port` (`ClientboundTransferPacket`), and
    /// the source is told who went — the player's name for one, the count for
    /// several. Without a players argument the source's own player is the
    /// target, as `getPlayerOrException` asks.
    pub(super) fn cmd_transfer(&mut self, ctx: &Context) -> R<i32> {
        let Some(Arg::Str(hostname)) = ctx.get("hostname").cloned() else {
            return Ok(0);
        };
        // `IntegerArgumentType.getInteger(c, "port")`, else 25565.
        let port = match ctx.get("port") {
            Some(Arg::Int(port)) => *port,
            _ => 25565,
        };
        let targets = if ctx.get("players").is_some() {
            self.entities(ctx, "players", true)?
        } else if self.lan_acting.is_none() {
            // CommandSourceStack.getPlayerOrException: a console or a command
            // block has no player to send.
            return Err(err("permissions.requires.player", &[]));
        } else {
            vec![Target::Player]
        };
        let players: Vec<(u32, String)> = targets
            .iter()
            .filter_map(|target| {
                let conn = match *target {
                    Target::Player => self.lan_acting?,
                    Target::Guest(conn) => conn,
                    _ => return None,
                };
                Some((conn, self.command_guest(conn)?.name.clone()))
            })
            .collect();
        if players.is_empty() {
            return Err(err("commands.transfer.error.no_players", &[]));
        }
        if let Some(host) = self.lan.as_mut() {
            for (conn, _) in &players {
                host.hub.transfer(*conn, &hostname, port);
            }
        }
        // CommandResponseTracker.messages: one player by display name, several
        // by count, both with the hostname and the port.
        let lit = |s: &str| Text::literal(s.to_owned());
        let (key, first) = match players.len() {
            1 => ("commands.transfer.success.single", lit(&players[0].1)),
            n => ("commands.transfer.success.multiple", lit(&n.to_string())),
        };
        self.feedback(Text::translate(
            key,
            &[first, lit(&hostname), lit(&port.to_string())],
        ));
        Ok(players.len() as i32)
    }

    /// `BanListCommands.showList`: the players' bans then the addresses'
    /// (`banlist`), or one of them; told to the source only.
    pub(super) fn cmd_banlist(&mut self, id: &str) -> R<i32> {
        let lists = self.dedicated_lists()?;
        horizon_gameplay::commands::profiles::banlist(&mut ListsTarget { game: self, lists }, id)
    }

    /// `BanListEntry.getReasonMessage`.
    pub(super) fn reason_message(reason: Option<&str>) -> Text {
        reason.map_or_else(
            || Text::translate("multiplayer.disconnect.banned.reason.default", &[]),
            |r| Text::literal(r.to_owned()),
        )
    }

    /// `WhitelistCommand`: `on`, `off`, `list`, `reload`, `add` and `remove`.
    pub(super) fn cmd_whitelist(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let lists = self.dedicated_lists()?;
        horizon_gameplay::commands::profiles::whitelist(
            &mut ListsTarget { game: self, lists },
            id,
            ctx,
        )
    }
}

#[cfg(test)]
#[path = "save_all_command_tests.rs"]
mod save_all_command_tests;
