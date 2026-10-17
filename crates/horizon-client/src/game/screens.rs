//! The pause menu (PauseScreen) and death screen (DeathScreen) layouts.
use super::{Game, Screen};
use horizon_client::{
    Result,
    hud::{HudBatch, rgba},
    text::{self, Text},
    ui,
};

/// PauseScreen buttons: (x, y, width, label key, action id, active).
/// https://mcsrc.dev/2/26.3/net/minecraft/client/gui/screens/PauseScreen
fn pause_buttons(w: f32, h: f32, remote: bool) -> Vec<(f32, f32, f32, &'static str, u8, bool)> {
    // GridLayout of two 98px columns (padding 4), aligned at (0.5, 0.25).
    let (grid_w, grid_h) = (212.0, 166.0);
    let x0 = ((w - grid_w) * 0.5).floor();
    let y0 = ((h - grid_h) * 0.25).floor();
    vec![
        (x0 + 4.0, y0 + 50.0, 204.0, "menu.returnToGame", 0, true),
        (x0 + 4.0, y0 + 74.0, 98.0, "gui.advancements", 4, true),
        (x0 + 110.0, y0 + 74.0, 98.0, "gui.stats", 3, true),
        (x0 + 4.0, y0 + 122.0, 98.0, "menu.options", 1, true),
        (
            x0 + 110.0,
            y0 + 122.0,
            98.0,
            "options.worldOptions.button",
            9,
            true,
        ),
        (
            x0 + 4.0,
            y0 + 146.0,
            204.0,
            if remote {
                "menu.disconnect"
            } else {
                "menu.returnToMenu"
            },
            2,
            true,
        ),
    ]
}

/// The icon row: report bugs, feedback, friends, player reporting.
fn pause_icons(w: f32, h: f32) -> [(f32, f32, &'static str); 4] {
    let x0 = ((w - 212.0) * 0.5).floor() + 60.0;
    let y = ((h - 166.0) * 0.25).floor() + 98.0;
    [
        (x0, y, "gui/sprites/pause_menu/bug"),
        (x0 + 24.0, y, "gui/sprites/pause_menu/social_interactions"),
        (x0 + 48.0, y, "gui/sprites/friends/friends"),
        (x0 + 72.0, y, "gui/sprites/pause_menu/player_reporting"),
    ]
}
fn death_buttons(w: f32, h: f32) -> [(f32, f32, &'static str, u8); 2] {
    [
        (w / 2.0 - 100.0, h / 4.0 + 72.0, "deathScreen.respawn", 0),
        (
            w / 2.0 - 100.0,
            h / 4.0 + 96.0,
            "deathScreen.titleScreen",
            1,
        ),
    ]
}
impl Game<'_> {
    /// Singleplayer also has a connection, backed by the integrated server.
    pub(super) fn external_server(&self) -> bool {
        self.remote.is_some() && self.integrated.is_none()
    }
    /// Whether the pause menu is drawn in Horizon's style (over a blurred world).
    pub(super) fn horizon_pause(&self) -> bool {
        self.screen == Screen::Paused
            && self.pause_menu
            && self.title.is_none()
            && horizon_client::ui_theme::horizon()
    }
    fn pause_labels(&self) -> (String, &'static str) {
        if let Some(server) = &self.integrated {
            return (server.world_name.clone(), "Save and quit");
        }
        if self.external_server() {
            ("Multiplayer".to_owned(), "Disconnect")
        } else {
            (self.level.name.clone(), "Save and quit")
        }
    }
    pub(super) fn draw_pause_menu(&mut self, batch: &mut HudBatch) {
        if horizon_client::ui_theme::horizon() {
            let opened = *self.pause_since.get_or_insert_with(std::time::Instant::now);
            let (world, quit) = self.pause_labels();
            self.tick_friends();
            let friends = self.friends_view();
            let mut ui = horizon_client::ui_theme::Ui::new(&self.gui.atlas, batch, self.cursor);
            horizon_client::front::pause::draw(
                &mut ui,
                &world,
                quit,
                opened,
                &friends,
                self.external_server(),
            );
            ui.finish();
            return;
        }
        let atlas = &self.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        ui::menu_background(batch, atlas, 0.0, 0.0, w, h, true);
        let title = Text::translate("menu.game", &[]);
        let tw = atlas.font.width_text(&title);
        batch.text_styled(
            atlas,
            &title,
            ((w - tw) / 2.0).floor(),
            40.0,
            u32::MAX,
            true,
        );
        for (x, y, bw, key, _, active) in pause_buttons(w, h, self.external_server()) {
            let hover = active && ui::hovered(self.cursor, x, y, bw, 20.0);
            ui::button(
                batch,
                atlas,
                x,
                y,
                bw,
                20.0,
                &Text::translate(key, &[]),
                active,
                hover,
            );
        }
        for (x, y, sprite) in pause_icons(w, h) {
            ui::button(batch, atlas, x, y, 20.0, 20.0, &Text::empty(), false, false);
            // SpriteIconButton.CenteredIcon: the 15x15 sprite at
            // x + width / 2 - 15 / 2 in integer pixels, untinted even
            // when the button is inactive.
            batch.sprite_scaled(atlas, sprite, x + 3.0, y + 3.0, 15.0, 15.0, u32::MAX);
        }
    }
    pub(super) fn pause_click(&mut self) -> Result<()> {
        let (w, h) = {
            let size = self
                .renderer
                .as_ref()
                .map_or([1, 1], |r| [r.config.width, r.config.height]);
            let b = HudBatch::new(size);
            (b.width, b.height)
        };
        let hit = if horizon_client::ui_theme::horizon() {
            let size = self
                .renderer
                .as_ref()
                .map_or([1, 1], |r| [r.config.width, r.config.height]);
            let mut b = HudBatch::new(size);
            let (world, quit) = self.pause_labels();
            horizon_client::front::pause::click(
                &self.gui.atlas,
                &mut b,
                self.cursor.0,
                self.cursor.1,
                &world,
                quit,
                &self.friends_view(),
                self.external_server(),
            )
        } else {
            pause_buttons(w, h, self.external_server())
                .into_iter()
                .find(|b| b.5 && ui::hovered(self.cursor, b.0, b.1, b.2, 20.0))
                .map(|b| b.4)
        };
        match hit {
            Some(0) => {
                self.screen = Screen::Playing;
                self.capture(true);
            }
            Some(1) => self.open_options(false),
            Some(3) => self.open_stats(),
            Some(4) => self.open_advancements(true),
            Some(9) => self.open_world_options(),
            Some(10) => {
                if let Err(e) = self.friends_start() {
                    self.system_message(Text::literal(format!("Couldn't share the world: {e}")));
                }
            }
            Some(11) => self.friends_copy(),
            Some(12) => self.friends_stop(),
            Some(2) => self.close_world()?,
            _ => {}
        }
        if hit.is_some() && !horizon_client::ui_theme::horizon() {
            self.audio.play_button_click();
        }
        Ok(())
    }
    /// DeathScreen: gradient, the title at double size, the cause and score.
    pub(super) fn draw_death_screen(&mut self, batch: &mut HudBatch) {
        let atlas = &self.gui.atlas;
        let (w, h) = (batch.width, batch.height);
        // fillGradient(0x60500000 -> 0xA0803030).
        let rows = h.ceil() as i32;
        for y in 0..rows {
            let t = y as f32 / h.max(1.0);
            let lerp = |a: f32, b: f32| (a + (b - a) * t) as u8;
            batch.fill(
                atlas,
                0.0,
                y as f32,
                w,
                1.0,
                rgba(
                    lerp(80.0, 128.0),
                    lerp(0.0, 48.0),
                    lerp(0.0, 48.0),
                    lerp(96.0, 160.0),
                ),
            );
        }
        let title = Text::translate("deathScreen.title", &[]);
        let tw = atlas.font.width_text(&title);
        ui::text_scaled(
            batch,
            atlas,
            &title,
            (w / 2.0 - tw).floor(),
            60.0,
            2.0,
            u32::MAX,
        );
        if let Some(cause) = &self.ui.death_text {
            let cw = atlas.font.width_text(cause);
            batch.text_styled(atlas, cause, ((w - cw) / 2.0).floor(), 85.0, u32::MAX, true);
        }
        let score = Text::translate(
            "deathScreen.score.value",
            &[Text::colored(
                self.experience.total.to_string(),
                text::YELLOW,
            )],
        );
        let sw = atlas.font.width_text(&score);
        batch.text_styled(
            atlas,
            &score,
            ((w - sw) / 2.0).floor(),
            100.0,
            u32::MAX,
            true,
        );
        let active = self
            .ui
            .died_at
            .is_none_or(|t| t.elapsed().as_millis() >= 1000);
        for (x, y, key, _) in death_buttons(w, h) {
            let hover = active && ui::hovered(self.cursor, x, y, 200.0, 20.0);
            ui::button(
                batch,
                atlas,
                x,
                y,
                200.0,
                20.0,
                &Text::translate(key, &[]),
                active,
                hover,
            );
        }
    }
    pub(super) fn death_click(&mut self) -> Result<()> {
        if self
            .ui
            .died_at
            .is_some_and(|t| t.elapsed().as_millis() < 1000)
        {
            return Ok(());
        }
        let size = self
            .renderer
            .as_ref()
            .map_or([1, 1], |r| [r.config.width, r.config.height]);
        let b = HudBatch::new(size);
        let hit = death_buttons(b.width, b.height)
            .into_iter()
            .find(|d| ui::hovered(self.cursor, d.0, d.1, 200.0, 20.0))
            .map(|d| d.3);
        if hit == Some(0) && self.remote.is_none() {
            let keep = self.ui.rules.bool("keep_inventory") || self.spectator();
            self.experience.respawn(keep);
            // Autosaves keep a login position; death must resolve the respawn point instead.
            self.level.position = None;
        }
        match hit {
            Some(0) if self.remote.is_some() => {
                self.ui.death_text = None;
                self.toggles.respawn();
                self.remote_respawn();
            }
            Some(0) if self.respawn_config.is_some() => {
                // LocalPlayer.respawn: KeyMapping.resetToggleKeys.
                self.toggles.respawn();
                // PlayerList.respawn at the respawn anchor, once it is loaded.
                self.begin_anchor_respawn()?;
                self.screen = Screen::Playing;
                self.ui.death_text = None;
                self.spawn_player()?;
                self.capture(true);
            }
            Some(0) => {
                // LocalPlayer.respawn: KeyMapping.resetToggleKeys.
                self.toggles.respawn();
                self.respawn_out_of_end()?;
                self.player = None;
                self.screen = Screen::Playing;
                self.ui.death_text = None;
                self.spawn_player()?;
                self.capture(true);
            }
            Some(1) => self.close_world()?,
            _ => {}
        }
        if hit.is_some() {
            self.audio.play_button_click();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::pause_buttons;

    #[test]
    fn pause_menu_uses_disconnect_for_servers_and_keeps_world_options() {
        for (remote, label) in [(false, "menu.returnToMenu"), (true, "menu.disconnect")] {
            let buttons = pause_buttons(427.0, 240.0, remote);
            let quit = buttons.iter().find(|b| b.4 == 2).unwrap();
            assert_eq!(quit.3, label);
            assert!(quit.5);
            assert!(buttons.iter().any(|b| b.4 == 9 && b.5));
        }
    }
}
