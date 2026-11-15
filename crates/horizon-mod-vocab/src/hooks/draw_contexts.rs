pub trait DrawContexts {
    #[draw_context(surface = "crate::hud::HudBatch", name = "DrawContext")]
    fn draw_context();
    #[draw_context(
        surface = "crate::game::native_block_entity_draw::BlockEntityDraw",
        name = "BlockEntityDrawContext"
    )]
    fn block_entity_draw_context();
}
