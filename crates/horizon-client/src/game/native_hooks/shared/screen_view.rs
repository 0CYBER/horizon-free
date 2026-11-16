use super::*;

/// `view` as the screen `id` (an overlay or a title screen, which shows over
/// the whole window with no menu).
pub(super) fn screen_view_with_id<'a>(view: &ScreenView<'a>, id: &'static str) -> ScreenView<'a> {
    ScreenView::from(mod_api::__private::built::ScreenView {
        id,
        menu: view.menu,
        title: view.title,
        rect: view.rect,
        slots: view.slots,
        mouse: view.mouse,
        partial_tick: view.partial_tick,
    })
}
