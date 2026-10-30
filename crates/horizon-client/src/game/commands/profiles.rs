//! `/op`, `/deop`, `/ban`, `/pardon` and `/whitelist` (OpCommand,
//! DeOpCommands, BanPlayerCommands, PardonCommand, WhitelistCommand):
//! commands of a dedicated server, run on its lists by an operator in game
//! (the console has its own handlers in game/dedicated.rs).
use super::{Arg, Context, Game, R, Target, err};
use horizon_client::{
    server::user_lists::{BanEntry, NameAndId, OpEntry, WhiteEntry},
    text::Text,
};
use horizon_protocol::text::translatable;

impl Game<'_> {
    /// Commands.CommandSelection.DEDICATED: on an integrated server these do
    /// not exist.
    fn dedicated_lists(
        &self,
    ) -> R<std::sync::Arc<std::sync::Mutex<super::super::dedicated::Lists>>> {
        match (&self.server_lists, self.options.dedicated.is_some()) {
            (Some(lists), true) => Ok(lists.clone()),
            _ => Err(err("command.unknown.command", &[])),
        }
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
    fn game_profiles(&mut self, ctx: &Context, name: &str) -> R<Vec<NameAndId>> {
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

    /// `WhitelistCommand`: `on`, `off`, `list`, `reload`, `add` and `remove`.
    pub(super) fn cmd_whitelist(&mut self, id: &str, ctx: &Context) -> R<i32> {
        let lists = self.dedicated_lists()?;
        let lit = |s: &str| Text::literal(s.to_owned());
        match id {
            "whitelist.reload" => {
                // PlayerList.reloadWhiteList.
                let _ = lists.lock().unwrap().white.load();
                self.feedback(Text::translate("commands.whitelist.reloaded", &[]));
                self.kick_unlisted_players(&lists);
                Ok(1)
            }
            "whitelist.on" => {
                if lists.lock().unwrap().use_whitelist {
                    return Err(err("commands.whitelist.alreadyOn", &[]));
                }
                self.set_using_whitelist(&lists, true);
                self.feedback(Text::translate("commands.whitelist.enabled", &[]));
                self.kick_unlisted_players(&lists);
                Ok(1)
            }
            "whitelist.off" => {
                if !lists.lock().unwrap().use_whitelist {
                    return Err(err("commands.whitelist.alreadyOff", &[]));
                }
                self.set_using_whitelist(&lists, false);
                self.feedback(Text::translate("commands.whitelist.disabled", &[]));
                Ok(1)
            }
            "whitelist.list" => {
                let names: Vec<String> = lists
                    .lock()
                    .unwrap()
                    .white
                    .entries()
                    .iter()
                    .map(|e| e.user.name.clone())
                    .collect();
                if names.is_empty() {
                    self.feedback_unbroadcast(Text::translate("commands.whitelist.none", &[]));
                } else {
                    self.feedback_unbroadcast(Text::translate(
                        "commands.whitelist.list",
                        &[lit(&names.len().to_string()), lit(&names.join(", "))],
                    ));
                }
                Ok(names.len() as i32)
            }
            _ => {
                let add = id == "whitelist.add";
                let targets = self.game_profiles(ctx, "targets")?;
                let mut count = 0;
                for target in &targets {
                    let done = {
                        let mut l = lists.lock().unwrap();
                        if add {
                            !l.white.contains(target)
                                && l.white.add(WhiteEntry {
                                    user: target.clone(),
                                })
                        } else {
                            l.white.contains(target) && l.white.remove(target)
                        }
                    };
                    if done {
                        count += 1;
                        let key = if add {
                            "commands.whitelist.add.success"
                        } else {
                            "commands.whitelist.remove.success"
                        };
                        self.feedback(Text::translate(key, &[lit(&target.name)]));
                    }
                }
                if count == 0 {
                    return Err(err(
                        if add {
                            "commands.whitelist.add.failed"
                        } else {
                            "commands.whitelist.remove.failed"
                        },
                        &[],
                    ));
                }
                if !add {
                    self.kick_unlisted_players(&lists);
                }
                Ok(count)
            }
        }
    }
}
