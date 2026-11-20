//! What agents read of the connection, and draining the network queue each frame.

use super::*;

impl Game<'_> {
    /// The minecarts the server sent, for the control socket's `status`
    /// (their velocity is not tracked here).
    pub(super) fn remote_carts_body(&self) -> Option<Vec<serde_json::Value>> {
        let r = self.remote.as_ref()?;
        let mut carts: Vec<&Tracked> = r
            .entities
            .values()
            .filter(|t| t.kind.ends_with("minecart"))
            .collect();
        carts.sort_by(|a, b| {
            a.position
                .partial_cmp(&b.position)
                .unwrap_or(std::cmp::Ordering::Equal)
        });
        Some(
            carts
                .into_iter()
                .map(|t| serde_json::json!({"kind": t.kind, "position": t.position, "velocity": [0.0, 0.0, 0.0]}))
                .collect(),
        )
    }

    /// This player's account: a splitscreen player's own, else the game's (P1's).
    pub(super) fn auth_body(&self) -> Option<std::sync::Arc<horizon_client::auth::AuthSession>> {
        match self.options.split_guest {
            true => self.split_auth.clone(),
            false => horizon_client::auth::current(),
        }
    }
    /// Connected to a server in online mode (its Login): each player needs its own account.
    pub(super) fn online_server_body(&self) -> bool {
        self.remote.as_ref().is_some_and(|r| r.online_mode)
    }

    /// `ClientSuggestionProvider.customCompletionSuggestions` of the server
    /// connection (none without one).
    pub(super) fn remote_custom_completions_body(&self) -> std::collections::BTreeSet<String> {
        self.remote
            .as_ref()
            .map(|r| r.custom_completions.clone())
            .unwrap_or_default()
    }
    /// The names in the server's player list, sorted.
    pub(super) fn remote_online_names_body(&self) -> Option<Vec<String>> {
        let r = self.remote.as_ref()?;
        let mut names: Vec<String> = r.players.values().cloned().collect();
        names.sort();
        Some(names)
    }
    /// The open menu's window id (0: the inventory).
    pub(super) fn remote_menu_id_body(&self) -> i32 {
        self.remote
            .as_ref()
            .and_then(|r| r.menu.as_ref())
            .map_or(0, |m| m.0)
    }
    /// The server's last `n` chat lines, newest first.
    pub(super) fn remote_chat_body(&self, n: usize) -> Option<Vec<String>> {
        let r = self.remote.as_ref()?;
        Some(r.chat.iter().rev().take(n).map(|l| l.0.clone()).collect())
    }

    /// Received events left for the next frame (`poll_remote`): until they
    /// are handled, the terrain the server sent is not all there.
    pub(super) fn remote_backlog_body(&self) -> usize {
        self.remote.as_ref().map_or(0, |r| r.backlog.len())
    }
    /// A headless command oracle has a real connection but no movement tick.
    /// Acknowledge loaded terrain without predicting or sending movement.
    pub(super) fn poll_stationary_client_body(&mut self) -> Result<()> {
        self.poll_remote()?;
        let loaded = self.player.as_ref().is_some_and(|p| {
            self.streaming.is_lit(
                (p.position.x.floor() as i32) >> 4,
                (p.position.z.floor() as i32) >> 4,
            )
        });
        if loaded && self.remote.as_ref().is_some_and(|r| !r.loaded_sent) {
            self.send(S::PlayerLoaded(Empty));
            let r = self.remote.as_mut().unwrap();
            r.loaded_sent = true;
            r.level_load.finish();
        }
        Ok(())
    }

    /// Process what the network thread received (once per frame, like the
    /// client's packet processor), in order. Chunk packets get
    /// `CHUNK_PACKETS_PER_FRAME`: once a frame has spent it, the events from
    /// the next chunk packet on wait for the next frame, unless they have
    /// already waited `BACKLOG_LIMIT`. A chunk packet still decoding holds
    /// back what follows it.
    pub(super) fn poll_remote_body(&mut self) -> Result<()> {
        let polled = self.poll_remote_events();
        // The block events and block changes of a piston clock start and end
        // thousands of moves at once: the collision view and the renderer
        // take them all in one sync, before the tick or frame reads them.
        if std::mem::take(&mut self.moving_pistons_stale) {
            self.sync_moving_pistons();
        }
        polled
    }
    /// ClientSuggestionProvider.customSuggestion: the chat's completion
    /// request for an `ask_server` node goes to the server.
    pub(super) fn send_suggestion_request_body(&mut self) {
        if self.remote.is_none() {
            return;
        }
        let Some((id, command)) = self
            .ui
            .chat_screen
            .as_mut()
            .and_then(|s| s.take_request())
            .or_else(|| self.command_block_suggestion_request())
        else {
            return;
        };
        let mut w = horizon_protocol::buf::Writer::new();
        w.varint(id);
        w.string(&command);
        self.send(S::Unknown(
            horizon_protocol::ids::play::serverbound::COMMAND_SUGGESTION,
            w.data,
        ));
    }
    /// ClientboundCommandSuggestionsPacket: id, range start and length, then
    /// each suggestion's text and optional tooltip.
    pub(super) fn handle_command_suggestions_body(&mut self, data: &[u8]) {
        let lang = self
            .remote
            .as_ref()
            .map(|r| r.lang.clone())
            .unwrap_or_default();
        let mut r = horizon_protocol::buf::Reader::new(data);
        let decoded = (|| {
            let id = r.varint()?;
            let start = r.varint()?;
            let length = r.varint()?;
            let n = r.count(usize::MAX)?;
            let mut list = Vec::with_capacity(n.min(1000));
            for _ in 0..n {
                let text = r.string(32767)?;
                let tooltip = if r.bool()? {
                    Some(plain(&r.nbt()?, &lang))
                } else {
                    None
                };
                list.push((text, tooltip));
            }
            horizon_protocol::Result::Ok((id, start, length, list))
        })();
        if let Ok((id, start, length, list)) = decoded {
            let range = (start.max(0) as usize, length.max(0) as usize);
            self.chat_server_suggestions(id, range, list.clone());
            self.command_block_server_suggestions(id, range, list);
        }
    }
}
