use super::*;

impl Game<'_> {
    /// The last window focus state reported by the window system.
    pub(in crate::game) fn window_focused(&self) -> bool {
        self.focused
    }

    /// The same screen identifier provided to screen hooks.
    pub(in crate::game) fn observed_screen_id(&self) -> Option<ScreenId> {
        self.observed_screen_view().map(|view| ScreenId {
            id: view.id.to_owned(),
        })
    }

    pub(in crate::game) fn title_splash(&self) -> Option<String> {
        let title = self.title.as_ref()?;
        if !self.title_screen_render_uncovered() {
            return None;
        }
        title
            .visible_splash(self.settings.file.bool("hideSplashTexts"))
            .map(str::to_owned)
    }
}
