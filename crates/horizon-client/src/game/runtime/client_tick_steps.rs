//! The client ticks of one frame (`Minecraft.runTick`'s tick loop), after the frame has handled
//! what the server sent (`update_world`).
use super::*;

impl Game<'_> {
    /// Runs `steps` client ticks: each tells `client_tick_pre`, ticks the game unless it is
    /// paused or on the title screen (the level, its block entities' client tickers included),
    /// does the client's own per-tick work and tells `client_tick_post`. With `event_loop`, an
    /// armed tick script plays its step before the game ticks.
    pub(in crate::game) fn client_tick_steps(
        &mut self,
        steps: u32,
        event_loop: Option<&dyn EventLoopCtl>,
    ) -> Result<()> {
        for _ in 0..steps {
            let started = Instant::now();
            horizon_client::social::set_presence_status(self.presence_status());
            horizon_client::social::tick();
            // Minecraft.tick: every client tick, paused or not, in menus too.
            self.native_client_tick_pre();
            if self.title.is_none() && !self.pause {
                if let Some(event_loop) = event_loop
                    && self.tick_script.armed()
                {
                    self.tick_script_step(event_loop);
                }
                horizon_client::spans::time("tick", || self.tick())?;
                // Cave mood (a shader pack's playerMood) samples on a random
                // seeded from the clock: under `HORIZON_STILL` it waits for the
                // world and draws from its tick.
                if !self.clock.still() {
                    self.tick_ambience();
                } else if self.clock.runs_normally() {
                    self.ambience.reseed(self.visual_ticks() as i64);
                    self.tick_ambience();
                }
                self.tick_fov();
            }
            self.tick_win_screen()?;
            self.native_configs_tick_client();
            // Gui.tick runs ChatListener.tick paused or not.
            self.tick_chat_queue();
            self.tick_music();
            self.tick_sound_engine();
            // ClientTickEvent.Post, the tick's work done.
            self.native_client_tick_post();
            self.clock.record(started.elapsed());
            if let Some(report) = self.clock.take_sprint_report() {
                self.sprint_report(&report);
                self.on_tick_rate_changed();
            }
        }
        Ok(())
    }
}
