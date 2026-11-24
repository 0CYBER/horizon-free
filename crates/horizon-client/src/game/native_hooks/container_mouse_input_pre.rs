use super::{Game, ScreenMousePost};
use mod_api::{ContainerMouseInput, HookId, MenuRef};

impl Game<'_> {
    /// Runs after a screen Pre passed. Never forward the input to a different
    /// screen if a listener closes the container.
    pub(in crate::game) fn container_mouse_input_pre(&mut self, post: &ScreenMousePost) -> bool {
        if !mod_api::__private::listened(HookId::ContainerMouseInputPre) {
            return false;
        }
        let Some(menu) = self.screen_menu() else {
            return false;
        };
        let input = match *post {
            ScreenMousePost::Button { button, pressed } => {
                ContainerMouseInput::Button { button, pressed }
            }
            ScreenMousePost::Scroll(scroll) => ContainerMouseInput::Scroll(scroll),
            ScreenMousePost::Drag { button, delta } => ContainerMouseInput::Drag { button, delta },
        };
        let modifiers = super::super::glfw_modifiers(self.modifiers);
        let mut canceled = false;
        self.native_container_mouse_input_pre(menu, input, modifiers, &mut canceled);
        canceled || self.screen_menu() != Some(menu)
    }

    pub(in crate::game) fn native_container_mouse_input_pre(
        &mut self,
        menu: MenuRef,
        input: ContainerMouseInput,
        modifiers: u32,
        canceled: &mut bool,
    ) {
        self.fire_container_mouse_input_pre(menu, input, modifiers, canceled);
    }
}
