use super::*;

impl Game<'_> {
    pub(in crate::game) fn show_title_with_registry(
        &mut self,
        mut title: horizon_client::title::Title,
    ) {
        title.set_additional_data_packs(self.pack.additional_data.clone());
        title.set_preset_data_source(self.pack.data_changes.clone());
        if !self.screen_hooks_heard() {
            self.screen_state.build.reopened();
            self.title = Some(title);
            self.build_screen();
            return;
        }
        let id = title.mode.id();
        self.title_change(Some(id));
        self.screen_state.build.reopened();
        self.title = Some(title);
        self.build_screen();
        self.screen_state.observed = self.observed_now();
        let none = self.screen_change_view(Screen::Playing);
        self.native_screen_init_post(&screen_view_with_id(&none, id));
    }
}
