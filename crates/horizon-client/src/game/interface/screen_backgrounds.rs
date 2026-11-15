//! What a frame draws behind the screens: the title panorama, the level loading background, the
//! menu blur and transparent background, the world border wall, and the screens' render events.
use super::*;

impl Game<'_> {
    /// The frame shows the title's panorama, not a level: the title screen, or a
    /// LevelLoadingScreen (Reason.OTHER), which draws it too.
    pub(super) fn shows_panorama(&self) -> bool {
        // The two cases that always had one: the title screen and waiting on
        // a server. Left exactly as they were.
        self.title.is_some() || self.remote_loading() || self.screen_wants_panorama()
    }
    /// Whether the screen on top asked for the panorama through
    /// [`horizon_client::screen_draw::Backdrop`].
    ///
    /// This is the opt-in path for Horizon's own screens: a screen sets its
    /// `backdrop` and the renderer draws the panorama behind it, instead of
    /// every such screen needing its own branch in `shows_panorama`. Vanilla
    /// screens leave the backdrop at `None` and so never reach this.
    ///
    /// Only in a menu: in a world the panorama would replace the world the
    /// player is standing in, which is not what any of these screens want.
    pub(super) fn screen_wants_panorama(&self) -> bool {
        use horizon_client::screen_draw::Backdrop;
        if self.player.is_some() {
            return false;
        }
        self.ui
            .options
            .pack_screen
            .as_ref()
            .is_some_and(|p| p.backdrop == Backdrop::Panorama)
    }
    /// Draw the presentation retained by the existing load tracker, if one matched at entry.
    pub(super) fn draw_dimension_transition_screen(
        &self,
        batch: &mut HudBatch,
        tracker: &horizon_client::level_loading::LevelLoadTracker,
        progress: Option<f32>,
    ) -> bool {
        let Some(screen) = tracker
            .transition_screen()
            .and_then(horizon_client_render::dimension_transition::screen)
        else {
            return false;
        };
        use horizon_client::level_loading::Reason;
        let reason = match tracker.reason {
            Reason::NetherPortal => mod_api::DimensionTransitionReason::NetherPortal,
            Reason::EndPortal => mod_api::DimensionTransitionReason::EndPortal,
            Reason::Other => mod_api::DimensionTransitionReason::Other,
        };
        self.native_dimension_transition_screen_draw(screen, reason, progress, batch);
        true
    }
    /// `LevelLoadingScreen.extractBackground`.
    pub(super) fn draw_level_loading_background(
        &self,
        batch: &mut HudBatch,
        reason: horizon_client::level_loading::Reason,
    ) {
        use horizon_client::level_loading::Reason;
        let atlas = &self.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        match reason {
            // The panorama and its blur are drawn by the renderer.
            Reason::Other => {
                horizon_client::ui::menu_background(batch, atlas, 0.0, 0.0, w, h, false)
            }
            Reason::NetherPortal => {
                let overlay = hud_overlays::portal_overlay(w as i32, h as i32, 1.0);
                hud_layout::paint(batch, atlas, &[overlay]);
            }
            Reason::EndPortal => batch.fill(atlas, 0.0, 0.0, w, h, rgba(20, 20, 20, 255)),
        }
    }
    /// `GameRenderer.update`'s `requestedPostEffects`: `end_of_frame` (a resource
    /// pack's own chain), then the server's `LocalPlayer.postEffects`. Only the
    /// world is drawn through them.
    pub(super) fn requested_post_effects(&self) -> Vec<String> {
        if self.title.is_some() || self.player.is_none() {
            return Vec::new();
        }
        std::iter::once("minecraft:end_of_frame".to_owned())
            .chain(self.post_effects.iter().cloned())
            .chain(self.spectated_post_effect.map(str::to_owned))
            .collect()
    }
    /// `Screen.extractBlurredBackground` over the panorama or the world and
    /// HUD, when `menuBackgroundBlurriness` is at least 1.
    pub(super) fn menu_blur(&self) -> Option<f32> {
        let blurred = self.remote_loading()
            || self.title.as_ref().is_some_and(|t| {
                matches!(self.screen, Screen::Options | Screen::Win)
                    || !matches!(t.mode, horizon_client::title::Mode::Main)
            })
            || (self.title.is_none()
                && self.player.is_some()
                && (self.screen.blurs_background(self.pause_menu)
                    || self.dialog_open()
                    || (self.screen == Screen::Win && self.win_screen_pauses())))
            || self.screen_blur_requested.get();
        let radius = self.settings.menu_blur as f32;
        let mut radius = (blurred && radius >= 1.0).then_some(radius);
        self.native_menu_background(&mut radius);
        radius.filter(|r| r.is_finite() && (1.0..=64.0).contains(r))
    }
    /// `Screen.renderTransparentBackground`: draws the translucent dark background
    /// behind an in-game screen (inventory, containers, creative, books, signs, editors),
    /// giving native mods a chance to suppress it (`Flow::Cancel`) or apply menu blur / tiled background.
    pub(super) fn render_transparent_background(&self, screen: &str, batch: &mut HudBatch) {
        let mut canceled = false;
        self.native_screen_transparent_background(screen, batch, &mut canceled);
        if !canceled {
            let atlas = &self.gui.atlas;
            batch.fill(
                atlas,
                0.0,
                0.0,
                batch.width,
                batch.height,
                rgba(16, 16, 16, 160),
            );
        }
    }
    /// `LevelExtractor` / `LevelRenderer`: the world border wall this frame.
    pub(super) fn border_draw(
        &mut self,
        camera: [f64; 3],
        partial: f32,
    ) -> Option<horizon_client::gfx::BorderDraw> {
        use horizon_client::border_render::{self, BorderRenderer};
        let distance = f64::from(self.streaming.radius * 16);
        border_render::extract(
            &self.border,
            partial,
            camera,
            distance,
            &mut self.border_state,
        );
        // `LevelRenderer.repositionCamera`: the wall is rebuilt when the camera
        // enters another `SectionPos` (it only holds the stretch in range).
        let section = camera.map(|c| (c.floor() as i32) >> 4);
        if self.border_section != Some(section) {
            self.border_renderer.invalidate();
            self.border_section = Some(section);
        }
        // Camera.depthFar, as the renderer's far plane.
        let depth_far =
            horizon_client::camera::depth_far(self.terrain_distance(), self.settings.cloud_range);
        if !self
            .border_renderer
            .prepare(&self.border_state, camera, distance, depth_far)
        {
            return None;
        }
        let millis = self.visual_millis();
        Some(horizon_client::gfx::BorderDraw {
            vertices: self.border_renderer.vertices,
            rebuilt: self.border_renderer.take_rebuilt(),
            transforms: self
                .border_renderer
                .transforms(&self.border_state, camera, millis),
            sides: BorderRenderer::sides(&self.border_state, camera, distance),
        })
    }
}

impl Game<'_> {
    /// `ScreenEvent.Render.Post`, after the screen's drawing and tooltips,
    /// including when Pre cancelled. Gameplay overlays keep their exclusions.
    pub(super) fn draw_screen_with_post_hook(&self, batch: &mut HudBatch) {
        if let Some(view) = self.screen_render_view(batch) {
            self.screen_render_end(batch, &view);
        }
    }
    /// `ScreenEvent.Render.Pre` for the open screen: whether it draws.
    /// Background and Foreground are reported by the screen while drawing.
    pub(super) fn draw_screen_with_pre_hook(&self, batch: &mut HudBatch) -> bool {
        match self.screen_render_view(batch) {
            Some(view) => self.screen_render_begin(batch, &view),
            None => true,
        }
    }
    /// `draw_screen_with_post_hook` for the title's own screen, once a frame,
    /// after it has drawn and before the toasts.
    pub(super) fn draw_title_screen_with_post_hook(&self, batch: &mut HudBatch) {
        if let Some(view) = self.title_screen_render_view(batch) {
            self.screen_render_end(batch, &view);
        }
    }
    /// `draw_screen_with_pre_hook` for the title's own screens.
    pub(super) fn draw_title_screen_with_pre_hook(&self, batch: &mut HudBatch) -> bool {
        match self.title_screen_render_view(batch) {
            Some(view) => self.screen_render_begin(batch, &view),
            None => true,
        }
    }
    /// The open screen as the render hooks see it, once a frame: not while
    /// playing with no screen, nor while the server pack prompt or a book
    /// hides the open screen (`hud_layers` draws `Playing` then), and None
    /// with no mod listening to a render hook, so nothing is built then.
    pub(super) fn screen_render_view(&self, batch: &HudBatch) -> Option<mod_api::ScreenView<'_>> {
        let screen = self.screen;
        if screen == Screen::Playing
            || self.server_packs.prompt.is_some()
            || self.book.is_some()
            || !self.screen_render_listened()
        {
            return None;
        }
        // slot_count mirrors slots() without allocating; keep them in step.
        debug_assert_eq!(self.slot_count(), self.slots().len());
        Some(self.screen_view(batch, screen))
    }
    /// The title's own screen as the render hooks see it, once a frame: not
    /// when a screen of the game's own is over the title (Options draws and
    /// fires its own), nor while a dialog, a prompt or a connect or load
    /// hides it, and None with no mod listening to a render hook.
    pub(super) fn title_screen_render_view(
        &self,
        batch: &HudBatch,
    ) -> Option<mod_api::ScreenView<'_>> {
        if self.screen != Screen::Playing
            || self.screen_over_playing()
            || self.remote_loading()
            || !self.screen_render_listened()
        {
            return None;
        }
        self.title_screen_view(batch)
    }
}

impl Game<'_> {
    /// Dispatch at the boundary reported by the screen that owns the drawing.
    /// No view or layout is built unless this particular phase has a listener.
    pub(super) fn draw_screen_phase(
        &self,
        batch: &mut HudBatch,
        phase: horizon_client::screen_draw::RenderPhase,
    ) {
        use horizon_client::screen_draw::RenderPhase;
        let hook = match phase {
            RenderPhase::Background => mod_api::HookId::ScreenRenderBackground,
            RenderPhase::Foreground => mod_api::HookId::ScreenRenderForeground,
        };
        if !mod_api::__private::listened(hook) {
            return;
        }
        let view = self
            .screen_render_view(batch)
            .or_else(|| self.title_screen_render_view(batch));
        if let Some(view) = view {
            match phase {
                RenderPhase::Background => self.native_screen_render_background(batch, &view),
                RenderPhase::Foreground => self.native_screen_render_foreground(batch, &view),
            }
        }
    }
}
