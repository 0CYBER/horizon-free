use super::*;

impl Game<'_> {
    /// Commands.performCommand for a guest with command access.
    /// Commands.performCommand for a guest, with the message arguments it
    /// signed (none after an unsigned command packet).
    pub(in crate::game) fn guest_command(
        &mut self,
        conn: u32,
        command: &str,
        signed: Option<Vec<(String, horizon_client::lan::hub::chat::ChatMessage)>>,
    ) {
        // The mods hear every command first, and may stop it.
        let mut canceled = false;
        let mut replies = Vec::new();
        if let Some((uuid, name)) = self
            .lan
            .as_ref()
            .and_then(|host| host.hub.guest(conn))
            .map(|g| (g.uuid, g.name.clone()))
        {
            self.native_player_command(
                uuid,
                &name,
                command.trim_start_matches('/'),
                &mut canceled,
                &mut replies,
            );
        }
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        for line in replies {
            host.hub.send_to(
                conn,
                &C::SystemChat(play::SystemChat {
                    content: horizon_client::lan::data::component_nbt(&line.component()),
                    overlay: false,
                }),
            );
        }
        if canceled {
            return;
        }
        if self.guest_message_command(conn, command, signed.as_ref()) {
            return;
        }
        // ListPlayersCommand needs no permission and lists every player.
        let words: Vec<&str> = command.trim_start_matches('/').split(' ').collect();
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        if words[0] == "list" && (words.len() == 1 || words[1..] == ["uuids"]) {
            let content = host.hub.list_reply(words.len() == 2);
            host.hub.send_to(
                conn,
                &C::SystemChat(play::SystemChat {
                    content,
                    overlay: false,
                }),
            );
            return;
        }
        // The chat filter's mute commands (game/interface/chat_filter.rs), on servers that filter.
        if crate::game::chat_filter::MUTE_COMMANDS.contains(&words[0]) && host.hub.chat_filter_on()
        {
            if let Some(mut host) = self.lan.take() {
                self.guest_mute_command(&mut host, conn, &words);
                self.lan = Some(host);
            }
            return;
        }
        // Horizon's `/tempban` (game/administration/tempban.rs), on dedicated servers.
        if crate::game::tempban::TEMPBAN_COMMANDS.contains(&words[0]) && self.server_lists.is_some()
        {
            if let Some(mut host) = self.lan.take() {
                self.guest_tempban_command(&mut host, conn, &words);
                self.lan = Some(host);
            }
            return;
        }
        // A native mod's command, for every player its level allows,
        // whether the world allows commands or not (game/modding/native_server.rs).
        let allowed = self.options.dedicated.is_some() || self.allow_commands();
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let integrated = host.hub.config.integrated;
        if let Some(level) = Self::native_command_level(host, command.trim_start_matches('/'))
            && let Some(permission) = host.hub.guest(conn).map(|g| g.permission)
            && permission >= level
        {
            host.commands.push(GuestCommand {
                conn,
                command: command.trim_start_matches('/').to_owned(),
                permission,
                integrated,
                quiet: false,
            });
            return;
        }
        // An integrated server runs every player's commands at their own
        // permission level (`CommandSourceStack.permissions`); a LAN world
        // follows its Allow Commands, and a dedicated server leaves it to
        // the ops list (the guest's permission level).
        if !integrated && (!host.settings.guest_commands || !allowed) {
            host.hub.send_to(
                conn,
                &C::SystemChat(play::SystemChat {
                    content: horizon_protocol::text::translatable("command.unknown.command", &[]),
                    overlay: false,
                }),
            );
            return;
        }
        // The guest's own level (Commands.performCommand with the player's
        // source): an operator's on a dedicated server, never the host's.
        let Some(permission) = host.hub.guest(conn).map(|g| g.permission) else {
            return;
        };
        host.commands.push(GuestCommand {
            conn,
            command: command.trim_start_matches('/').to_owned(),
            permission,
            integrated,
            quiet: false,
        });
    }

    /// The guests' queued commands, run with the hub in `self.lan` so that
    /// they reach the other players too (`/give Bob`, `/tp Bob`, `/kick`).
    pub(in crate::game) fn run_guest_commands(&mut self) {
        loop {
            let Some(host) = self.lan.as_mut() else {
                return;
            };
            if host.commands.is_empty() {
                return;
            }
            let c = host.commands.remove(0);
            self.run_guest_command(c);
        }
    }

    /// One guest's command as that guest, its feedback sent to them; false
    /// when it failed (or the guest is gone).
    pub(in crate::game) fn run_guest_command(&mut self, c: GuestCommand) -> bool {
        let Some(swap) = self.enter_guest(c.conn) else {
            return false;
        };
        self.chat_capture = Some(Vec::new());
        let level = std::mem::replace(&mut self.ui.registries.permission, c.permission);
        let ok = if self.perform_native_command(c.conn, c.permission, &c.command) {
            true
        } else if c.quiet {
            self.run_command_quietly(&c.command)
        } else {
            self.run_command(&c.command)
        };
        self.ui.registries.permission = level;
        let mut lines = self.chat_capture.take().unwrap_or_default();
        self.leave_guest(swap);
        if std::mem::take(&mut self.save_after_guest_turn) {
            lines.push(self.save_all_reply());
        }
        if let Some(host) = self.lan.as_mut() {
            host.hub.sync_all_effects();
        }
        if !c.integrated {
            // A teleport into another level happens before its feedback is sent.
            self.run_level_teleports();
        }
        // What the command earned is announced before its feedback.
        self.announce_guest_advancements();
        for line in lines {
            let Some(host) = self.lan.as_ref() else {
                return ok;
            };
            host.hub.send_to(
                c.conn,
                &C::SystemChat(play::SystemChat {
                    content: horizon_client::lan::data::component_nbt(&line.component()),
                    overlay: false,
                }),
            );
        }
        // The other operators hear of it; never the player whose command it was.
        self.send_admin_feedback(Some(c.conn));
        ok
    }

    /// The `broadcastToAdmins` lines a command queued (`source`: the guest
    /// whose it was; none for the host's or the console's): every other
    /// operator gets them, and the console logs them as the server's own
    /// system messages (MinecraftServer.sendSystemMessage).
    pub(in crate::game) fn send_admin_feedback(&mut self, source: Option<u32>) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        // server.toml's `admin_feedback_to_ops` off: the console's log only.
        let to_ops = host.hub.admin_feedback_to_ops;
        for (line, ops, console) in std::mem::take(&mut self.admin_feedback) {
            if ops && to_ops {
                let content = horizon_client::lan::data::component_nbt(&line.component());
                for g in host
                    .hub
                    .guests
                    .iter()
                    .filter(|g| Some(g.conn) != source && g.permission > 0)
                {
                    g.send(&C::SystemChat(play::SystemChat {
                        content: content.clone(),
                        overlay: false,
                    }));
                }
            }
            if console {
                crate::game::dedicated::log("INFO", &format!("System chat: {}", line.plain()));
            }
        }
    }

    /// ServerGamePacketListenerImpl.handleCustomCommandSuggestions: the
    /// completions of the command typed so far (after an optional slash), for
    /// the nodes the tree marks `ask_server`.
    /// ServerCommandSuggestionsProvider.tryProcessRequest: answer the latest request if this
    /// tick's answer is free. One client asking a hundred times a tick costs one parse.
    pub(super) fn guest_try_suggestions(&mut self, conn: u32) {
        let Some(host) = self.lan.as_mut() else {
            return;
        };
        let play = host.play.entry(conn).or_default();
        if !play.suggestion_budget {
            return;
        }
        let Some(payload) = play.suggestion.take() else {
            return;
        };
        play.suggestion_budget = false;
        self.guest_suggestions(conn, &payload);
    }

    pub(in crate::game) fn guest_suggestions(&mut self, conn: u32, payload: &[u8]) {
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        let mut r = horizon_protocol::buf::Reader::new(payload);
        let (Ok(id), Ok(text)) = (r.varint(), r.string(32500)) else {
            return;
        };
        let Some(permission) = host.hub.guest(conn).map(|g| g.permission) else {
            return;
        };
        let Some((player_uuid, player_name)) =
            host.hub.guest(conn).map(|g| (g.uuid, g.name.clone()))
        else {
            return;
        };
        let skip = usize::from(text.starts_with('/'));
        let command = &text[skip..];
        let d = self.ui.dispatcher.0.clone();
        // Server players and lists, not the host's registries: a dedicated server has no local player.
        let triggers =
            super::super::scoreboard::trigger_objectives_for(&self.score.board, &player_name);
        let mut players = if self.options.dedicated.is_some() {
            Vec::new()
        } else {
            self.ui.registries.players.clone()
        };
        players.extend(host.hub.online_players().map(|g| g.name.clone()));
        let (ops, banned_players, banned_ips, whitelisted) = match &self.server_lists {
            Some(lists) => {
                let l = lists.lock().unwrap();
                (
                    l.ops
                        .entries()
                        .iter()
                        .map(|e| e.user.name.clone())
                        .collect(),
                    l.bans
                        .entries()
                        .iter()
                        .map(|e| e.user.name.clone())
                        .collect(),
                    l.ip_bans.entries().iter().map(|e| e.user.clone()).collect(),
                    l.white
                        .entries()
                        .iter()
                        .map(|e| e.user.name.clone())
                        .collect(),
                )
            }
            None => Default::default(),
        };
        let reg = &mut self.ui.registries;
        let level = std::mem::replace(&mut reg.permission, permission);
        let own_players = std::mem::replace(&mut reg.players, players);
        let own_ops = std::mem::replace(&mut reg.ops, ops);
        let own_banned = std::mem::replace(&mut reg.banned_players, banned_players);
        let own_banned_ips = std::mem::replace(&mut reg.banned_ips, banned_ips);
        let own_whitelisted = std::mem::replace(&mut reg.whitelisted, whitelisted);
        let own_triggers = std::mem::replace(&mut reg.trigger_objectives, triggers);
        let parse = d.parse(command, &self.ui.registries);
        let ((start, end), mut suggestions) =
            d.suggestions(&parse, command.len(), &self.ui.registries);
        self.native_tab_complete(
            player_uuid,
            &player_name,
            command,
            [start, end],
            &mut suggestions,
        );
        let reg = &mut self.ui.registries;
        reg.permission = level;
        reg.players = own_players;
        reg.ops = own_ops;
        reg.banned_players = own_banned;
        reg.banned_ips = own_banned_ips;
        reg.whitelisted = own_whitelisted;
        reg.trigger_objectives = own_triggers;
        // ServerCommandSuggestionsProvider.limitSuggestionCount.
        suggestions.truncate(1000);
        let utf16 = |byte: usize| command[..byte.min(command.len())].encode_utf16().count() as i32;
        let mut w = horizon_protocol::buf::Writer::new();
        w.varint(id);
        w.varint(utf16(start) + skip as i32);
        w.varint(utf16(end) - utf16(start));
        w.varint(suggestions.len() as i32);
        for s in &suggestions {
            w.string(&s.text);
            w.bool(s.tooltip.is_some());
            if let Some(tip) = &s.tooltip {
                w.nbt(&literal(tip));
            }
        }
        let Some(host) = self.lan.as_ref() else {
            return;
        };
        host.hub.send_to(
            conn,
            &C::Unknown(
                horizon_protocol::ids::play::clientbound::COMMAND_SUGGESTIONS,
                w.data,
            ),
        );
    }
}
