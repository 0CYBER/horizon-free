//! Receiving-provider credit follows ordered events and decoder placeholders.
use super::*;

#[inline(always)]
fn backlog_item_overdue(queued_at: Instant, now: Instant) -> bool {
    now.saturating_duration_since(queued_at) >= BACKLOG_LIMIT
}

#[inline(always)]
fn backlog_head_since<T>(events: &VecDeque<(T, Instant)>) -> Option<Instant> {
    events.front().map(|(_, queued_at)| *queued_at)
}

impl Remote {
    pub(super) fn receive_credited_backlog(&mut self, events: &mut VecDeque<(Received, Instant)>) {
        // Chunk packets start decoding now; the frame that reaches one in
        // order inserts its column.
        let now = Instant::now();
        for receipt in self.session.poll() {
            let (event, credit) = receipt.into_parts();
            let received = match event {
                Event::Packet(p) if matches!(*p, C::LevelChunkWithLight(_)) => {
                    match self.decoder.submit(self.chunk_format.clone(), p) {
                        None => Received::Chunk(self.chunk_format.clone(), credit),
                        Some(p) => Received::Event(Event::Packet(p), credit),
                    }
                }
                event => Received::Event(event, credit),
            };
            events.push_back((received, now));
        }
    }
}

impl Game<'_> {
    pub(super) fn poll_remote_events(&mut self) -> Result<()> {
        let Some(r) = self.remote.as_mut() else {
            return Ok(());
        };
        let mut events = std::mem::take(&mut r.backlog);
        r.receive_credited_backlog(&mut events);
        let mut chunks = Duration::ZERO;
        // A run of ForgetLevelChunk (thousands when the server's view moves far: a
        // teleport at 32 chunks) is forgotten at once, before whatever follows it.
        let mut forgotten = horizon_client::fast_hash::FxHashSet::default();
        let mut forgotten_credits = Vec::new();
        while let Some((item, queued_at)) = events.pop_front() {
            if let Received::Event(Event::Packet(packet), _) = &item
                && let C::ForgetLevelChunk(p) = &**packet
            {
                self.observe_client_packet(packet);
                forgotten.insert((p.pos[0], p.pos[1]));
                forgotten_credits.push(item.into_event_credit());
                continue;
            }
            horizon_client::spans::time("pr_forget", || {
                self.forget_remote_columns(&mut forgotten, true)
            });
            forgotten_credits.clear();
            let chunk = matches!(item, Received::Chunk(_, _));
            if chunk
                && chunks >= CHUNK_PACKETS_PER_FRAME
                && !backlog_item_overdue(queued_at, Instant::now())
            {
                events.push_front((item, queued_at));
                break;
            }
            let started = chunk.then(Instant::now);
            let (event, _credit) = match item {
                Received::Event(event, credit) => (event, credit),
                Received::Chunk(format, credit) => {
                    // A column still decoding waits for the next frame, with
                    // what came after it, overdue or not.
                    let Some(next) = self.remote.as_ref().map(|r| r.decoder.next()) else {
                        break;
                    };
                    let (packet, column) = match next {
                        Next::Ready(decoded) => decoded,
                        Next::Decoding => {
                            events.push_front((Received::Chunk(format, credit), queued_at));
                            break;
                        }
                        Next::Lost => {
                            log::warn!("Horizon: the chunk decoder stopped; a column is missing");
                            continue;
                        }
                    };
                    self.observe_client_packet(&packet);
                    if let C::LevelChunkWithLight(p) = *packet {
                        horizon_client::spans::time("pr_chunk", || {
                            self.remote_chunk(&p, Some((&format, column)))
                        });
                    }
                    chunks += started.map_or(Duration::ZERO, |t| t.elapsed());
                    continue;
                }
            };
            match event {
                Event::Status(s) => {
                    if let Some(r) = self.remote.as_mut() {
                        r.status = s;
                    }
                }
                Event::Joined {
                    registries,
                    name,
                    uuid,
                } => {
                    self.set_registries(registries);
                    self.extras_joined();
                    self.close_dialogs();
                    if let Some(r) = self.remote.as_mut() {
                        r.joined = true;
                        r.profile = uuid;
                        r.profile_name = name.clone();
                        r.status = horizon_client::lang::get_or(
                            "multiplayer.downloadingTerrain",
                            "Loading terrain...",
                        );
                    }
                    self.begin_book_negotiation();
                }
                Event::Features(features) => {
                    if let Some(remote) = &mut self.remote {
                        remote.features = features.into();
                    }
                }
                Event::Brand(brand) => {
                    if let Some(remote) = &mut self.remote {
                        remote.brand = brand;
                    }
                }
                Event::QuietCommands => {
                    if let Some(remote) = &mut self.remote {
                        remote.quiet_commands = true;
                    }
                }
                Event::CustomBooks => {
                    if let Some(remote) = &mut self.remote {
                        remote.custom_books = true;
                    }
                }
                Event::FarTerrain(offer) => {
                    if let Some(remote) = &mut self.remote {
                        remote.far_terrain = Some(offer);
                    }
                }
                Event::Reconfiguring => {
                    self.reset_book_view();
                    // handleConfigurationStart: the held-back chat shows, then
                    // clearClientLevel opens ServerReconfigScreen over no level.
                    self.flush_chat_queue();
                    self.send_chat_acknowledgement();
                    self.close_dialogs();
                    // The server configuring us now (a proxy's next one)
                    // offers its own far terrain, or none, and announces its
                    // own features again.
                    if let Some(remote) = &mut self.remote {
                        remote.viewed.clear();
                        remote.far_terrain = None;
                        remote.quiet_commands = false;
                        remote.custom_books = false;
                    }
                    self.clear_lod();
                    if let Some(remote) = &self.remote {
                        remote.session.reset_far_terrain();
                    }
                    self.remove_local_player_body();
                    self.set_screen(Screen::Playing);
                    self.capture(false);
                    // clearClientLevel drops the client level now, so its
                    // unload comes before the configuration screen, not
                    // after the next login's load.
                    self.forget_all_remote_columns();
                    if let Some(old) = self.remote.as_mut().and_then(|r| r.heard_level.take()) {
                        self.native_client_level_unload(&old);
                    }
                    self.native_lifecycle.begin_close();
                    self.native_lifecycle_world_closed();
                    let ticks = self.ticks;
                    if let Some(r) = self.remote.as_mut() {
                        r.reconfiguring = Some(ticks);
                        r.chat_tree_received = false;
                    }
                }
                Event::Reconfigured(registries) => {
                    // handleConfigurationStart → clearClientLevel, and the next
                    // ClientPacketListener starts with no players or scoreboard,
                    // no waypoints (its own ClientWaypointManager: a proxy's next
                    // server never untracks the last one's), and its new
                    // ClientSuggestionProvider with no custom chat completions.
                    // clearClientLevel clears the narrator.
                    self.clear_narrator();
                    self.hud_disconnected();
                    self.score.board = Default::default();
                    self.clear_waypoints();
                    if let Some(r) = self.remote.as_mut() {
                        r.attachments.clear();
                        r.players.clear();
                        r.tab_players.clear();
                        r.listed.clear();
                        r.signatures = Default::default();
                        r.custom_completions.clear();
                    }
                    self.ui.registries.custom_completions.clear();
                    self.set_registries(registries);
                }
                // handleTransfer: "Cannot transfer to server from
                // singleplayer" (no ServerData), so the world just closes.
                Event::Transfer { host, port } => {
                    if let Some(r) = self.remote.as_mut()
                        && r.session.address != "local"
                    {
                        r.transfer = Some(format!("{host}:{port}"));
                    }
                }
                Event::PackPush(p) => self.handle_pack_push(p),
                Event::PackPop(id) => self.handle_pack_pop(id),
                Event::ServerLinks(links) => {
                    if let Some(r) = self.remote.as_mut() {
                        r.server_links = crate::game::server_links::trusted(links);
                    }
                }
                Event::ShowDialog(dialog) => self.show_dialog(&dialog),
                Event::ClearDialog => self.clear_dialog(),
                Event::CodeOfConduct(text) => self.handle_code_of_conduct(text),
                Event::DecodeError(e) => {
                    if let Some(r) = self.remote.as_mut() {
                        r.decode_errors += 1;
                        if r.decode_errors <= 20 {
                            log::warn!("Horizon: skipped undecodable packet {e}");
                        }
                    }
                }
                Event::Disconnected {
                    reason,
                    key,
                    from_server,
                } => {
                    self.reset_book_view();
                    log::info!("Horizon: disconnected: {reason}");
                    // ClientboundTransferPacket left a target: connect there
                    // instead of showing the disconnect, the stored cookies
                    // carried over (ClientCommonPacketListenerImpl.
                    // handleTransfer → ConnectScreen.startConnecting).
                    let transfer = self.remote.as_mut().and_then(|r| {
                        r.transfer.take().map(|address| {
                            (address, r.session.name.clone(), r.session.cookies.clone())
                        })
                    });
                    if let Some((address, name, cookies)) = transfer {
                        self.leave_server(None)?;
                        self.connect_server_with_cookies(&address, &name, Some(cookies))?;
                        return Ok(());
                    }
                    let retry = self.reconnect_after(&reason, key.as_deref(), from_server);
                    self.leave_server(Some(Text::literal(reason)))?;
                    self.offer_reconnect(retry);
                    return Ok(());
                }
                Event::RawPayload(mut incoming) => {
                    if !self.receive_client_payload(&mut incoming)? {
                        events.push_front((
                            Received::Event(Event::RawPayload(incoming), _credit),
                            queued_at,
                        ));
                        break;
                    }
                }
                Event::Packet(packet) => match *packet {
                    C::CustomPayload(p) if p.channel == horizon_client::perf_net::CHANNEL => {
                        self.remote_perf_reply(&p.data)
                    }
                    packet => self.handle_packet(packet)?,
                },
            }
            chunks += started.map_or(Duration::ZERO, |t| t.elapsed());
            if self.remote.is_none() {
                break;
            }
        }
        if self.remote.is_some() {
            horizon_client::spans::time("pr_forget", || {
                self.forget_remote_columns(&mut forgotten, true)
            });
        }
        if let Some(r) = self.remote.as_mut() {
            r.backlog_since = backlog_head_since(&events);
            r.backlog = events;
        }
        self.send_suggestion_request();
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Continuous chunk arrivals keep `events` non-empty across frames; once an
    /// overdue head drains, fresher items behind it must still respect the frame
    /// budget instead of staying overdue until the queue fully empties (#11751).
    #[test]
    fn continuous_arrivals_do_not_keep_fresh_items_overdue_after_head_drains() {
        fn drain_past_budget(
            queue: &mut VecDeque<(&'static str, Instant)>,
            now: Instant,
        ) -> (Vec<&'static str>, Option<Instant>) {
            let mut drained = Vec::new();
            while let Some((item, queued_at)) = queue.pop_front() {
                if !backlog_item_overdue(queued_at, now) {
                    queue.push_front((item, queued_at));
                    break;
                }
                drained.push(item);
            }
            (drained, backlog_head_since(queue))
        }

        let t0 = Instant::now();
        let fresh_at = t0 + (BACKLOG_LIMIT - Duration::from_millis(1));
        let frame1_now = t0 + BACKLOG_LIMIT;
        let mut events: VecDeque<(&'static str, Instant)> =
            VecDeque::from([("old_head", t0), ("fresh_1", fresh_at)]);

        // At `t0 + BACKLOG_LIMIT`, the old head is overdue and drains even after
        // the frame's chunk budget is spent, while `fresh_1` (1 ms old) defers and
        // `backlog_since` advances to `fresh_at`.
        let (drained, since) = drain_past_budget(&mut events, frame1_now);
        assert_eq!(drained, ["old_head"]);
        assert_eq!(since, Some(fresh_at));

        // A continuous stream appends another fresh item while the queue is still
        // non-empty; 10 ms later `t0` would still be overdue, yet `fresh_1` is only
        // 11 ms old so nothing bypasses the frame budget.
        let arrival_at = frame1_now + Duration::from_millis(10);
        events.push_back(("fresh_2", arrival_at));
        let (drained, since) = drain_past_budget(&mut events, arrival_at);
        assert!(drained.is_empty());
        assert_eq!(since, Some(fresh_at));

        // Once `fresh_1` itself reaches `BACKLOG_LIMIT`, only it drains while
        // `fresh_2` remains deferred and `backlog_since` advances to `arrival_at`.
        let frame3_now = fresh_at + BACKLOG_LIMIT;
        let (drained, since) = drain_past_budget(&mut events, frame3_now);
        assert_eq!(drained, ["fresh_1"]);
        assert_eq!(since, Some(arrival_at));

        // Once the last item drains, `backlog_since` clears to `None`.
        let (drained, since) = drain_past_budget(&mut events, arrival_at + BACKLOG_LIMIT);
        assert_eq!(drained, ["fresh_2"]);
        assert!(since.is_none());
        assert!(events.is_empty());
    }
}
