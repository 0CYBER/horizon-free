use super::*;

/// `ClientClockManager.tick` for one clock: it moves on by `delta` game ticks at its rate, the
/// part of a tick it makes carried to the next. None, and the clock left as it was, when its total
/// would pass the range of a long: a server's times must not wrap or panic the client.
pub(in crate::game) fn advance_clock(clock: &mut play::ClockState, delta: i64) -> Option<()> {
    let partial = f64::from(clock.partial_tick) + delta as f64 * f64::from(clock.rate);
    let full = partial.floor();
    if !(i64::MIN as f64..i64::MAX as f64).contains(&full) {
        return None;
    }
    clock.total_ticks = clock.total_ticks.checked_add(full as i64)?;
    clock.partial_tick = (partial - full) as f32;
    Some(())
}

/// Per-level client work bracketed by the shared LevelTickEvent hooks.
/// No level means no event; failed ticks never emit the post event.
impl Game<'_> {
    pub(in crate::game) fn tick_remote_level(&mut self) -> Result<()> {
        self.tick_remote_level_with(Self::tick_remote_level_body)
    }

    pub(in crate::game) fn tick_remote_level_with(
        &mut self,
        body: impl FnOnce(&mut Self) -> Result<()>,
    ) -> Result<()> {
        let level = (mod_api::__private::listened(mod_api::HookId::LevelTickPre)
            || mod_api::__private::listened(mod_api::HookId::LevelTickPost))
        .then(|| self.native_client_level())
        .flatten();
        if let Some(level) = level {
            self.native_level_tick_pre(level);
        }
        let result = body(self);
        // Each entity's own move this tick (`EntityEvent.EnteringSection`).
        self.client_entity_sections_stepped();
        // `ClientLevel.tick`'s RETURN, which a failed tick does not reach.
        if result.is_ok()
            && let Some(level) = level
        {
            self.native_level_tick_post(level);
        }
        result
    }

    fn tick_remote_level_body(&mut self) -> Result<()> {
        self.fire_pending_block_entity_loads();
        self.ticks += 1;
        self.repro_tick();
        // Hud.tick ends with ChatComponent.tick.
        let chat_ticks = self.visual_ticks();
        self.ui.chat.tick(&self.gui.atlas, chat_ticks);
        self.tick_sign_editor();
        self.track_experience_display();
        self.tick_enchanting_book();
        // Leaving a slot clears its bundle's selection here and on the server.
        self.tick_bundle_hover();
        // Options.broadcastOptions: a changed render distance or Chat option
        // is sent again.
        let info = self
            .settings
            .client_information(client_view_distance(self.settings.render_distance));
        if let Some(r) = self.remote.as_mut()
            && r.joined
            && r.sent_info != info
        {
            r.sent_info = info.clone();
            self.send(S::ClientInformation(info));
        }
        // Minecraft.tick: TickRateManager.tick; ClientLevel.tickTime advances
        // the time only while the level runs normally.
        let running = self.clock.world_runs();
        self.boss_overlay.tick_darken(running);
        // Under `HORIZON_STILL` the frame's countdowns (this one, the hotbar's pop, the HUD's
        // below) wait for the world, as the border's size change does in any frozen level: a
        // frozen client keeps ticking, as many ticks as the run took.
        let hold = !running && self.clock.still();
        if !hold {
            self.tick_item_activation();
        }
        if running {
            self.world.game_time += 1;
            self.remote_clock_tick();
        }
        // Weather levels drift toward their targets between server updates.
        let w = &mut self.world.weather;
        w.rain_level.0 = w.rain_level.1;
        w.thunder_level.0 = w.thunder_level.1;
        if self.use_delay > 0 {
            self.use_delay -= 1;
        }
        // Minecraft.handleKeybinds precedes entity ticking and sendChanges.
        // Window callbacks only queue clicks, so an interaction cannot leak
        // between CLIENT_TICK_END and a later pong.
        self.remote_handle_input();
        self.tick_remote_player(running, hold)
    }
}
