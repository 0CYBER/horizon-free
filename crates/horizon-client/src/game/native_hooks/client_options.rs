use super::*;

/// Helpers for native mods, kept as stable as the hook points.
// Native mods use these; nothing in Horizon does.
#[allow(dead_code)]
impl Game<'_> {
    /// A snapshot of the current graphics preferences.
    pub(in crate::game) fn graphics_options(&self) -> ClientGraphicsOptions {
        let s = &self.settings;
        ClientGraphicsOptions {
            fov: s.fov,
            render_distance: s.render_distance,
            simulation_distance: s.simulation_distance,
            brightness: s.gamma,
            max_fps: s.max_fps,
            vsync: s.vsync,
            fullscreen: s.fullscreen,
            view_bobbing: s.view_bobbing,
            smooth_lighting: s.smooth_lighting,
            particles: s.particles,
            entity_distance: s.entity_distance,
            biome_blend: s.biome_blend,
        }
    }
    /// A snapshot of the current GUI and accessibility preferences.
    pub(in crate::game) fn gui_options(&self) -> ClientGuiOptions {
        gui_options_snapshot(&self.settings)
    }
    /// The GUI scale in effect (`Window.getGuiScale`): the scale the window
    /// applies from the GUI Scale option and its size, at least 1; none on a
    /// server or when no window exists. `gui_options().gui_scale` is only the
    /// option, where 0 means automatic.
    pub(in crate::game) fn gui_scale(&self) -> Option<u32> {
        // The size the renderer applies, as the drawing code reads it: it keeps its previous
        // configuration when the window reports a size it cannot draw at.
        let framebuffer = self
            .window
            .as_ref()
            .and(self.renderer.as_ref())
            .map(|r| [r.config.width, r.config.height]);
        resolved_gui_scale(
            framebuffer,
            self.settings.gui_scale,
            self.settings.force_unicode,
        )
    }
    /// A snapshot of the current chat preferences.
    pub(in crate::game) fn chat_options(&self) -> ClientChatOptions {
        let s = &self.settings;
        ClientChatOptions {
            visibility: s.chat_visibility,
            colors: s.chat_colors,
            links: s.chat_links,
            opacity: s.chat_opacity,
            scale: s.chat_scale,
            width: s.chat_width,
            height_focused: s.chat_height_focused,
            height_unfocused: s.chat_height_unfocused,
            line_spacing: s.chat_line_spacing,
            delay: s.chat_delay,
        }
    }
    /// A snapshot of the current audio preferences.
    pub(in crate::game) fn audio_options(&self) -> ClientAudioOptions {
        ClientAudioOptions {
            master_volume: self.settings.master_volume,
        }
    }
}

/// Convert live client settings into the API-owned GUI snapshot.
fn gui_options_snapshot(s: &horizon_client::settings::Settings) -> ClientGuiOptions {
    ClientGuiOptions {
        gui_scale: s.gui_scale,
        attack_indicator: s.attack_indicator,
        main_hand_left: s.main_hand_left,
        reduced_debug_info: s.reduced_debug_info,
        advanced_item_tooltips: s.file.bool("advancedItemTooltips"),
        high_contrast: s.high_contrast,
        show_subtitles: s.show_subtitles,
        notification_display_time: s.notification_display_time,
        text_background_opacity: s.text_background_opacity,
        menu_blur: s.menu_blur,
    }
}

/// `Window.getGuiScale` for a framebuffer (`None` without a window): the
/// window's own `GuiSize` resolution, the same one it draws with.
fn resolved_gui_scale(framebuffer: Option<[u32; 2]>, gui_scale: u32, unicode: bool) -> Option<u32> {
    framebuffer.map(|size| horizon_client::window::GuiSize::new(size, gui_scale, unicode).scale)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gui_snapshot_tracks_advanced_tooltip_toggle() {
        let mut settings = horizon_client::settings::Settings::default();
        for enabled in [false, true, false] {
            settings.file.set(
                "advancedItemTooltips",
                horizon_client::options_file::V::Bool(enabled),
            );
            assert_eq!(
                gui_options_snapshot(&settings).advanced_item_tooltips,
                enabled
            );
        }
    }

    /// The literal `Window.calculateScale` outcomes the issue asks for: the
    /// option is a maximum, and the 320x240 floor wins over it.
    #[test]
    fn gui_scale_resolves_the_option_and_the_window() {
        assert_eq!(resolved_gui_scale(Some([1280, 720]), 0, false), Some(3));
        assert_eq!(resolved_gui_scale(Some([1920, 1080]), 0, false), Some(4));
        assert_eq!(resolved_gui_scale(Some([1920, 1080]), 2, false), Some(2));
        assert_eq!(resolved_gui_scale(Some([1280, 720]), 5, false), Some(3));
        assert_eq!(resolved_gui_scale(Some([320, 240]), 0, false), Some(1));
        // The Unicode font raises an odd resolved scale by one.
        assert_eq!(resolved_gui_scale(Some([1920, 1080]), 3, true), Some(4));
    }

    /// A window the renderer did not resize (an undrawable size keeps its previous
    /// configuration) resolves from the renderer's 1280x720, not the window's own
    /// 1920x1080, which alone would give 4.
    #[test]
    fn gui_scale_follows_the_size_the_renderer_applies() {
        assert_eq!(resolved_gui_scale(Some([1280, 720]), 0, false), Some(3));
        assert_eq!(resolved_gui_scale(Some([1920, 1080]), 0, false), Some(4));
    }

    #[test]
    fn gui_scale_needs_a_window() {
        assert_eq!(resolved_gui_scale(None, 0, false), None);
        assert_eq!(resolved_gui_scale(None, 4, true), None);
    }
}
