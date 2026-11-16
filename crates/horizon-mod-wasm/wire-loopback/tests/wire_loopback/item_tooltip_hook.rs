pub(super) struct ItemTooltipHooks;
impl ItemTooltipHooks {
    pub(super) fn item_tooltip(
        &self,
        _: &dyn mod_api::ClientHost,
        event: &mut mod_api::ItemTooltipEvent<'_>,
    ) {
        let mask = u32::from(event.display.name_visible(false))
            | (u32::from(event.display.name_visible(true)) << 1)
            | (u32::from(event.display.shows("minecraft:banner_patterns")) << 2)
            | (u32::from(event.display.shows("test:hidden")) << 3);
        event
            .lines
            .push((mod_api::Text::literal("projection"), mask));
    }
}
