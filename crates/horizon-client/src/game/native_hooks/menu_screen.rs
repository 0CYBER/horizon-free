use super::*;

impl Game<'_> {
    /// The menu type's client layout when the open has no matching mod block.
    pub(in crate::game) fn native_menu_screen(&self, call: mod_api::__private::MenuScreenCall<'_>) {
        self.fire_menu_screen(call);
    }
}
