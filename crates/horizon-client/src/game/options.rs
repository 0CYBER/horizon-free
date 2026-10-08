//! The options screens bound to the game: drawing, input, applying settings
//! (video, sound, chat, font, language) and the F11 fullscreen toggle.
use super::{Game, Screen};
use horizon_client::{
    Result,
    hud::HudBatch,
    lang,
    options_ui::{Event, Page},
};
use winit::{event::MouseButton, keyboard::KeyCode, window::Fullscreen};

impl Game<'_> {
    fn options_path() -> std::path::PathBuf {
        std::env::current_dir()
            .unwrap_or_default()
            .join("options.txt")
    }
    fn gui_size(&self) -> (f32, f32) {
        let size = self
            .renderer
            .as_ref()
            .map_or([1280, 720], |r| [r.config.width, r.config.height]);
        let b = HudBatch::new(size);
        (b.width, b.height)
    }
    /// Open the options screens (OptionsScreen) from the title or pause menu.
    pub(super) fn open_options(&mut self, from_title: bool) {
        self.screen = Screen::Options;
        self.options_from_title = from_title;
        let ui = &mut self.ui.options;
        ui.open(!from_title && self.title.is_none());
        let root = self.options.resources.clone();
        ui.languages = lang::available(&[&root]);
        self.refresh_pack_lists();
    }
    pub(super) fn draw_options(&mut self, batch: &mut HudBatch) {
        let mut ui = std::mem::take(&mut self.ui.options);
        ui.draw(&self.settings, batch, &self.res.atlas, self.cursor);
        self.ui.options = ui;
    }
    pub(super) fn options_mouse(&mut self, pressed: bool, right: bool) -> Result<()> {
        if !pressed {
            self.ui.options.release();
            self.dragging = false;
            return Ok(());
        }
        self.dragging = true;
        let size = self.gui_size();
        let mut ui = std::mem::take(&mut self.ui.options);
        let events = ui.click(
            &mut self.settings,
            &self.res.atlas,
            self.cursor.0,
            self.cursor.1,
            size,
            false,
            right,
        );
        self.ui.options = ui;
        self.handle_option_events(events)
    }
    pub(super) fn options_drag(&mut self) -> Result<()> {
        let size = self.gui_size();
        let mut ui = std::mem::take(&mut self.ui.options);
        let events = ui.click(
            &mut self.settings,
            &self.res.atlas,
            self.cursor.0,
            self.cursor.1,
            size,
            true,
            false,
        );
        self.ui.options = ui;
        self.handle_option_events(events)
    }
    pub(super) fn options_key(&mut self, key: KeyCode, text: Option<&str>) -> Result<()> {
        let mut ui = std::mem::take(&mut self.ui.options);
        let events = ui.key(&mut self.settings, key, text);
        self.ui.options = ui;
        self.handle_option_events(events)
    }
    pub(super) fn options_scroll(&mut self, amount: f32) {
        self.ui.options.scroll(amount);
    }
    /// Mouse buttons pressed while a key is being bound.
    pub(super) fn options_waiting_for_key(&self) -> bool {
        self.ui.options.selected_key.is_some() && self.ui.options.page() == Page::KeyBinds
    }
    pub(super) fn options_bind_mouse(&mut self, button: MouseButton) -> Result<()> {
        if let Some(name) = self.ui.options.selected_key.take() {
            self.settings
                .keys
                .set(name, &horizon_client::keybinds::mouse_name(button));
            self.handle_option_events(vec![Event::Changed])?;
        }
        Ok(())
    }
    fn handle_option_events(&mut self, events: Vec<Event>) -> Result<()> {
        for e in events {
            match e {
                Event::Changed => {
                    self.settings.save(&Self::options_path());
                    self.apply_settings();
                }
                Event::Close => {
                    self.settings.save(&Self::options_path());
                    self.screen = if self.options_from_title {
                        Screen::Playing
                    } else {
                        Screen::Paused
                    };
                    self.dragging = false;
                }
                Event::Language(code) => {
                    self.load_language(&code);
                }
                Event::Packs(_) => {
                    self.settings.save(&Self::options_path());
                    self.reload_packs();
                }
                Event::OpenPackFolder => {
                    let dir = std::env::current_dir()
                        .unwrap_or_default()
                        .join("resourcepacks");
                    let _ = std::fs::create_dir_all(&dir);
                    let _ = std::process::Command::new("xdg-open").arg(&dir).spawn();
                }
            }
        }
        Ok(())
    }
    /// LanguageManager.setSelected + reload: translations, then re-wrap chat.
    pub(super) fn load_language(&mut self, code: &str) {
        let root = self.options.resources.clone();
        lang::set(lang::Language::load(&[&root], code));
        self.ui.chat.rescale(&self.res.atlas);
    }
    /// Settings that Horizon applies beyond render distance, volume and GUI scale.
    pub(super) fn apply_ui_settings(&mut self) {
        let s = &self.settings;
        if let Some(r) = self.renderer.as_mut() {
            r.gamma = s.gamma;
            r.set_vsync(s.vsync);
        }
        if let Some(w) = &self.window {
            let want = s.fullscreen;
            if want != w.fullscreen().is_some() {
                w.set_fullscreen(want.then_some(Fullscreen::Borderless(None)));
            }
        }
        let mut volumes = s.volumes;
        volumes[0] = 1.0;
        horizon_client::settings::set_source_volumes(volumes);
        let chat = s.chat_options();
        let rescale = {
            let o = &self.ui.chat.options;
            o.width != chat.width || o.scale != chat.scale || o.line_spacing != chat.line_spacing
        };
        self.ui.chat.options = chat;
        if rescale {
            self.ui.chat.rescale(&self.res.atlas);
        }
        let font = &mut self.res.atlas.font;
        if font.uniform != s.force_unicode || font.japanese != s.japanese_glyphs {
            font.uniform = s.force_unicode;
            font.japanese = s.japanese_glyphs;
            font.reset_cache();
            self.ui.chat.rescale(&self.res.atlas);
        }
    }
    /// F11 (key.fullscreen): Window.toggleFullScreen, remembered in options.txt.
    pub(super) fn toggle_fullscreen(&mut self) {
        self.settings.fullscreen = !self.settings.fullscreen;
        self.settings.save(&Self::options_path());
        self.apply_ui_settings();
    }
    /// Frame limiter for the Max Framerate option: when the next frame is due.
    pub(super) fn next_frame_due(&self) -> Option<std::time::Instant> {
        let fps = self.settings.max_fps;
        if fps >= 260 || self.options.frames.is_some() {
            return None;
        }
        // Menus without a world run at 60 fps like vanilla's title screen.
        let fps = if self.title.is_some() {
            fps.min(60)
        } else {
            fps
        };
        let due = self.previous + std::time::Duration::from_secs_f64(1.0 / f64::from(fps));
        (due > std::time::Instant::now()).then_some(due)
    }
}
